//! The document a text message presents.
//!
//! This is the application's message text pipeline —
//! `session_view/room_history/message_row/text/` — with the widgets taken
//! out. The same sanitizer keeps the same elements, the same grouping turns
//! them into blocks and runs, and the same detection turns URLs, Matrix
//! identifiers and `@room` into links and mentions. What comes out is a flat
//! list of [`Block`]s, each carrying its place in the quotes and lists it
//! sits in, so a UI can draw the message without a tree of its own to walk.
//! No UI type crosses this module.
//!
//! A formatted body that is not HTML, or that yields nothing once sanitized,
//! falls back to the plain body, exactly as the application does.

use std::{collections::BTreeSet, fmt::Write as _, sync::LazyLock};

use linkify::{LinkFinder, LinkKind};
use ruma::{
    MatrixUri, OwnedMxcUri, OwnedUserId, RoomAliasId, RoomId, UserId,
    events::room::message::{FormattedBody, MessageFormat},
    html::{
        Attribute, Html, ListBehavior, NodeData, NodeRef, PropertiesNames, SanitizerConfig,
        StrTendril,
        matrix::{AnchorUri, ImageData, MatrixElement, MatrixElementData, SpanData},
    },
};
use tracing::debug;
use url::Url;

use super::{AT_ROOM, MatrixIdUri, MatrixRoomIdUri, find_at_room};
use crate::utils::StrMutExt;

/// The attribute that marks an `img` element as a custom emoticon.
///
/// Its value, if it has one, must be ignored.
const CUSTOM_EMOTICON_ATTRIBUTE: &str = "data-mx-emoticon";

/// The prefix of an email URI.
const EMAIL_URI_PREFIX: &str = "mailto:";
/// The prefix of an HTTPS URI.
const HTTPS_URI_PREFIX: &str = "https://";
/// The scheme of a Matrix URI.
const MATRIX_URI_SCHEME: &str = "matrix:";

/// All supported inline elements from the Matrix spec.
const SUPPORTED_INLINE_ELEMENTS: &[&str] = &[
    "del", "a", "sup", "sub", "b", "i", "u", "strong", "em", "s", "code", "br", "span", "img",
];

/// All supported block elements from the Matrix spec.
const SUPPORTED_BLOCK_ELEMENTS: &[&str] = &[
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "p",
    "ul",
    "ol",
    "li",
    "hr",
    "div",
    "pre",
    "details",
    "summary",
];

/// HTML sanitizer config for HTML messages.
static HTML_MESSAGE_SANITIZER_CONFIG: LazyLock<SanitizerConfig> = LazyLock::new(|| {
    SanitizerConfig::compat()
        .allow_elements(
            SUPPORTED_INLINE_ELEMENTS
                .iter()
                .chain(SUPPORTED_BLOCK_ELEMENTS.iter())
                .copied(),
            ListBehavior::Override,
        )
        // The attribute that marks an image as a custom emoticon is not part of
        // the sanitizer's list, and it is the only way to tell one apart.
        .allow_attributes(
            [PropertiesNames {
                parent: "img",
                properties: &[CUSTOM_EMOTICON_ATTRIBUTE],
            }],
            ListBehavior::Add,
        )
        .remove_reply_fallback()
});

/// The appearance of a run of text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one flag per inline element; the UI reads them side by side"
)]
pub struct InlineStyle {
    /// Bold, from `b` and `strong`.
    pub bold: bool,
    /// Italic, from `i` and `em`.
    pub italic: bool,
    /// Underlined, from `u`.
    pub underline: bool,
    /// Struck through, from `s` and `del`.
    pub strikethrough: bool,
    /// Monospace, from `code`.
    pub code: bool,
    /// Superscript, from `sup`.
    pub superscript: bool,
    /// Subscript, from `sub`.
    pub subscript: bool,
    /// The foreground color of a `span`, as the message wrote it.
    pub color: Option<String>,
    /// The background color of a `span`, as the message wrote it.
    pub bg_color: Option<String>,
}

/// Who or what a mention points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mention {
    /// A user.
    User {
        /// The ID of the user.
        user_id: OwnedUserId,
    },
    /// A room, by ID or alias.
    Room {
        /// The URI of the room.
        uri: MatrixRoomIdUri,
    },
    /// Everyone in the room.
    AtRoom,
}

/// One run of a line of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Text with one appearance, possibly a link.
    Text {
        /// The text.
        text: String,
        /// Its appearance.
        style: InlineStyle,
        /// The URI it links to, if it is a link.
        link: Option<String>,
    },
    /// A mention, presented as a pill.
    Mention {
        /// What is mentioned.
        mention: Mention,
        /// The name to show on the pill.
        name: String,
    },
    /// A custom emoticon, presented as an image among the words.
    Emoticon {
        /// The image of the emoticon.
        uri: OwnedMxcUri,
        /// Its textual description.
        body: String,
    },
}

/// What a block is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    /// A line of runs.
    Paragraph,
    /// A heading.
    Heading {
        /// The level, 1 to 6.
        level: u8,
    },
    /// Preformatted text, as a code block.
    Code {
        /// The language, if the message named one.
        language: Option<String>,
        /// The text, whitespace untouched.
        text: String,
    },
    /// A horizontal rule.
    Rule,
    /// The summary of a details disclosure; the blocks that follow at one
    /// more level of indentation are its content.
    Summary,
}

/// One block of a message.
///
/// The tree of quotes and lists is flattened: each block knows how deep it
/// sits, and the first block of a list item carries the item's marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// What the block is.
    pub kind: BlockKind,
    /// How many quotes the block sits in.
    pub quote_depth: u8,
    /// How many lists and disclosures the block sits in.
    pub indent: u8,
    /// The list marker, on the first block of a list item.
    pub marker: Option<String>,
    /// The runs of the block; empty for a rule or a code block.
    pub inlines: Vec<Inline>,
}

impl Block {
    /// Whether the block holds nothing but custom emoticons.
    ///
    /// The application presents such a message like a sticker: the
    /// emoticons large, as widgets of their own.
    #[must_use]
    pub fn is_emoticons_only(&self) -> bool {
        !self.inlines.is_empty()
            && self.inlines.iter().all(|inline| match inline {
                Inline::Emoticon { .. } => true,
                Inline::Text { text, .. } => text.trim().is_empty(),
                Inline::Mention { .. } => false,
            })
    }
}

/// The names a mention shows, looked up where the message was sent.
pub trait MentionResolver {
    /// The name to show for the given user.
    fn user_name(&self, user_id: &UserId) -> String;

    /// The name to show for the given room.
    fn room_name(&self, uri: &MatrixRoomIdUri) -> String;
}

/// A resolver that names every mention by its identifier.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentifierNames;

impl MentionResolver for IdentifierNames {
    fn user_name(&self, user_id: &UserId) -> String {
        user_id.to_string()
    }

    fn room_name(&self, uri: &MatrixRoomIdUri) -> String {
        uri.id.to_string()
    }
}

/// Whether the given [`FormattedBody`] contains HTML.
fn formatted_body_is_html(formatted: &FormattedBody) -> bool {
    formatted.format == MessageFormat::Html && !formatted.body.contains("<!-- raw HTML omitted -->")
}

/// The document of a message, from its formatted body when that is usable
/// HTML and from its plain body otherwise.
///
/// If `detect_at_room` is `true`, `@room` in the text becomes a mention.
///
/// If `sender_name` is set, it opens the message in bold, the way an emote
/// is presented.
pub fn message_blocks(
    formatted: Option<&FormattedBody>,
    body: &str,
    resolver: &dyn MentionResolver,
    detect_at_room: bool,
    sender_name: Option<&str>,
) -> Vec<Block> {
    if let Some(formatted) = formatted.filter(|formatted| formatted_body_is_html(formatted)) {
        let blocks = html_blocks(&formatted.body, resolver, detect_at_room, sender_name);

        if !blocks.is_empty() {
            return blocks;
        }
    }

    plain_blocks(body, resolver, detect_at_room, sender_name)
}

/// The document of the given HTML.
///
/// Returns an empty list if the HTML contains nothing to present.
pub fn html_blocks(
    html: &str,
    resolver: &dyn MentionResolver,
    detect_at_room: bool,
    sender_name: Option<&str>,
) -> Vec<Block> {
    let mut html = html.to_owned();
    html.clean_string();

    let html = Html::parse(html.trim_matches('\n'));
    html.sanitize_with(&HTML_MESSAGE_SANITIZER_CONFIG);

    if !html.has_children() {
        return Vec::new();
    }

    let mut builder = DocumentBuilder {
        resolver,
        detect_at_room,
        sender_name,
        blocks: Vec::new(),
        pending_marker: None,
    };
    builder.append_nodes(html.children(), Placement::default());
    builder.finish()
}

/// The document of the given plain text.
///
/// Newlines are kept; URLs, Matrix identifiers and `@room` are detected.
pub fn plain_blocks(
    body: &str,
    resolver: &dyn MentionResolver,
    detect_at_room: bool,
    sender_name: Option<&str>,
) -> Vec<Block> {
    let mut body = body.to_owned();
    body.clean_string();

    let mut inline = InlineBuilder::new(resolver, detect_at_room, true);
    inline.append_emote_name(sender_name);
    inline.linkify(&body);
    let inlines = inline.finish();

    if inlines.is_empty() {
        return Vec::new();
    }

    vec![Block {
        kind: BlockKind::Paragraph,
        quote_depth: 0,
        indent: 0,
        marker: None,
        inlines,
    }]
}

/// Where a block sits in the document.
#[derive(Debug, Clone, Copy, Default)]
struct Placement {
    /// How many quotes deep.
    quote_depth: u8,
    /// How many lists and disclosures deep.
    indent: u8,
    /// Whether whitespace is preserved, inside `pre`.
    preformatted: bool,
}

/// A group of nodes, representing the nodes contained in a single block.
#[derive(Debug)]
enum NodeGroup {
    /// A group of inline nodes.
    Inline(Vec<NodeRef>),
    /// A block node.
    Block(NodeRef),
}

/// Group subsequent nodes that are inline.
fn group_inline_nodes(nodes: impl IntoIterator<Item = NodeRef>) -> Vec<NodeGroup> {
    let mut result = Vec::new();
    let mut inline_group: Option<Vec<NodeRef>> = None;

    for node in nodes {
        let is_block = node
            .as_element()
            .is_some_and(|element| SUPPORTED_BLOCK_ELEMENTS.contains(&element.name.local.as_ref()));

        if is_block {
            if let Some(inline) = inline_group.take() {
                result.push(NodeGroup::Inline(inline));
            }

            result.push(NodeGroup::Block(node));
        } else {
            inline_group.get_or_insert_with(Vec::default).push(node);
        }
    }

    if let Some(inline) = inline_group.take() {
        result.push(NodeGroup::Inline(inline));
    }

    result
}

/// Whether the given node is the given element.
fn is_element(node: &NodeRef, name: &str) -> bool {
    node.as_element()
        .is_some_and(|element| element.name.local.as_ref() == name)
}

/// Builds the blocks of a document from HTML nodes.
struct DocumentBuilder<'a> {
    /// The names of the mentions.
    resolver: &'a dyn MentionResolver,
    /// Whether to detect `@room`.
    detect_at_room: bool,
    /// The sender name still to place, for an emote.
    sender_name: Option<&'a str>,
    /// The blocks built so far.
    blocks: Vec<Block>,
    /// The marker of the list item whose first block is still to come.
    pending_marker: Option<String>,
}

impl DocumentBuilder<'_> {
    /// The finished blocks.
    fn finish(mut self) -> Vec<Block> {
        if let Some(name) = self.sender_name.take() {
            // Nothing took the name: the message is only the name.
            self.blocks.insert(
                0,
                Block {
                    kind: BlockKind::Paragraph,
                    quote_depth: 0,
                    indent: 0,
                    marker: None,
                    inlines: vec![emote_name_inline(name)],
                },
            );
        }

        self.blocks
    }

    /// Add a block at the given placement.
    fn push_block(&mut self, kind: BlockKind, placement: Placement, mut inlines: Vec<Inline>) {
        let takes_text = matches!(kind, BlockKind::Paragraph | BlockKind::Heading { .. });

        if let Some(name) = self.sender_name.take() {
            if takes_text {
                inlines.insert(0, emote_name_inline(name));
            } else {
                // The block cannot open with the name, so the name gets a
                // line of its own before it.
                self.blocks.push(Block {
                    kind: BlockKind::Paragraph,
                    quote_depth: placement.quote_depth,
                    indent: placement.indent,
                    marker: self.pending_marker.take(),
                    inlines: vec![emote_name_inline(name)],
                });
            }
        }

        self.blocks.push(Block {
            kind,
            quote_depth: placement.quote_depth,
            indent: placement.indent,
            marker: self.pending_marker.take(),
            inlines,
        });
    }

    /// Add the given nodes at the given placement.
    fn append_nodes(&mut self, nodes: impl IntoIterator<Item = NodeRef>, placement: Placement) {
        for group in group_inline_nodes(nodes) {
            match group {
                NodeGroup::Inline(inline_nodes) => {
                    self.append_inline_group(inline_nodes, placement);
                }
                NodeGroup::Block(block_node) => {
                    self.append_block(&block_node, placement);
                }
            }
        }
    }

    /// Add a paragraph for the given inline nodes, if they say anything.
    fn append_inline_group(&mut self, nodes: Vec<NodeRef>, placement: Placement) {
        let inlines = self.inlines_for(nodes, placement);

        if inlines.is_empty() {
            return;
        }

        self.push_block(BlockKind::Paragraph, placement, inlines);
    }

    /// The runs of the given inline nodes.
    fn inlines_for(
        &self,
        nodes: impl IntoIterator<Item = NodeRef>,
        placement: Placement,
    ) -> Vec<Inline> {
        let mut builder =
            InlineBuilder::new(self.resolver, self.detect_at_room, placement.preformatted);
        builder.append_nodes(nodes, &InlineStyle::default(), None, true);
        builder.finish()
    }

    /// Add the given block node.
    fn append_block(&mut self, node: &NodeRef, placement: Placement) {
        let Some(element) = node.as_element() else {
            return;
        };

        match element.to_matrix().element {
            MatrixElement::H(heading) => {
                // A heading only has inline children. An empty one is still
                // shown, as the application shows an empty title.
                let inlines = self.inlines_for(node.children(), placement);
                self.push_block(
                    BlockKind::Heading {
                        level: heading.level.value(),
                    },
                    placement,
                    inlines,
                );
            }
            MatrixElement::Blockquote => {
                let inner = Placement {
                    quote_depth: placement.quote_depth.saturating_add(1),
                    ..placement
                };
                self.append_nodes(node.children(), inner);
            }
            MatrixElement::P
            | MatrixElement::Div(_)
            | MatrixElement::Li
            | MatrixElement::Summary => {
                self.append_nodes(node.children(), placement);
            }
            MatrixElement::Ul => {
                self.append_list(node, placement, None);
            }
            MatrixElement::Ol(list) => {
                self.append_list(node, placement, Some(list.start.unwrap_or(1)));
            }
            MatrixElement::Hr => {
                self.push_block(BlockKind::Rule, placement, Vec::new());
            }
            MatrixElement::Pre => {
                self.append_preformatted(node, placement);
            }
            MatrixElement::Details => {
                self.append_details(node, placement);
            }
            element => {
                debug!("Unexpected HTML block element: {element:?}");
            }
        }
    }

    /// Add the items of the given list.
    ///
    /// `start` is the first number of an ordered list; an unordered list has
    /// none.
    fn append_list(&mut self, node: &NodeRef, placement: Placement, start: Option<i64>) {
        // Lists are supposed to only have list items as children.
        let items = node.children().filter(|node| is_element(node, "li"));

        let inner = Placement {
            indent: placement.indent.saturating_add(1),
            ..placement
        };

        for (position, item) in items.enumerate() {
            let marker = match start {
                Some(start) => format!(
                    "{}.",
                    start.saturating_add(i64::try_from(position).unwrap_or(i64::MAX))
                ),
                None => "•".to_owned(),
            };
            self.pending_marker = Some(marker);

            let before = self.blocks.len();
            self.append_nodes(item.children(), inner);

            if self.blocks.len() == before {
                // An empty list item is still shown.
                self.push_block(BlockKind::Paragraph, inner, Vec::new());
            }
        }

        self.pending_marker = None;
    }

    /// Add the given preformatted text.
    fn append_preformatted(&mut self, node: &NodeRef, placement: Placement) {
        let children = node.children().collect::<Vec<_>>();

        if children.is_empty() {
            return;
        }

        let unique_code_child = (children.len() == 1)
            .then_some(&children[0])
            .and_then(|child| {
                match child
                    .as_element()
                    .map(|element| element.to_matrix().element)
                {
                    Some(MatrixElement::Code(code)) => Some((child, code)),
                    _ => None,
                }
            });

        let (nodes, language) = match unique_code_child {
            Some((child, code)) => (
                child.children().collect::<Vec<_>>(),
                code.language.as_ref().map(ToString::to_string),
            ),
            // This is just preformatted text, presented in monospace with
            // its whitespace untouched.
            None => (children, None),
        };

        if nodes.is_empty() {
            return;
        }

        let mut text = String::new();
        append_nodes_text(&mut text, nodes);
        text.truncate_end_whitespaces();

        self.push_block(BlockKind::Code { language, text }, placement, Vec::new());
    }

    /// Add the given details disclosure.
    fn append_details(&mut self, node: &NodeRef, placement: Placement) {
        let (summary, other_children) = node
            .children()
            .partition::<Vec<_>, _>(|node| is_element(node, "summary"));

        let inner = Placement {
            indent: placement.indent.saturating_add(1),
            ..placement
        };

        let summary_inlines = summary
            .into_iter()
            .next()
            .map(|node| self.inlines_for(node.children(), placement))
            .filter(|inlines| !inlines.is_empty());

        let before = self.blocks.len();
        // The summary goes first, but whether there is content decides
        // what it is; build the content aside, then place both.
        self.append_nodes(other_children, inner);
        let content = self.blocks.split_off(before);

        if content.is_empty() {
            if let Some(inlines) = summary_inlines {
                self.push_block(BlockKind::Paragraph, placement, inlines);
            }
            return;
        }

        let inlines = summary_inlines.unwrap_or_else(|| {
            vec![Inline::Text {
                text: "Details".to_owned(),
                style: InlineStyle::default(),
                link: None,
            }]
        });
        self.push_block(BlockKind::Summary, placement, inlines);
        self.blocks.extend(content);
    }
}

/// The bold run that opens an emote with its sender's name.
fn emote_name_inline(name: &str) -> Inline {
    Inline::Text {
        text: format!("{name} "),
        style: InlineStyle {
            bold: true,
            ..Default::default()
        },
        link: None,
    }
}

/// Append the text contained in the nodes to the string.
///
/// Markup is dropped and newlines are kept, as inside `pre`.
fn append_nodes_text(text: &mut String, nodes: impl IntoIterator<Item = NodeRef>) {
    for node in nodes {
        match node.data() {
            NodeData::Text(t) => {
                text.push_str(t.borrow().as_ref());
            }
            NodeData::Element(data) => {
                if data.name.local.as_ref() == "br" {
                    text.push('\n');
                } else {
                    append_nodes_text(text, node.children());
                }
            }
            _ => {}
        }
    }
}

/// Context for an HTML node.
#[derive(Debug, Clone, Copy)]
struct NodeContext {
    /// Whether we should try to search for links in the text of the node.
    should_linkify: bool,
    /// Whether this is the first child node of an element.
    is_first_child: bool,
    /// Whether this is the last child node of an element.
    is_last_child: bool,
}

/// Builds the runs of one line of text.
struct InlineBuilder<'a> {
    /// The names of the mentions.
    resolver: &'a dyn MentionResolver,
    /// Whether to detect `@room`.
    detect_at_room: bool,
    /// Whether whitespace should be preserved.
    preserve_whitespace: bool,
    /// The runs so far.
    inlines: Vec<Inline>,
}

impl<'a> InlineBuilder<'a> {
    /// Constructs a new builder.
    ///
    /// If `preserve_whitespace` is `true`, all whitespace is kept, otherwise
    /// it is collapsed according to the HTML spec.
    fn new(
        resolver: &'a dyn MentionResolver,
        detect_at_room: bool,
        preserve_whitespace: bool,
    ) -> Self {
        Self {
            resolver,
            detect_at_room,
            preserve_whitespace,
            inlines: Vec::new(),
        }
    }

    /// The finished runs, with the trailing whitespace removed unless it is
    /// preserved.
    fn finish(mut self) -> Vec<Inline> {
        if !self.preserve_whitespace {
            self.truncate_end_whitespaces();
        }

        self.inlines
            .retain(|inline| !matches!(inline, Inline::Text { text, .. } if text.is_empty()));

        self.inlines
    }

    /// Open the line with the given emote sender's name, if there is one.
    fn append_emote_name(&mut self, name: Option<&str>) {
        if let Some(name) = name {
            self.inlines.push(emote_name_inline(name));
        }
    }

    /// Whether the text so far ends with a newline.
    fn ends_with_newline(&self) -> bool {
        self.inlines
            .iter()
            .rev()
            .find_map(|inline| match inline {
                Inline::Text { text, .. } if text.is_empty() => None,
                Inline::Text { text, .. } => Some(text.ends_with('\n')),
                _ => Some(false),
            })
            .unwrap_or(false)
    }

    /// Remove the whitespace at the end of the text so far.
    fn truncate_end_whitespaces(&mut self) {
        while let Some(Inline::Text { text, .. }) = self.inlines.last_mut() {
            text.truncate_end_whitespaces();

            if text.is_empty() {
                self.inlines.pop();
            } else {
                break;
            }
        }
    }

    /// Remove the given number of bytes from the end of the text so far.
    ///
    /// Only the last run is shortened: this takes back text that was just
    /// appended in one piece.
    fn truncate_end(&mut self, len: usize) {
        if let Some(Inline::Text { text, .. }) = self.inlines.last_mut() {
            let new_len = text.len().saturating_sub(len);
            text.truncate(new_len);
        }
    }

    /// Append the given text with the given appearance, merging it into
    /// the previous run when nothing tells them apart.
    fn push_text(&mut self, text: &str, style: &InlineStyle, link: Option<&str>) {
        if text.is_empty() {
            return;
        }

        if let Some(Inline::Text {
            text: last,
            style: last_style,
            link: last_link,
        }) = self.inlines.last_mut()
            && last_style == style
            && last_link.as_deref() == link
        {
            last.push_str(text);
            return;
        }

        self.inlines.push(Inline::Text {
            text: text.to_owned(),
            style: style.clone(),
            link: link.map(ToOwned::to_owned),
        });
    }

    /// Append the given inline nodes.
    fn append_nodes(
        &mut self,
        nodes: impl IntoIterator<Item = NodeRef>,
        style: &InlineStyle,
        link: Option<&str>,
        should_linkify: bool,
    ) {
        let mut is_first_child = true;
        let mut nodes_iter = nodes.into_iter().peekable();

        while let Some(node) = nodes_iter.next() {
            let context = NodeContext {
                should_linkify,
                is_first_child,
                is_last_child: nodes_iter.peek().is_none(),
            };

            self.append_node(&node, style, link, context);

            is_first_child = false;
        }
    }

    /// Append the given inline node.
    fn append_node(
        &mut self,
        node: &NodeRef,
        style: &InlineStyle,
        link: Option<&str>,
        context: NodeContext,
    ) {
        match node.data() {
            NodeData::Element(data) => {
                let data = data.to_matrix();
                self.append_element_node(node, data, style, link, context.should_linkify);
            }
            NodeData::Text(text) => {
                self.append_text_node(text.borrow().as_ref(), style, link, context);
            }
            data => {
                debug!("Unexpected HTML node: {data:?}");
            }
        }
    }

    /// Append the given inline element node.
    fn append_element_node(
        &mut self,
        node: &NodeRef,
        data: MatrixElementData,
        style: &InlineStyle,
        link: Option<&str>,
        should_linkify: bool,
    ) {
        let MatrixElementData { element, attrs } = data;

        let styled = |f: fn(&mut InlineStyle)| {
            let mut style = style.clone();
            f(&mut style);
            style
        };

        match element {
            MatrixElement::Del | MatrixElement::S => {
                let style = styled(|s| s.strikethrough = true);
                self.append_nodes(node.children(), &style, link, should_linkify);
            }
            MatrixElement::A(anchor) => {
                // First, check if it's a mention.
                if let Some(uri) = &anchor.href
                    && self.maybe_append_mention(uri)
                {
                    return;
                }

                // It's not a mention, render the link, if it has a URI.
                let href = anchor.href.as_ref().and_then(anchor_uri_string);

                // Don't try to linkify text if we render the element, it does not make
                // sense to nest links.
                let should_linkify = href.is_none() && should_linkify;
                let link = href.as_deref().or(link);

                self.append_nodes(node.children(), style, link, should_linkify);
            }
            MatrixElement::Sup => {
                let style = styled(|s| s.superscript = true);
                self.append_nodes(node.children(), &style, link, should_linkify);
            }
            MatrixElement::Sub => {
                let style = styled(|s| s.subscript = true);
                self.append_nodes(node.children(), &style, link, should_linkify);
            }
            MatrixElement::B | MatrixElement::Strong => {
                let style = styled(|s| s.bold = true);
                self.append_nodes(node.children(), &style, link, should_linkify);
            }
            MatrixElement::I | MatrixElement::Em => {
                let style = styled(|s| s.italic = true);
                self.append_nodes(node.children(), &style, link, should_linkify);
            }
            MatrixElement::U => {
                let style = styled(|s| s.underline = true);
                self.append_nodes(node.children(), &style, link, should_linkify);
            }
            MatrixElement::Code(_) => {
                // Don't try to linkify text, it does not make sense to detect links inside
                // code.
                let style = styled(|s| s.code = true);
                self.append_nodes(node.children(), &style, link, false);
            }
            MatrixElement::Br => {
                if !self.preserve_whitespace {
                    // Remove whitespaces before the newline.
                    self.truncate_end_whitespaces();
                }

                self.push_text("\n", style, link);
            }
            MatrixElement::Span(span) => {
                let style = span_style(&span, style);
                self.append_nodes(node.children(), &style, link, should_linkify);
            }
            MatrixElement::Img(image) => {
                self.append_image(&image, &attrs, style, link);
            }
            element => {
                debug!("Unexpected HTML inline element: {element:?}");
                self.append_nodes(node.children(), style, link, should_linkify);
            }
        }
    }

    /// Append the given image element.
    ///
    /// Only a custom emoticon is presented as an image. Any other image is
    /// replaced by its description, because a message is not supposed to
    /// contain one.
    // The attributes are given to us in a set by the HTML parser, and we only
    // read them.
    #[allow(clippy::mutable_key_type)]
    fn append_image(
        &mut self,
        image: &ImageData,
        attrs: &BTreeSet<Attribute>,
        style: &InlineStyle,
        link: Option<&str>,
    ) {
        let body = image
            .alt
            .as_ref()
            .or(image.title.as_ref())
            .map(StrTendril::to_string)
            .unwrap_or_default();

        // The value of the attribute, if it has one, must be ignored.
        let has_emoticon_attribute = attrs
            .iter()
            .any(|attr| attr.name.local.as_ref() == CUSTOM_EMOTICON_ATTRIBUTE);

        // The specification says an image is a custom emoticon if and only if
        // it carries the attribute, but we never see it: the SDK sanitizes the
        // HTML of every message with the rules of the specification, which only
        // keep `src`, `alt`, `title`, `width` and `height` on an image, before
        // we are given it, and there is no way to opt out.
        //
        // An inline image in a message is a custom emoticon in practice, that
        // being the reason image packs exist, so one is presented whenever it
        // comes from the homeserver. The attribute is still honoured, so this
        // becomes exact again if the SDK ever stops removing it.
        let is_emoticon = has_emoticon_attribute || image.src.is_some();

        // `src` is only set when it is a valid `mxc:` URI, which is the only
        // scheme that the specification allows, so a message cannot make us
        // fetch anything from outside the homeserver.
        if is_emoticon && let Some(uri) = &image.src {
            self.inlines.push(Inline::Emoticon {
                uri: uri.clone(),
                body,
            });
            return;
        }

        if is_emoticon {
            debug!("Could not present a custom emoticon, using its description instead");
        }

        self.push_text(&body, style, link);
    }

    /// Append the given text node content.
    fn append_text_node(
        &mut self,
        text: &str,
        style: &InlineStyle,
        link: Option<&str>,
        context: NodeContext,
    ) {
        // Collapse whitespaces and remove them at the beginning and end of an HTML
        // element, and after a newline.
        let text = if self.preserve_whitespace {
            text.to_owned()
        } else {
            collapse_whitespaces(
                text,
                context.is_first_child || self.ends_with_newline(),
                context.is_last_child,
            )
        };

        if context.should_linkify {
            Linkifier {
                builder: self,
                style,
            }
            .linkify(&text);
        } else {
            self.push_text(&text, style, link);
        }
    }

    /// Search and replace links in the given text.
    fn linkify(&mut self, text: &str) {
        Linkifier {
            builder: self,
            style: &InlineStyle::default(),
        }
        .linkify(text);
    }

    /// Append the given URI as a mention, if it is one.
    ///
    /// Returns `true` if it was added as a mention.
    fn maybe_append_mention(&mut self, uri: impl TryInto<MatrixIdUri>) -> bool {
        let Ok(uri) = uri.try_into() else {
            return false;
        };

        let (mention, name) = match uri {
            MatrixIdUri::Room(room_uri) => {
                let name = self.resolver.room_name(&room_uri);
                (Mention::Room { uri: room_uri }, name)
            }
            MatrixIdUri::User(user_id) => {
                let name = self.resolver.user_name(&user_id);
                (Mention::User { user_id }, name)
            }
            MatrixIdUri::Event(_) => return false,
        };

        self.inlines.push(Inline::Mention { mention, name });
        true
    }

    /// Append the given string and replace `@room` with a mention.
    fn append_and_replace_at_room(&mut self, s: &str, style: &InlineStyle) {
        if self.detect_at_room
            && let Some(pos) = find_at_room(s)
        {
            self.push_text(&s[..pos], style, None);
            self.inlines.push(Inline::Mention {
                mention: Mention::AtRoom,
                name: AT_ROOM.to_owned(),
            });
            self.push_text(&s[pos + AT_ROOM.len()..], style, None);
        } else {
            self.push_text(s, style, None);
        }
    }
}

/// The appearance inside the given span, over the given one.
fn span_style(span: &SpanData, style: &InlineStyle) -> InlineStyle {
    let mut style = style.clone();

    if let Some(bg_color) = &span.bg_color {
        style.bg_color = Some(bg_color.to_string());
    }
    if let Some(color) = &span.color {
        style.color = Some(color.to_string());
    }

    style
}

/// The URI of the given anchor as a string, if it is one we can open.
fn anchor_uri_string(uri: &AnchorUri) -> Option<String> {
    match uri {
        AnchorUri::Matrix(uri) => Some(uri.to_string()),
        AnchorUri::MatrixTo(uri) => Some(uri.to_string()),
        AnchorUri::Other(uri) => Some(uri.to_string()),
        uri => {
            debug!("Unsupported anchor URI format: {uri:?}");
            None
        }
    }
}

/// Collapse the whitespace of the given text according to the HTML spec.
fn collapse_whitespaces(text: &str, trim_start: bool, trim_end: bool) -> String {
    let mut str = text;

    if trim_start {
        str = str.trim_start();
    }
    if trim_end {
        str = str.trim_end();
    }

    let mut new_string = String::with_capacity(str.len());
    let mut prev_is_space = false;

    for char in str.chars() {
        if char.is_whitespace() {
            if prev_is_space {
                // We have already added a space as the last character, ignore this whitespace.
                continue;
            }

            prev_is_space = true;
            new_string.push(' ');
        } else {
            prev_is_space = false;
            new_string.push(char);
        }
    }

    new_string
}

/// A helper type to linkify text into runs.
struct Linkifier<'a, 'b> {
    /// The builder receiving the runs.
    builder: &'a mut InlineBuilder<'b>,
    /// The appearance of the text.
    style: &'a InlineStyle,
}

impl Linkifier<'_, '_> {
    /// Search and replace links in the given text.
    fn linkify(mut self, text: &str) {
        let mut finder = LinkFinder::new();
        // Allow URLS without a scheme.
        finder.url_must_have_scheme(false);

        let mut prev_span = None;

        for span in finder.spans(text) {
            let span_text = span.as_str();

            match span.kind() {
                Some(LinkKind::Url) => {
                    let is_valid_url = self.append_detected_url(span_text, prev_span);

                    if is_valid_url {
                        prev_span = None;
                    } else {
                        prev_span = Some(span_text);
                    }
                }
                Some(LinkKind::Email) => {
                    let uri = format!("{EMAIL_URI_PREFIX}{span_text}");
                    self.builder.push_text(span_text, self.style, Some(&uri));

                    // The span was a valid email so we will not need to check it for the next span.
                    prev_span = None;
                }
                _ => {
                    self.builder
                        .append_and_replace_at_room(span_text, self.style);
                    prev_span = Some(span_text);
                }
            }
        }
    }

    /// Append the given URI with the given link content.
    fn append_uri(&mut self, uri: &str, content: &str) {
        if self.builder.maybe_append_mention(uri) {
            return;
        }

        self.builder.push_text(content, self.style, Some(uri));
    }

    /// Append the given string detected as a URL.
    ///
    /// Appends false positives as normal strings, otherwise appends it as a
    /// URI.
    ///
    /// Returns `true` if it was detected as a valid URL.
    fn append_detected_url(&mut self, detected_url: &str, prev_span: Option<&str>) -> bool {
        if Url::parse(detected_url).is_ok() {
            // This is a full URL with a scheme, we can trust that it is valid.
            self.append_uri(detected_url, detected_url);
            return true;
        }

        // It does not have a scheme, try to split it to get only the domain.
        let domain = if let Some((domain, _)) = detected_url.split_once('/') {
            // This is a URL with a path component.
            domain
        } else if let Some((domain, _)) = detected_url.split_once('?') {
            // This is a URL with a query component.
            domain
        } else if let Some((domain, _)) = detected_url.split_once('#') {
            // This is a URL with a fragment.
            domain
        } else {
            // It should only contain the full domain.
            detected_url
        };

        // Check that the top-level domain is known.
        if !domain.rsplit_once('.').is_some_and(|(_, d)| tld::exist(d)) {
            // This is a false positive, treat it like a regular string.
            self.builder.push_text(detected_url, self.style, None);
            return false;
        }

        // The LinkFinder detects the homeserver part of `matrix:` URIs and Matrix
        // identifiers, e.g. it detects `example.org` in `matrix:r/somewhere:
        // example.org` or in `#somewhere:matrix.org`. We can use that to detect the
        // full URI or identifier with the previous span.

        // First, detect if the previous character is `:`, this is common to URIs and
        // identifiers.
        if let Some(prev_span) = prev_span.filter(|s| s.ends_with(':')) {
            // Most identifiers in Matrix do not have a list of allowed characters, so all
            // characters are allowed… which makes it difficult to find where they start.
            // We have to set arbitrary rules for the localpart to match most cases:
            // - No whitespaces
            // - No `:`, as it is the separator between localpart and server name, and after
            //   the scheme in URIs
            // - As soon as we encounter a known sigil, we assume we have the full ID. We
            //   ignore event IDs because we need a room to be able to generate a link.
            if let Some((pos, c)) = prev_span[..]
                .char_indices()
                .rev()
                // Skip the `:` we detected earlier.
                .skip(1)
                .find(|(_, c)| c.is_whitespace() || matches!(c, ':' | '!' | '#' | '@'))
            {
                let maybe_id_start = &prev_span[pos..];

                match c {
                    ':' if prev_span[..pos].ends_with(MATRIX_URI_SCHEME) => {
                        // This should be a matrix URI.
                        let maybe_full_uri =
                            format!("{MATRIX_URI_SCHEME}{maybe_id_start}{detected_url}");
                        if MatrixUri::parse(&maybe_full_uri).is_ok() {
                            // Remove the start of the URI from the string.
                            self.builder
                                .truncate_end(maybe_id_start.len() + MATRIX_URI_SCHEME.len());
                            self.append_uri(&maybe_full_uri, &maybe_full_uri);

                            return true;
                        }
                    }
                    '!' => {
                        // This should be a room ID.
                        if let Ok(room_id) =
                            RoomId::parse(format!("{maybe_id_start}{detected_url}"))
                        {
                            // Remove the start of the ID from the string.
                            self.builder.truncate_end(maybe_id_start.len());
                            // Transform it into a link.
                            self.append_uri(&room_id.matrix_to_uri().to_string(), room_id.as_str());
                            return true;
                        }
                    }
                    '#' => {
                        // This should be a room alias.
                        if let Ok(room_alias) =
                            RoomAliasId::parse(format!("{maybe_id_start}{detected_url}"))
                        {
                            // Remove the start of the ID from the string.
                            self.builder.truncate_end(maybe_id_start.len());
                            // Transform it into a link.
                            self.append_uri(
                                &room_alias.matrix_to_uri().to_string(),
                                room_alias.as_str(),
                            );
                            return true;
                        }
                    }
                    '@' => {
                        // This should be a user ID.
                        if let Ok(user_id) =
                            UserId::parse(format!("{maybe_id_start}{detected_url}"))
                        {
                            // Remove the start of the ID from the string.
                            self.builder.truncate_end(maybe_id_start.len());
                            // Transform it into a link.
                            self.append_uri(&user_id.matrix_to_uri().to_string(), user_id.as_str());
                            return true;
                        }
                    }
                    _ => {
                        // We reached a whitespace without a sigil or URI
                        // scheme, this must be a regular URL.
                    }
                }
            }
        }

        let mut uri = String::with_capacity(HTTPS_URI_PREFIX.len() + detected_url.len());
        let _ = write!(uri, "{HTTPS_URI_PREFIX}{detected_url}");
        self.append_uri(&uri, detected_url);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text {
            text: s.to_owned(),
            style: InlineStyle::default(),
            link: None,
        }
    }

    fn styled(s: &str, f: fn(&mut InlineStyle)) -> Inline {
        let mut style = InlineStyle::default();
        f(&mut style);
        Inline::Text {
            text: s.to_owned(),
            style,
            link: None,
        }
    }

    fn link(s: &str, href: &str) -> Inline {
        Inline::Text {
            text: s.to_owned(),
            style: InlineStyle::default(),
            link: Some(href.to_owned()),
        }
    }

    fn paragraph(inlines: Vec<Inline>) -> Block {
        Block {
            kind: BlockKind::Paragraph,
            quote_depth: 0,
            indent: 0,
            marker: None,
            inlines,
        }
    }

    fn html(s: &str) -> Vec<Block> {
        html_blocks(s, &IdentifierNames, false, None)
    }

    fn inlines(s: &str) -> Vec<Inline> {
        let blocks = html(s);
        assert_eq!(blocks.len(), 1, "one block from {s:?}: {blocks:?}");
        blocks.into_iter().next().unwrap().inlines
    }

    #[test]
    fn text_with_no_markup() {
        assert_eq!(inlines("A simple text"), vec![text("A simple text")]);
    }

    #[test]
    fn trim_end_spaces() {
        assert_eq!(
            inlines("A high-altitude text 🗻   "),
            vec![text("A high-altitude text 🗻")]
        );
    }

    #[test]
    fn collapse_whitespace() {
        let original = "Hello \nyou! \nYou are <b>my \nfriend</b>.";
        assert_eq!(
            inlines(original),
            vec![
                text("Hello you! You are "),
                styled("my friend", |s| s.bold = true),
                text("."),
            ]
        );

        let original = " Hello    \nyou! \n\nYou are \n<b>   my \nfriend   </b>.  ";
        assert_eq!(
            inlines(original),
            vec![
                text("Hello you! You are "),
                styled("my friend", |s| s.bold = true),
                text("."),
            ]
        );
    }

    #[test]
    fn line_breaks() {
        assert_eq!(
            inlines("A simple text <br>on several lines"),
            vec![text("A simple text\non several lines")]
        );
    }

    #[test]
    fn sanitize_inline_html() {
        assert_eq!(
            inlines(
                r#"A <strong>text</strong> with <a href="https://docs.local/markup"><i>markup</i></a>"#
            ),
            vec![
                text("A "),
                styled("text", |s| s.bold = true),
                text(" with "),
                Inline::Text {
                    text: "markup".to_owned(),
                    style: InlineStyle {
                        italic: true,
                        ..Default::default()
                    },
                    link: Some("https://docs.local/markup".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn linkify() {
        assert_eq!(
            inlines(
                "The homepage is https://gnome.org, and you can contact me at contact@me.local"
            ),
            vec![
                text("The homepage is "),
                link("https://gnome.org", "https://gnome.org"),
                text(", and you can contact me at "),
                link("contact@me.local", "mailto:contact@me.local"),
            ]
        );
    }

    #[test]
    fn linkify_without_scheme() {
        assert_eq!(
            inlines("See gnome.org and not foo.notatld"),
            vec![
                text("See "),
                link("gnome.org", "https://gnome.org"),
                text(" and not foo.notatld"),
            ]
        );
    }

    #[test]
    fn do_not_linkify_inside_anchor() {
        assert_eq!(
            inlines(r#"The homepage is <a href="https://gnome.org">https://gnome.org</a>"#),
            vec![
                text("The homepage is "),
                link("https://gnome.org", "https://gnome.org"),
            ]
        );
    }

    #[test]
    fn do_not_linkify_inside_code() {
        assert_eq!(
            inlines("The homepage is <code>https://gnome.org</code>"),
            vec![
                text("The homepage is "),
                styled("https://gnome.org", |s| s.code = true),
            ]
        );
    }

    #[test]
    fn mentions_in_anchors_and_text() {
        let blocks = html(
            r#"Hello <a href="https://matrix.to/#/@alice:example.org">Alice</a>, see !room:example.org"#,
        );
        assert_eq!(
            blocks[0].inlines,
            vec![
                text("Hello "),
                Inline::Mention {
                    mention: Mention::User {
                        user_id: UserId::parse("@alice:example.org").unwrap(),
                    },
                    name: "@alice:example.org".to_owned(),
                },
                text(", see "),
                Inline::Mention {
                    mention: Mention::Room {
                        uri: MatrixRoomIdUri::parse("https://matrix.to/#/!room:example.org")
                            .unwrap(),
                    },
                    name: "!room:example.org".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn at_room_is_a_mention_when_allowed() {
        let blocks = html_blocks("Hey @room, hi", &IdentifierNames, true, None);
        assert_eq!(
            blocks[0].inlines,
            vec![
                text("Hey "),
                Inline::Mention {
                    mention: Mention::AtRoom,
                    name: "@room".to_owned(),
                },
                text(", hi"),
            ]
        );

        assert_eq!(inlines("Hey @room, hi"), vec![text("Hey @room, hi")]);
    }

    #[test]
    fn blocks_and_lists() {
        let blocks = html(
            "<h2>Title</h2><p>Para</p><blockquote><p>Quoted</p></blockquote><ol start=\"3\"><li>Three</li><li>Four</li></ol><ul><li>Dot</li></ul><hr><pre><code class=\"language-rust\">let x = 1;\n</code></pre>",
        );

        assert_eq!(
            blocks,
            vec![
                Block {
                    kind: BlockKind::Heading { level: 2 },
                    ..paragraph(vec![text("Title")])
                },
                paragraph(vec![text("Para")]),
                Block {
                    quote_depth: 1,
                    ..paragraph(vec![text("Quoted")])
                },
                Block {
                    indent: 1,
                    marker: Some("3.".to_owned()),
                    ..paragraph(vec![text("Three")])
                },
                Block {
                    indent: 1,
                    marker: Some("4.".to_owned()),
                    ..paragraph(vec![text("Four")])
                },
                Block {
                    indent: 1,
                    marker: Some("•".to_owned()),
                    ..paragraph(vec![text("Dot")])
                },
                Block {
                    kind: BlockKind::Rule,
                    ..paragraph(Vec::new())
                },
                Block {
                    kind: BlockKind::Code {
                        language: Some("rust".to_owned()),
                        text: "let x = 1;".to_owned(),
                    },
                    ..paragraph(Vec::new())
                },
            ]
        );
    }

    #[test]
    fn details() {
        let blocks = html("<details><summary>More</summary><p>Hidden</p></details>");
        assert_eq!(
            blocks,
            vec![
                Block {
                    kind: BlockKind::Summary,
                    ..paragraph(vec![text("More")])
                },
                Block {
                    indent: 1,
                    ..paragraph(vec![text("Hidden")])
                },
            ]
        );
    }

    #[test]
    fn custom_emoticon() {
        let blocks =
            html(r#"<img data-mx-emoticon src="mxc://example.org/abc" alt=":cat:" height="32">"#);
        assert_eq!(
            blocks[0].inlines,
            vec![Inline::Emoticon {
                uri: "mxc://example.org/abc".into(),
                body: ":cat:".to_owned(),
            }]
        );
        assert!(blocks[0].is_emoticons_only());
    }

    #[test]
    fn emote_opens_with_the_name() {
        let blocks = html_blocks("<p>waves</p>", &IdentifierNames, false, Some("Alice"));
        assert_eq!(
            blocks,
            vec![paragraph(vec![
                styled("Alice ", |s| s.bold = true),
                text("waves"),
            ])]
        );
    }

    #[test]
    fn plain_text_keeps_newlines_and_links() {
        let blocks = plain_blocks("Line one\nSee gnome.org", &IdentifierNames, false, None);
        assert_eq!(
            blocks,
            vec![paragraph(vec![
                text("Line one\nSee "),
                link("gnome.org", "https://gnome.org"),
            ])]
        );
    }

    #[test]
    fn empty_html_falls_back_to_the_body() {
        let formatted = FormattedBody::html("<mx-reply>gone</mx-reply>");
        let blocks = message_blocks(Some(&formatted), "plain", &IdentifierNames, false, None);
        assert_eq!(blocks, vec![paragraph(vec![text("plain")])]);
    }
}

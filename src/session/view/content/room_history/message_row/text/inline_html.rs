//! Helpers for making Pango-compatible strings from inline HTML.

use std::fmt::Write;

use ruma::html::{
    matrix::{AnchorUri, MatrixElement, SpanData},
    Children, NodeData, NodeRef,
};
use tracing::debug;

use crate::{
    components::Pill,
    prelude::*,
    session::model::Room,
    utils::string::{Linkifier, PangoStrMutExt},
};

/// Helper type to construct a Pango-compatible string from inline HTML nodes.
#[derive(Debug)]
pub(super) struct InlineHtmlBuilder<'a> {
    /// Whether this string should be on a single line.
    single_line: bool,
    /// Whether to append an ellipsis at the end of the string.
    ellipsis: bool,
    /// The mentions detection setting and results.
    mentions: MentionsMode<'a>,
    /// The inner string.
    inner: String,
    /// Whether this string was truncated because at the first newline.
    truncated: bool,
}

impl<'a> InlineHtmlBuilder<'a> {
    /// Constructs a new inline HTML string builder for the given room.
    ///
    /// If `single_line` is set to `true`, the string will be ellipsized at the
    /// first line break.
    ///
    /// If `ellipsis` is set to `true`, and ellipsis will be added at the end of
    /// the string.
    pub(super) fn new(single_line: bool, ellipsis: bool) -> Self {
        Self {
            single_line,
            ellipsis,
            mentions: MentionsMode::default(),
            inner: String::new(),
            truncated: false,
        }
    }

    /// Enable mentions detection in the given room.
    ///
    /// If `detect_at_room` is `true`, it will also try to detect `@room`
    /// mentions.
    pub(super) fn detect_mentions(mut self, room: &'a Room, detect_at_room: bool) -> Self {
        self.mentions = MentionsMode::WithMentions {
            room,
            pills: Vec::new(),
            detect_at_room,
        };
        self
    }

    /// Append and consume the given sender name for an emote, if it is set.
    pub(super) fn append_emote_with_name(mut self, name: &mut Option<&str>) -> Self {
        self.inner.maybe_append_emote_name(name);
        self
    }

    /// Export the Pango-compatible string and the [`Pill`]s that were
    /// constructed, if any.
    pub(super) fn build(self) -> (String, Option<Vec<Pill>>) {
        let mut inner = self.inner;
        let ellipsis = self.ellipsis | self.truncated;

        if ellipsis {
            inner.append_ellipsis();
        } else {
            inner.truncate_end_whitespaces();
        }

        let pills = if let MentionsMode::WithMentions { pills, .. } = self.mentions {
            (!pills.is_empty()).then_some(pills)
        } else {
            None
        };

        (inner, pills)
    }

    /// Construct the string with the given inline nodes by converting them to
    /// Pango markup.
    ///
    /// Returns the Pango-compatible string and the [`Pill`]s that were
    /// constructed, if any.
    pub(super) fn build_with_nodes(
        mut self,
        nodes: impl IntoIterator<Item = NodeRef>,
    ) -> (String, Option<Vec<Pill>>) {
        self.append_nodes(nodes, true);
        self.build()
    }

    /// Construct the string by traversing the nodes an returning only the text
    /// it contains.
    ///
    /// Node that markup contained in the text is not escaped and newlines are
    /// not removed.
    pub(super) fn build_with_nodes_text(
        mut self,
        nodes: impl IntoIterator<Item = NodeRef>,
    ) -> String {
        self.append_nodes_text(nodes);

        let (inner, _) = self.build();
        inner
    }

    /// Append the given inline node by converting it to Pango markup.
    fn append_node(&mut self, node: &NodeRef, should_linkify: bool) {
        match node.data() {
            NodeData::Element(data) => {
                let data = data.to_matrix();
                match data.element {
                    MatrixElement::Del | MatrixElement::S => {
                        self.append_tags_and_children("s", node.children(), should_linkify);
                    }
                    MatrixElement::A(anchor) => {
                        // First, check if it's a mention, if we detect mentions.
                        if let Some(uri) = &anchor.href {
                            if let MentionsMode::WithMentions { pills, room, .. } =
                                &mut self.mentions
                            {
                                if let Some(pill) = self.inner.maybe_append_mention(uri, room) {
                                    pills.push(pill);

                                    return;
                                }
                            }
                        }

                        // It's not a mention, render the link, if it has a URI.
                        let mut has_opening_tag = false;

                        if let Some(uri) = &anchor.href {
                            has_opening_tag = self.append_link_opening_tag_from_anchor_uri(uri);
                        }

                        // Always render the children.
                        for node in node.children() {
                            // Don't try to linkify text if we render the element, it does not make
                            // sense to nest links.
                            self.append_node(&node, !has_opening_tag && should_linkify);
                        }

                        if has_opening_tag {
                            self.inner.push_str("</a>");
                        }
                    }
                    MatrixElement::Sup => {
                        self.append_tags_and_children("sup", node.children(), should_linkify);
                    }
                    MatrixElement::Sub => {
                        self.append_tags_and_children("sub", node.children(), should_linkify);
                    }
                    MatrixElement::B | MatrixElement::Strong => {
                        self.append_tags_and_children("b", node.children(), should_linkify);
                    }
                    MatrixElement::I | MatrixElement::Em => {
                        self.append_tags_and_children("i", node.children(), should_linkify);
                    }
                    MatrixElement::U => {
                        self.append_tags_and_children("u", node.children(), should_linkify);
                    }
                    MatrixElement::Code(_) => {
                        // Don't try to linkify text, it does not make sense to detect links inside
                        // code.
                        self.append_tags_and_children("tt", node.children(), false);
                    }
                    MatrixElement::Br => {
                        if self.single_line {
                            self.truncated = true;
                        } else {
                            self.inner.push('\n');
                        }
                    }
                    MatrixElement::Span(span) => {
                        self.append_span(&span, node.children(), should_linkify);
                    }
                    element => {
                        debug!("Unexpected HTML inline element: {element:?}");
                        self.append_nodes(node.children(), should_linkify);
                    }
                }
            }
            NodeData::Text(text) => {
                let text = text.borrow().collapse_whitespaces();

                if should_linkify {
                    if let MentionsMode::WithMentions {
                        pills,
                        room,
                        detect_at_room,
                    } = &mut self.mentions
                    {
                        Linkifier::new(&mut self.inner)
                            .detect_mentions(room, pills, *detect_at_room)
                            .linkify(&text);
                    } else {
                        Linkifier::new(&mut self.inner).linkify(&text);
                    }
                } else {
                    self.inner.push_str(&text.escape_markup());
                }
            }
            data => {
                debug!("Unexpected HTML node: {data:?}");
            }
        }
    }

    /// Append the given inline nodes, converted to Pango markup.
    fn append_nodes(&mut self, nodes: impl IntoIterator<Item = NodeRef>, should_linkify: bool) {
        for node in nodes {
            self.append_node(&node, should_linkify);

            if self.truncated {
                // Stop as soon as the string is truncated.
                break;
            }
        }
    }

    /// Append the given inline children, converted to Pango markup, surrounded
    /// by tags with the given name.
    fn append_tags_and_children(
        &mut self,
        tag_name: &str,
        children: Children,
        should_linkify: bool,
    ) {
        let _ = write!(self.inner, "<{tag_name}>");

        self.append_nodes(children, should_linkify);

        let _ = write!(self.inner, "</{tag_name}>");
    }

    /// Append the opening Pango markup link tag of the given anchor URI.
    ///
    /// The URI is also used as a title, so users can preview the link on hover.
    ///
    /// Returns `true` if the opening tag was successfully constructed.
    fn append_link_opening_tag_from_anchor_uri(&mut self, uri: &AnchorUri) -> bool {
        match uri {
            AnchorUri::Matrix(uri) => {
                self.inner.append_link_opening_tag(uri.to_string());
                true
            }
            AnchorUri::MatrixTo(uri) => {
                self.inner.append_link_opening_tag(uri.to_string());
                true
            }
            AnchorUri::Other(uri) => {
                self.inner.append_link_opening_tag(uri);
                true
            }
            uri => {
                debug!("Unsupported anchor URI format: {uri:?}");
                false
            }
        }
    }

    /// Append the span with the given data and inline children as Pango Markup.
    ///
    /// Whether we are an inside an anchor or not decides if we try to linkify
    /// the text contained in the children nodes.
    fn append_span(&mut self, span: &SpanData, children: Children, should_linkify: bool) {
        self.inner.push_str("<span");

        if let Some(bg_color) = &span.bg_color {
            let _ = write!(self.inner, r#" bgcolor="{bg_color}""#);
        }
        if let Some(color) = &span.color {
            let _ = write!(self.inner, r#" color="{color}""#);
        }

        self.inner.push('>');

        self.append_nodes(children, should_linkify);

        self.inner.push_str("</span>");
    }

    /// Append the text contained in the nodes to the string.
    ///
    /// Returns `true` if the text was ellipsized.
    fn append_nodes_text(&mut self, nodes: impl IntoIterator<Item = NodeRef>) {
        for node in nodes {
            match node.data() {
                NodeData::Text(t) => {
                    let borrowed_t = t.borrow();
                    let t = borrowed_t.as_ref();

                    if self.single_line {
                        if let Some(newline) = t.find('\n') {
                            self.truncated = true;

                            self.inner.push_str(&t[..newline]);
                            self.inner.append_ellipsis();

                            break;
                        }
                    }

                    self.inner.push_str(t);
                }
                NodeData::Element(data) => {
                    if data.name.local.as_ref() == "br" {
                        if self.single_line {
                            self.truncated = true;
                            break;
                        }

                        self.inner.push('\n');
                    } else {
                        self.append_nodes_text(node.children());
                    }
                }
                _ => {}
            }

            if self.truncated {
                // Stop as soon as the string is truncated.
                break;
            }
        }
    }
}

/// The mentions mode of the [`InlineHtmlBuilder`].
#[derive(Debug, Default)]
enum MentionsMode<'a> {
    /// The builder will not detect mentions.
    #[default]
    NoMentions,
    /// The builder will detect mentions.
    WithMentions {
        /// The pills for the detected mentions.
        pills: Vec<Pill>,
        /// The room containing the mentions.
        room: &'a Room,
        /// Whether to detect `@room` mentions.
        detect_at_room: bool,
    },
}

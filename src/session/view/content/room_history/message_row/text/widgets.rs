//! Build HTML messages.

use gtk::{pango, prelude::*};
use ruma::html::{
    matrix::{MatrixElement, OrderedListData},
    Children, NodeRef,
};
use sourceview::prelude::*;

use super::{inline_html::InlineHtmlBuilder, SUPPORTED_BLOCK_ELEMENTS};
use crate::{components::LabelWithWidgets, prelude::*, session::model::Room};

/// Construct a new label for displaying a message's content.
pub(super) fn new_message_label() -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .xalign(0.0)
        .valign(gtk::Align::Start)
        .use_markup(true)
        .build()
}

/// Create a widget for the given HTML nodes in the given room.
///
/// If `ellipsize` is true, we will only render the first block.
///
/// If the sender name is set, it will be added as soon as possible.
///
/// Returns `None` if the widget would have been empty.
pub(super) fn widget_for_html_nodes<'a>(
    nodes: impl IntoIterator<Item = NodeRef<'a>>,
    room: &Room,
    ellipsize: bool,
    add_ellipsis: bool,
    sender_name: &mut Option<&str>,
) -> Option<gtk::Widget> {
    let nodes = nodes.into_iter().collect::<Vec<_>>();

    if nodes.is_empty() {
        return None;
    }

    let groups = group_inline_nodes(nodes);
    let len = groups.len();

    let mut children = Vec::new();
    for (i, group) in groups.into_iter().enumerate() {
        let is_last = i == (len - 1);
        let add_ellipsis = add_ellipsis || (ellipsize && !is_last);

        match group {
            NodeGroup::Inline(inline_nodes) => {
                if let Some(widget) =
                    label_for_inline_html(inline_nodes, room, ellipsize, add_ellipsis, sender_name)
                {
                    children.push(widget);
                }
            }
            NodeGroup::Block(block_node) => {
                let Some(widget) =
                    widget_for_html_block(block_node, room, ellipsize, add_ellipsis, sender_name)
                else {
                    continue;
                };

                // Include sender name before, if the child widget did not handle it.
                if let Some(sender_name) = sender_name.take() {
                    let label = new_message_label();
                    let (text, _) = InlineHtmlBuilder::new(false, false)
                        .append_emote_with_name(&mut Some(sender_name))
                        .build();
                    label.set_label(&text);

                    children.push(label.upcast());
                }

                children.push(widget);
            }
        }

        if ellipsize {
            // Stop at the first constructed child.
            break;
        }
    }

    if children.is_empty() {
        return None;
    }
    if children.len() == 1 {
        return children.into_iter().next();
    }

    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .accessible_role(gtk::AccessibleRole::Group)
        .build();

    for (row, child) in children.into_iter().enumerate() {
        grid.attach(&child, 0, row as i32, 1, 1);
    }

    Some(grid.upcast())
}

/// A group of nodes, representing the nodes contained in a single widget.
enum NodeGroup<'a> {
    /// A group of inline nodes.
    Inline(Vec<NodeRef<'a>>),
    /// A block node.
    Block(NodeRef<'a>),
}

/// Group subsequent nodes that are inline.
///
/// Allows to group nodes by widget that will need to be constructed.
fn group_inline_nodes(nodes: Vec<NodeRef<'_>>) -> Vec<NodeGroup<'_>> {
    let mut result = Vec::new();
    let mut inline_group = None;

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
            let inline = inline_group.get_or_insert_with(Vec::default);
            inline.push(node);
        }
    }

    if let Some(inline) = inline_group.take() {
        result.push(NodeGroup::Inline(inline));
    }

    result
}

/// Construct a `GtkLabel` for the given inline nodes.
///
/// Returns `None` if the label would have been empty.
fn label_for_inline_html<'a>(
    nodes: impl IntoIterator<Item = NodeRef<'a>>,
    room: &'a Room,
    ellipsize: bool,
    add_ellipsis: bool,
    sender_name: &mut Option<&str>,
) -> Option<gtk::Widget> {
    let (text, widgets) = InlineHtmlBuilder::new(ellipsize, add_ellipsis)
        .detect_mentions(room)
        .append_emote_with_name(sender_name)
        .build_with_nodes(nodes);

    if text.is_empty() {
        return None;
    }

    if let Some(widgets) = widgets {
        widgets.iter().for_each(|p| {
            // Show the profile on click.
            p.set_activatable(true);
        });
        let w = LabelWithWidgets::with_label_and_widgets(&text, widgets);
        w.set_use_markup(true);
        w.set_ellipsize(ellipsize);
        Some(w.upcast())
    } else {
        let w = new_message_label();
        w.set_markup(&text);
        w.set_ellipsize(if ellipsize {
            pango::EllipsizeMode::End
        } else {
            pango::EllipsizeMode::None
        });
        Some(w.upcast())
    }
}

/// Create a widget for the given HTML block node.
fn widget_for_html_block(
    node: NodeRef<'_>,
    room: &Room,
    ellipsize: bool,
    add_ellipsis: bool,
    sender_name: &mut Option<&str>,
) -> Option<gtk::Widget> {
    let widget = match node.as_element()?.to_matrix().element {
        MatrixElement::H(heading) => {
            // Heading should only have inline elements as children.
            let w =
                label_for_inline_html(node.children(), room, ellipsize, add_ellipsis, sender_name)
                    .unwrap_or_else(|| {
                        // We should show an empty title.
                        new_message_label().upcast()
                    });
            w.add_css_class(&format!("h{}", heading.level.value()));
            w
        }
        MatrixElement::Blockquote => {
            let w =
                widget_for_html_nodes(node.children(), room, ellipsize, add_ellipsis, &mut None)?;
            w.add_css_class("quote");
            w
        }
        MatrixElement::P | MatrixElement::Div | MatrixElement::Li => {
            widget_for_html_nodes(node.children(), room, ellipsize, add_ellipsis, sender_name)?
        }
        MatrixElement::Ul => widget_for_list(
            ListType::Unordered,
            node.children(),
            room,
            ellipsize,
            add_ellipsis,
        )?,
        MatrixElement::Ol(list) => {
            widget_for_list(list.into(), node.children(), room, ellipsize, add_ellipsis)?
        }
        MatrixElement::Hr => gtk::Separator::new(gtk::Orientation::Horizontal).upcast(),
        MatrixElement::Pre => {
            widget_for_preformatted_text(node.children(), ellipsize, add_ellipsis)?
        }
        _ => return None,
    };

    Some(widget)
}

/// Create a widget for a list.
fn widget_for_list(
    list_type: ListType,
    list_items: Children<'_>,
    room: &Room,
    ellipsize: bool,
    add_ellipsis: bool,
) -> Option<gtk::Widget> {
    let list_items = list_items
        // Lists are supposed to only have list items as children.
        .filter(|node| {
            node.as_element()
                .is_some_and(|element| element.name.local.as_ref() == "li")
        })
        .collect::<Vec<_>>();

    if list_items.is_empty() {
        return None;
    }

    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(6)
        .margin_end(6)
        .margin_start(6)
        .build();

    let len = list_items.len();

    for (pos, li) in list_items.into_iter().enumerate() {
        let is_last = pos == (len - 1);
        let add_ellipsis = add_ellipsis || (ellipsize && !is_last);

        let w = widget_for_html_nodes(li.children(), room, ellipsize, add_ellipsis, &mut None)
            // We should show an empty list item.
            .unwrap_or_else(|| new_message_label().upcast());

        let bullet = list_type.bullet(pos);

        grid.attach(&bullet, 0, pos as i32, 1, 1);
        grid.attach(&w, 1, pos as i32, 1, 1);

        if ellipsize {
            break;
        }
    }

    Some(grid.upcast())
}

/// The type of bullet for a list.
#[derive(Debug, Clone, Copy)]
enum ListType {
    /// An unordered list.
    Unordered,
    /// An ordered list.
    Ordered {
        /// The number to start counting from.
        start: i64,
    },
}

impl ListType {
    /// Construct the widget for the bullet of the current type at the given
    /// position.
    fn bullet(&self, position: usize) -> gtk::Label {
        let bullet = gtk::Label::builder().valign(gtk::Align::Baseline).build();

        match self {
            ListType::Unordered => bullet.set_label("•"),
            ListType::Ordered { start } => {
                bullet.set_label(&format!("{}.", *start + position as i64))
            }
        }

        bullet
    }
}

impl From<OrderedListData> for ListType {
    fn from(value: OrderedListData) -> Self {
        Self::Ordered {
            start: value.start.unwrap_or(1),
        }
    }
}

/// Create a widget for preformatted text.
fn widget_for_preformatted_text(
    children: Children<'_>,
    ellipsize: bool,
    add_ellipsis: bool,
) -> Option<gtk::Widget> {
    let children = children.collect::<Vec<_>>();

    if children.is_empty() {
        return None;
    }

    let unique_code_child = (children.len() == 1)
        .then_some(&children[0])
        .and_then(|child| child.as_element())
        .and_then(|element| match element.to_matrix().element {
            MatrixElement::Code(code) => Some(code),
            _ => None,
        });

    let (children, code_language) = if let Some(code) = unique_code_child {
        let children = children[0].children().collect::<Vec<_>>();

        if children.is_empty() {
            return None;
        }

        (children, code.language)
    } else {
        (children, None)
    };

    let text = InlineHtmlBuilder::new(ellipsize, add_ellipsis).build_with_nodes_text(children);

    if ellipsize {
        // Present text as inline code.
        let text = format!("<tt>{}</tt>", text.escape_markup());

        let label = new_message_label();
        label.set_ellipsize(if ellipsize {
            pango::EllipsizeMode::End
        } else {
            pango::EllipsizeMode::None
        });
        label.set_label(&text);

        return Some(label.upcast());
    }

    let buffer = sourceview::Buffer::builder()
        .highlight_matching_brackets(false)
        .text(text)
        .build();
    crate::utils::sourceview::setup_style_scheme(&buffer);

    let language = code_language
        .and_then(|lang| sourceview::LanguageManager::default().language(lang.as_ref()));
    buffer.set_language(language.as_ref());

    let view = sourceview::View::builder()
        .buffer(&buffer)
        .editable(false)
        .css_classes(["codeview", "frame"])
        .hexpand(true)
        .build();

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    scrolled.set_child(Some(&view));
    Some(scrolled.upcast())
}

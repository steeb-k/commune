use std::fmt::Write;

use adw::{prelude::BinExt, subclass::prelude::*};
use gtk::{glib, glib::clone, pango, prelude::*};
use html2pango::{
    block::{markup_html, HtmlBlock},
    html_escape, markup_links,
};
use matrix_sdk::ruma::events::room::message::{FormattedBody, MessageFormat};

use super::ContentFormat;
use crate::{
    components::LabelWithWidgets,
    prelude::*,
    session::model::{Member, Room},
    utils::{matrix::extract_mentions, BoundObjectWeakRef, EMOJI_REGEX},
};

enum WithMentions<'a> {
    Yes(&'a Room),
    No,
}

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::MessageText)]
    pub struct MessageText {
        /// The original text of the message that is displayed.
        #[property(get)]
        pub original_text: RefCell<String>,
        /// Whether the original text is HTML.
        ///
        /// Only used for emotes.
        #[property(get)]
        pub is_html: Cell<bool>,
        /// The text format.
        #[property(get, builder(ContentFormat::default()))]
        pub format: Cell<ContentFormat>,
        /// The sender of the message, if we need to listen to changes.
        pub sender: BoundObjectWeakRef<Member>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageText {
        const NAME: &'static str = "ContentMessageText";
        type Type = super::MessageText;
        type ParentType = adw::Bin;
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageText {}

    impl WidgetImpl for MessageText {}
    impl BinImpl for MessageText {}
}

glib::wrapper! {
    /// A widget displaying the content of a text message.
    // FIXME: We have to be able to allow text selection and override popover
    // menu. See https://gitlab.gnome.org/GNOME/gtk/-/issues/4606
    pub struct MessageText(ObjectSubclass<imp::MessageText>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageText {
    /// Creates a text widget.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Display the given plain text.
    pub fn with_text(&self, body: String, format: ContentFormat) {
        if !self.original_text_changed(&body) && !self.format_changed(format) {
            return;
        }

        self.reset();
        self.set_original_text(body.clone());
        self.set_format(format);

        self.build_text(body, WithMentions::No, false);
    }

    /// Display the given text with markup.
    ///
    /// It will detect if it should display the body or the formatted body.
    pub fn with_markup(
        &self,
        formatted: Option<FormattedBody>,
        body: String,
        room: &Room,
        format: ContentFormat,
    ) {
        if let Some(formatted) = formatted.filter(is_valid_formatted_body).map(|f| f.body) {
            if !self.original_text_changed(&formatted) && !self.format_changed(format) {
                return;
            }

            if let Some(html_blocks) = parse_formatted_body(&formatted) {
                self.reset();
                self.set_original_text(formatted);
                self.set_format(format);

                self.build_html(html_blocks, room);
                return;
            }
        }

        if !self.original_text_changed(&body) && !self.format_changed(format) {
            return;
        }

        let linkified_body = linkify(&body);

        self.reset();
        self.set_original_text(body);
        self.set_format(format);

        self.build_text(linkified_body, WithMentions::Yes(room), false);
    }

    /// Display the given emote for `sender`.
    ///
    /// It will detect if it should display the body or the formatted body.
    pub fn with_emote(
        &self,
        formatted: Option<FormattedBody>,
        body: String,
        sender: Member,
        room: &Room,
        format: ContentFormat,
    ) {
        if let Some(body) = formatted.filter(is_valid_formatted_body).map(|f| f.body) {
            if !self.original_text_changed(&body)
                && !self.format_changed(format)
                && !self.sender_changed(&sender)
            {
                return;
            }

            let with_sender = format!("<b>{}</b> {body}", sender.display_name());

            if let Some(html_blocks) = parse_formatted_body(&with_sender) {
                self.reset();
                self.add_css_class("emote");
                self.set_original_text(body);
                self.set_is_html(true);
                self.set_format(format);

                let handler = sender.connect_display_name_notify(
                    clone!(@weak self as obj, @weak room => move |sender| {
                        obj.update_emote(&room, &sender.display_name());
                    }),
                );
                self.imp().sender.set(&sender, vec![handler]);

                self.build_html(html_blocks, room);
                return;
            }
        }

        let body = linkify(&body);

        if !self.original_text_changed(&body)
            && !self.format_changed(format)
            && !self.sender_changed(&sender)
        {
            return;
        }

        let with_sender = format!("<b>{}</b> {body}", sender.display_name());

        self.reset();
        self.add_css_class("emote");
        self.set_original_text(body.clone());
        self.set_is_html(false);
        self.set_format(format);

        let handler = sender.connect_display_name_notify(
            clone!(@weak self as obj, @weak room => move |sender| {
                obj.update_emote(&room, &sender.display_name());
            }),
        );
        self.imp().sender.set(&sender, vec![handler]);

        self.build_text(with_sender, WithMentions::Yes(room), true);
    }

    fn update_emote(&self, room: &Room, sender_name: &str) {
        let with_sender = format!("<b>{sender_name}</b> {}", self.original_text());

        if self.is_html() {
            if let Some(html_blocks) = parse_formatted_body(&with_sender) {
                self.build_html(html_blocks, room);
                return;
            }
        }

        self.build_text(with_sender, WithMentions::Yes(room), true);
    }

    fn build_text(&self, text: String, with_mentions: WithMentions, use_markup: bool) {
        let ellipsize = self.format() == ContentFormat::Ellipsized;

        let (linkified, (label, widgets)) = match with_mentions {
            WithMentions::Yes(room) => (true, extract_mentions(&text, room)),
            WithMentions::No => (false, (text, Vec::new())),
        };

        // FIXME: This should not be necessary but spaces at the end of the string cause
        // criticals.
        let label = label.trim_end_matches(' ');

        if widgets.is_empty() {
            let child = if let Some(child) = self.child().and_downcast::<gtk::Label>() {
                child
            } else {
                let child = new_label();
                self.set_child(Some(&child));
                child
            };

            if EMOJI_REGEX.is_match(label) {
                child.add_css_class("emoji");
            } else {
                child.remove_css_class("emoji");
            }

            child.set_ellipsize(if ellipsize {
                pango::EllipsizeMode::End
            } else {
                pango::EllipsizeMode::None
            });

            child.set_use_markup(use_markup || linkified);
            child.set_label(label);
        } else {
            let widgets = widgets
                .into_iter()
                .map(|(p, _)| {
                    // Show the profile on click.
                    p.set_activatable(true);
                    p
                })
                .collect();
            let child = if let Some(child) = self.child().and_downcast::<LabelWithWidgets>() {
                child
            } else {
                let child = LabelWithWidgets::new();
                self.set_child(Some(&child));
                child
            };

            child.set_ellipsize(ellipsize);
            child.set_use_markup(true);
            child.set_label(Some(label.to_owned()));
            child.set_widgets(widgets);
        }
    }

    fn build_html(&self, blocks: Vec<HtmlBlock>, room: &Room) {
        let ellipsize = self.format() == ContentFormat::Ellipsized;

        if blocks.len() == 1 {
            let widget = create_widget_for_html_block(&blocks[0], room, ellipsize, false);
            self.set_child(Some(&widget));
        } else {
            let child = gtk::Grid::builder().row_spacing(6).build();
            self.set_child(Some(&child));

            for (row, block) in blocks.into_iter().enumerate() {
                let widget = create_widget_for_html_block(&block, room, ellipsize, true);
                child.attach(&widget, 0, row as i32, 1, 1);

                if ellipsize {
                    break;
                }
            }
        }
    }

    /// Whether the given text is different than the current original text.
    fn original_text_changed(&self, text: &str) -> bool {
        *self.imp().original_text.borrow() != text
    }

    /// Set the original text of the message to display.
    fn set_original_text(&self, text: String) {
        self.imp().original_text.replace(text);
        self.notify_original_text();
    }

    /// Set whether the original text of the message is HTML.
    fn set_is_html(&self, is_html: bool) {
        if self.is_html() == is_html {
            return;
        }

        self.imp().is_html.set(is_html);
        self.notify_is_html();
    }

    /// Whether the given format is different than the current format.
    fn format_changed(&self, format: ContentFormat) -> bool {
        self.format() != format
    }

    /// Set the text format.
    fn set_format(&self, format: ContentFormat) {
        self.imp().format.set(format);
        self.notify_format();
    }

    /// Whether the sender of the message changed.
    fn sender_changed(&self, sender: &Member) -> bool {
        self.imp().sender.obj().as_ref() == Some(sender)
    }

    /// Reset this `MessageText`.
    fn reset(&self) {
        self.imp().sender.disconnect_signals();
        self.remove_css_class("emote");
    }
}

/// Transform URLs into links.
fn linkify(text: &str) -> String {
    hoverify_links(&markup_links(&html_escape(text)))
}

/// Make links show up on hover.
fn hoverify_links(text: &str) -> String {
    let mut res = String::with_capacity(text.len());

    for (i, chunk) in text.split_inclusive("<a href=\"").enumerate() {
        if i > 0 {
            if let Some((url, end)) = chunk.split_once('"') {
                let escaped_url = html_escape(url);
                write!(&mut res, "{url}\" title=\"{escaped_url}\"{end}").unwrap();

                continue;
            }
        }

        res.push_str(chunk);
    }

    res
}

fn is_valid_formatted_body(formatted: &FormattedBody) -> bool {
    formatted.format == MessageFormat::Html && !formatted.body.contains("<!-- raw HTML omitted -->")
}

fn parse_formatted_body(formatted: &str) -> Option<Vec<HtmlBlock>> {
    markup_html(formatted).ok()
}

fn create_widget_for_html_block(
    block: &HtmlBlock,
    room: &Room,
    ellipsize: bool,
    has_more: bool,
) -> gtk::Widget {
    match block {
        HtmlBlock::Heading(n, s) => {
            let w = create_label_for_html(s, room, ellipsize, has_more);
            w.add_css_class(&format!("h{n}"));
            w
        }
        HtmlBlock::UList(elements) => {
            let grid = gtk::Grid::builder()
                .row_spacing(6)
                .column_spacing(6)
                .margin_end(6)
                .margin_start(6)
                .build();

            for (row, li) in elements.iter().enumerate() {
                let bullet = gtk::Label::builder()
                    .label("•")
                    .valign(gtk::Align::Baseline)
                    .build();

                let w = create_label_for_html(li, room, ellipsize, has_more || elements.len() > 1);

                grid.attach(&bullet, 0, row as i32, 1, 1);
                grid.attach(&w, 1, row as i32, 1, 1);

                if ellipsize {
                    break;
                }
            }

            grid.upcast()
        }
        HtmlBlock::OList(elements) => {
            let grid = gtk::Grid::builder()
                .row_spacing(6)
                .column_spacing(6)
                .margin_end(6)
                .margin_start(6)
                .build();

            for (row, ol) in elements.iter().enumerate() {
                let bullet = gtk::Label::builder()
                    .label(format!("{}.", row + 1))
                    .valign(gtk::Align::Baseline)
                    .build();

                let w = create_label_for_html(ol, room, ellipsize, has_more || elements.len() > 1);

                grid.attach(&bullet, 0, row as i32, 1, 1);
                grid.attach(&w, 1, row as i32, 1, 1);

                if ellipsize {
                    break;
                }
            }

            grid.upcast()
        }
        HtmlBlock::Code(s) => {
            if ellipsize {
                let label = if let Some(pos) = s.find('\n') {
                    format!("<tt>{}…</tt>", &s[0..pos])
                } else if has_more {
                    format!("<tt>{s}…</tt>")
                } else {
                    format!("<tt>{s}</tt>")
                };

                gtk::Label::builder()
                    .label(label)
                    .use_markup(true)
                    .ellipsize(if ellipsize {
                        pango::EllipsizeMode::End
                    } else {
                        pango::EllipsizeMode::None
                    })
                    .build()
                    .upcast()
            } else {
                let scrolled = gtk::ScrolledWindow::new();
                scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
                let buffer = sourceview::Buffer::builder()
                    .highlight_matching_brackets(false)
                    .text(s)
                    .build();
                crate::utils::sourceview::setup_style_scheme(&buffer);
                let view = sourceview::View::builder()
                    .buffer(&buffer)
                    .editable(false)
                    .css_classes(["codeview", "frame"])
                    .hexpand(true)
                    .build();
                scrolled.set_child(Some(&view));
                scrolled.upcast()
            }
        }
        HtmlBlock::Quote(blocks) => {
            let grid = gtk::Grid::builder()
                .row_spacing(6)
                .css_classes(["quote"])
                .build();

            for (row, block) in blocks.iter().enumerate() {
                let w = create_widget_for_html_block(
                    block,
                    room,
                    ellipsize,
                    has_more || blocks.len() > 1,
                );
                grid.attach(&w, 0, row as i32, 1, 1);

                if ellipsize {
                    break;
                }
            }

            grid.upcast()
        }
        HtmlBlock::Text(s) => create_label_for_html(s, room, ellipsize, has_more).upcast(),
        HtmlBlock::Separator => gtk::Separator::new(gtk::Orientation::Horizontal).upcast(),
    }
}

fn new_label() -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .xalign(0.0)
        .valign(gtk::Align::Start)
        .build()
}

fn create_label_for_html(label: &str, room: &Room, ellipsize: bool, cut_text: bool) -> gtk::Widget {
    // FIXME: This should not be necessary but spaces at the end of the string cause
    // criticals.
    let label = label.trim_end_matches(' ');
    let (label, widgets) = extract_mentions(label, room);
    let mut label = hoverify_links(&label);
    if ellipsize && cut_text && !label.ends_with('…') && !label.ends_with("...") {
        label.push('…');
    }

    if widgets.is_empty() {
        let w = new_label();
        w.set_markup(&label);
        w.set_ellipsize(if ellipsize {
            pango::EllipsizeMode::End
        } else {
            pango::EllipsizeMode::None
        });
        w.upcast()
    } else {
        let widgets = widgets
            .into_iter()
            .map(|(p, _)| {
                // Show the profile on click.
                p.set_activatable(true);
                p
            })
            .collect();
        let w = LabelWithWidgets::with_label_and_widgets(&label, widgets);
        w.set_use_markup(true);
        w.set_ellipsize(ellipsize);
        w.upcast()
    }
}

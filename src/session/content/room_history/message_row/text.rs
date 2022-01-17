use adw::{prelude::BinExt, subclass::prelude::*};
use gtk::{glib, pango, prelude::*, subclass::prelude::*};
use html2pango::{
    block::{markup_html, HtmlBlock},
    html_escape, markup_links,
};
use log::warn;
use matrix_sdk::ruma::events::room::message::{FormattedBody, MessageFormat};
use once_cell::sync::Lazy;
use regex::Regex;
use sourceview::prelude::*;

use crate::session::{
    content::room_history::ItemRow,
    room::{EventActions, Member},
    UserExt,
};

static EMOJI_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?x)
        ^
        [\p{White_Space}\p{Emoji_Component}]*
        [\p{Emoji}--\p{Decimal_Number}]+
        [\p{White_Space}\p{Emoji}\p{Emoji_Component}--\p{Decimal_Number}]*
        $
        # That string is made of at least one emoji, except digits, possibly more,
        # possibly with modifiers, possibly with spaces, but nothing else
        ",
    )
    .unwrap()
});

mod imp {
    use super::*;
    use once_cell::sync::Lazy;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct MessageText {
        /// The displayed content of the message.
        pub body: RefCell<Option<String>>,
        /// The sender of the message(only used for emotes).
        pub sender: RefCell<Option<Member>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageText {
        const NAME: &'static str = "ContentMessageText";
        type Type = super::MessageText;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for MessageText {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_string(
                        "body",
                        "Body",
                        "The displayed content of the message",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_object(
                        "sender",
                        "Sender",
                        "The sender of the message",
                        Member::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "body" => obj.set_body(value.get().unwrap()),
                "sender" => obj.set_sender(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "body" => obj.body().to_value(),
                "sender" => obj.sender().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);
        }
    }

    impl WidgetImpl for MessageText {}

    impl BinImpl for MessageText {}
}

glib::wrapper! {
    /// A widget displaying the content of a text message.
    pub struct MessageText(ObjectSubclass<imp::MessageText>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageText {
    /// Creates a text widget.
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create MessageText")
    }

    /// Display the given plain text.
    pub fn text(&self, body: String) {
        self.build_text(&body, false);
        self.set_body(Some(body));
    }

    /// Display the given text with markup.
    ///
    /// It will detect if it should display the body or the formatted body.
    pub fn markup(&self, formatted: Option<FormattedBody>, body: String) {
        if let Some((html_blocks, body)) =
            formatted
                .filter(is_valid_formatted_body)
                .and_then(|formatted| {
                    parse_formatted_body(strip_reply(&formatted.body))
                        .map(|blocks| (blocks, formatted.body))
                })
        {
            self.build_html(html_blocks);
            self.set_body(Some(body));
        } else {
            let body = linkify(strip_reply(&body));
            self.build_text(&body, true);
            self.set_body(Some(body));
        }
    }

    pub fn set_body(&self, body: Option<String>) {
        let priv_ = imp::MessageText::from_instance(self);

        if body.as_ref() == priv_.body.borrow().as_ref() {
            return;
        }

        priv_.body.replace(body);
    }

    pub fn body(&self) -> Option<String> {
        let priv_ = imp::MessageText::from_instance(self);
        priv_.body.borrow().to_owned()
    }

    pub fn set_sender(&self, sender: Option<Member>) {
        let priv_ = imp::MessageText::from_instance(self);

        if sender.as_ref() == priv_.sender.borrow().as_ref() {
            return;
        }

        priv_.sender.replace(sender);
        self.notify("sender");
    }

    pub fn sender(&self) -> Option<Member> {
        let priv_ = imp::MessageText::from_instance(self);
        priv_.sender.borrow().to_owned()
    }

    /// Display the given emote for `sender`.
    ///
    /// It will detect if it should display the body or the formatted body.
    pub fn emote(&self, formatted: Option<FormattedBody>, body: String, sender: Member) {
        if let Some(body) = formatted
            .filter(is_valid_formatted_body)
            .and_then(|formatted| {
                let body = format!("<b>{}</b> {}", sender.display_name(), formatted.body);

                parse_formatted_body(&body).map(|_| formatted.body)
            })
        {
            // TODO: we need to bind the display name to the sender
            let formatted = FormattedBody {
                body: format!("<b>{}</b> {}", sender.display_name(), strip_reply(&body)),
                format: MessageFormat::Html,
            };

            let html = parse_formatted_body(&formatted.body).unwrap();
            self.build_html(html);
            self.set_body(Some(body));
            self.set_sender(Some(sender));
        } else {
            // TODO: we need to bind the display name to the sender
            let body = linkify(&body);
            self.build_text(&format!("<b>{}</b> {}", sender.display_name(), &body), true);
            self.set_body(Some(body));
            self.set_sender(Some(sender));
        }
    }

    fn build_text(&self, text: &str, use_markup: bool) {
        let child = if let Some(Ok(child)) = self.child().map(|w| w.downcast::<gtk::Label>()) {
            child
        } else {
            let child = gtk::Label::new(None);
            set_label_styles(&child);
            self.set_child(Some(&child));
            child
        };

        if EMOJI_REGEX.is_match(text) {
            child.add_css_class("emoji");
        }

        if use_markup {
            child.set_markup(text);
        } else {
            child.set_text(text);
        }
    }

    fn build_html(&self, blocks: Vec<HtmlBlock>) {
        let child = gtk::Box::new(gtk::Orientation::Vertical, 6);
        self.set_child(Some(&child));

        for block in blocks {
            let widget = create_widget_for_html_block(&block);
            child.append(&widget);
        }
    }
}

fn linkify(text: &str) -> String {
    markup_links(&html_escape(text))
}

fn is_valid_formatted_body(formatted: &FormattedBody) -> bool {
    formatted.format == MessageFormat::Html && !formatted.body.contains("<!-- raw HTML omitted -->")
}

fn parse_formatted_body(formatted: &str) -> Option<Vec<HtmlBlock>> {
    markup_html(formatted).ok()
}

fn set_label_styles(w: &gtk::Label) {
    w.set_wrap(true);
    w.set_wrap_mode(pango::WrapMode::WordChar);
    w.set_justify(gtk::Justification::Left);
    w.set_xalign(0.0);
    w.set_valign(gtk::Align::Start);
    w.set_halign(gtk::Align::Fill);
    w.set_selectable(true);
    w.set_extra_menu(Some(ItemRow::event_message_menu_model()));
}

fn create_widget_for_html_block(block: &HtmlBlock) -> gtk::Widget {
    match block {
        HtmlBlock::Heading(n, s) => {
            let w = gtk::Label::new(None);
            set_label_styles(&w);
            w.set_markup(s);
            w.add_css_class(&format!("h{}", n));
            w.upcast::<gtk::Widget>()
        }
        HtmlBlock::UList(elements) => {
            let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
            bx.set_margin_end(6);
            bx.set_margin_start(6);

            for li in elements.iter() {
                let h_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let bullet = gtk::Label::new(Some("•"));
                bullet.set_valign(gtk::Align::Start);
                let w = gtk::Label::new(None);
                set_label_styles(&w);
                h_box.append(&bullet);
                h_box.append(&w);
                w.set_markup(li);
                bx.append(&h_box);
            }

            bx.upcast::<gtk::Widget>()
        }
        HtmlBlock::OList(elements) => {
            let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
            bx.set_margin_end(6);
            bx.set_margin_start(6);

            for (i, ol) in elements.iter().enumerate() {
                let h_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let bullet = gtk::Label::new(Some(&format!("{}.", i + 1)));
                bullet.set_valign(gtk::Align::Start);
                let w = gtk::Label::new(None);
                set_label_styles(&w);
                h_box.append(&bullet);
                h_box.append(&w);
                w.set_markup(ol);
                bx.append(&h_box);
            }

            bx.upcast::<gtk::Widget>()
        }
        HtmlBlock::Code(s) => {
            let scrolled = gtk::ScrolledWindow::new();
            scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
            let buffer = sourceview::Buffer::new(None);
            buffer.set_highlight_matching_brackets(false);
            buffer.set_text(s);
            crate::utils::setup_style_scheme(&buffer);
            let view = sourceview::View::with_buffer(&buffer);
            view.set_editable(false);
            view.add_css_class("codeview");
            scrolled.set_child(Some(&view));
            scrolled.upcast::<gtk::Widget>()
        }
        HtmlBlock::Quote(blocks) => {
            let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
            bx.add_css_class("quote");
            for block in blocks.iter() {
                let w = create_widget_for_html_block(block);
                bx.append(&w);
            }
            bx.upcast::<gtk::Widget>()
        }
        HtmlBlock::Text(s) => {
            let w = gtk::Label::new(None);
            set_label_styles(&w);
            w.set_markup(s);
            w.upcast::<gtk::Widget>()
        }
    }
}

/// Remove the content between `mx-reply` tags.
///
/// Returns the unchanged string if none was found to be able to chain calls.
fn strip_reply(text: &str) -> &str {
    if let Some(end) = text.find("</mx-reply>") {
        if !text.starts_with("<mx-reply>") {
            warn!("Received a rich reply that doesn't start with '<mx-reply>'");
        }

        &text[end + 11..]
    } else {
        text
    }
}

impl Default for MessageText {
    fn default() -> Self {
        Self::new()
    }
}

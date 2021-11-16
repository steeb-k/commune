use adw::{prelude::BinExt, subclass::prelude::*};
use gtk::{gio, glib, pango, prelude::*, subclass::prelude::*};
use html2pango::{
    block::{markup_html, HtmlBlock},
    html_escape, markup_links,
};
use matrix_sdk::ruma::events::room::message::{FormattedBody, MessageFormat};
use sourceview::prelude::*;

use crate::session::{room::Member, UserExt};

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "TextFormat")]
pub enum TextFormat {
    Text = 0,
    Markup = 1,
    Html = 2,
    Emote = 3,
    HtmlEmote = 4,
}

impl Default for TextFormat {
    fn default() -> Self {
        TextFormat::Text
    }
}

mod imp {
    use super::*;
    use once_cell::sync::Lazy;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default)]
    pub struct MessageText {
        /// The format of the text message.
        pub format: Cell<TextFormat>,
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
                    glib::ParamSpec::new_enum(
                        "format",
                        "Format",
                        "The format of the text message",
                        TextFormat::static_type(),
                        TextFormat::default() as i32,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "body",
                        "Body",
                        "The displayed content of the message",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
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
                "format" => obj.set_format(value.get().unwrap()),
                "body" => obj.set_body(value.get().unwrap()),
                "sender" => obj.set_sender(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "format" => obj.format().to_value(),
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
    // Creates a widget that displays plain text.
    pub fn text(body: String) -> Self {
        glib::Object::new(&[("body", &body)]).expect("Failed to create MessageText")
    }

    // Creates a widget that displays text with markup. It will detect if it should display the body or the formatted body.
    pub fn markup(formatted: Option<FormattedBody>, body: String) -> Self {
        if let Some((html_blocks, body)) = formatted
            .filter(|formatted| is_valid_formatted_body(formatted))
            .and_then(|formatted| {
                parse_formatted_body(&formatted.body)
                    .and_then(|blocks| Some((blocks, formatted.body)))
            })
        {
            let self_: Self = glib::Object::new(&[("format", &TextFormat::Html), ("body", &body)])
                .expect("Failed to create MessageText");

            self_.build_html(html_blocks);
            self_
        } else {
            let self_: Self =
                glib::Object::new(&[("format", &TextFormat::Markup), ("body", &linkify(&body))])
                    .expect("Failed to create MessageText");

            self_.build();
            self_
        }
    }

    // Creates a widget that displays an emote. It will detect if it should display the body or the formatted body.
    pub fn emote(formatted: Option<FormattedBody>, body: String, sender: Member) -> Self {
        if let Some(body) = formatted
            .filter(|formatted| is_valid_formatted_body(formatted))
            .and_then(|formatted| {
                let body = format!("<b>{}</b> {}", sender.display_name(), formatted.body);

                parse_formatted_body(&body).and_then(|_| Some(formatted.body))
            })
        {
            glib::Object::new(&[
                ("format", &TextFormat::HtmlEmote),
                ("body", &body),
                ("sender", &Some(sender)),
            ])
            .expect("Failed to create MessageText")
        } else {
            glib::Object::new(&[
                ("format", &TextFormat::Emote),
                ("body", &linkify(&body)),
                ("sender", &Some(sender)),
            ])
            .expect("Failed to create MessageText")
        }
    }

    pub fn set_format(&self, format: TextFormat) {
        let priv_ = imp::MessageText::from_instance(self);

        if format == priv_.format.get() {
            return;
        }

        priv_.format.set(format);
    }

    pub fn format(&self) -> TextFormat {
        let priv_ = imp::MessageText::from_instance(self);
        priv_.format.get()
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

        if self.format() == TextFormat::Emote || self.format() == TextFormat::HtmlEmote {
            self.build();
        }

        self.notify("sender");
    }

    pub fn sender(&self) -> Option<Member> {
        let priv_ = imp::MessageText::from_instance(self);
        priv_.sender.borrow().to_owned()
    }

    fn build(&self) {
        match self.format() {
            TextFormat::Text => {
                self.build_text(&self.body().unwrap(), false);
            }
            TextFormat::Markup => {
                self.build_text(&self.body().unwrap(), true);
            }
            TextFormat::Html => {
                let formatted = FormattedBody {
                    body: self.body().unwrap(),
                    format: MessageFormat::Html,
                };

                let html = parse_formatted_body(&formatted.body).unwrap();
                self.build_html(html);
            }
            TextFormat::Emote => {
                // TODO: we need to bind the display name to the sender
                self.build_text(
                    &format!(
                        "<b>{}</b> {}",
                        self.sender().unwrap().display_name(),
                        &self.body().unwrap()
                    ),
                    true,
                );
            }
            TextFormat::HtmlEmote => {
                // TODO: we need to bind the display name to the sender
                let formatted = FormattedBody {
                    body: format!(
                        "<b>{}</b> {}",
                        self.sender().unwrap().display_name(),
                        self.body().unwrap()
                    ),
                    format: MessageFormat::Html,
                };

                let html = parse_formatted_body(&formatted.body).unwrap();
                self.build_html(html);
            }
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
    let menu_model: Option<gio::MenuModel> =
        gtk::Builder::from_resource("/org/gnome/FractalNext/content-item-row-menu.ui")
            .object("menu_model");
    w.set_extra_menu(menu_model.as_ref());
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

impl Default for MessageText {
    fn default() -> Self {
        Self::text(format!(""))
    }
}

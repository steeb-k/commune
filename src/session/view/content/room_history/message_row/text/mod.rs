use adw::{prelude::BinExt, subclass::prelude::*};
use gtk::{glib, glib::clone, pango, prelude::*};
use matrix_sdk::ruma::events::room::message::FormattedBody;
use once_cell::sync::Lazy;
use ruma::{
    events::room::message::MessageFormat,
    html::{Html, ListBehavior, SanitizerConfig},
};

mod inline_html;
#[cfg(test)]
mod tests;
mod widgets;

use self::widgets::{new_message_label, widget_for_html_nodes, HtmlWidgetConfig};
use super::ContentFormat;
use crate::{
    components::{AtRoom, LabelWithWidgets},
    prelude::*,
    session::model::{Member, Room},
    utils::{
        string::{Linkifier, PangoStrMutExt},
        BoundObjectWeakRef, EMOJI_REGEX,
    },
};

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
        /// Whether the message might contain an `@room` mention.
        pub detect_at_room: Cell<bool>,
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
    pub fn with_plain_text(&self, body: String, format: ContentFormat) {
        if !self.original_text_changed(&body) && !self.format_changed(format) {
            return;
        }

        self.reset();
        self.set_format(format);

        let mut escaped_body = body.escape_markup();
        escaped_body.truncate_end_whitespaces();

        self.build_plain_text(escaped_body);
        self.set_original_text(body);
    }

    /// Display the given text with possible markup.
    ///
    /// It will detect if it should display the body or the formatted body.
    pub fn with_markup(
        &self,
        formatted: Option<FormattedBody>,
        body: String,
        room: &Room,
        format: ContentFormat,
        detect_at_room: bool,
    ) {
        self.set_detect_at_room(detect_at_room);

        if let Some(formatted) = formatted.filter(formatted_body_is_html).map(|f| f.body) {
            if !self.original_text_changed(&formatted) && !self.format_changed(format) {
                return;
            }

            self.reset();
            self.set_format(format);

            if self.build_html(&formatted, room, None).is_ok() {
                self.set_original_text(formatted);
                return;
            }
        }

        if !self.original_text_changed(&body) && !self.format_changed(format) {
            return;
        }

        self.reset();
        self.set_format(format);

        self.build_text(&body, room, None);
        self.set_original_text(body);
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
        detect_at_room: bool,
    ) {
        self.set_detect_at_room(detect_at_room);

        if let Some(formatted) = formatted.filter(formatted_body_is_html).map(|f| f.body) {
            if !self.original_text_changed(&body)
                && !self.format_changed(format)
                && !self.sender_changed(&sender)
            {
                return;
            }

            self.reset();
            self.set_format(format);

            let sender_name = sender.disambiguated_name();

            if self
                .build_html(&formatted, room, Some(&sender_name))
                .is_ok()
            {
                self.add_css_class("emote");
                self.set_is_html(true);
                self.set_original_text(formatted);

                let handler = sender.connect_disambiguated_name_notify(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    #[weak]
                    room,
                    move |sender| {
                        obj.update_emote(&room, &sender.disambiguated_name());
                    }
                ));
                self.imp().sender.set(&sender, vec![handler]);

                return;
            }
        }

        if !self.original_text_changed(&body)
            && !self.format_changed(format)
            && !self.sender_changed(&sender)
        {
            return;
        }

        self.reset();
        self.set_format(format);
        self.add_css_class("emote");
        self.set_is_html(false);

        let sender_name = sender.disambiguated_name();
        self.build_text(&body, room, Some(&sender_name));
        self.set_original_text(body);

        let handler = sender.connect_disambiguated_name_notify(clone!(
            #[weak(rename_to = obj)]
            self,
            #[weak]
            room,
            move |sender| {
                obj.update_emote(&room, &sender.disambiguated_name());
            }
        ));
        self.imp().sender.set(&sender, vec![handler]);
    }

    fn update_emote(&self, room: &Room, sender_name: &str) {
        let text = self.original_text();

        if self.is_html() && self.build_html(&text, room, Some(sender_name)).is_ok() {
            return;
        }

        self.build_text(&text, room, Some(sender_name));
    }

    /// Build the message for the given plain text.
    ///
    /// The text must have been escaped and the end whitespaces removed before
    /// calling this method.
    fn build_plain_text(&self, mut text: String) {
        let child = if let Some(child) = self.child().and_downcast::<gtk::Label>() {
            child
        } else {
            let child = new_message_label();
            self.set_child(Some(&child));
            child
        };

        if EMOJI_REGEX.is_match(&text) {
            child.add_css_class("emoji");
        } else {
            child.remove_css_class("emoji");
        }

        let ellipsize = self.format() == ContentFormat::Ellipsized;
        if ellipsize {
            text.truncate_newline();
        }

        let ellipsize_mode = if ellipsize {
            pango::EllipsizeMode::End
        } else {
            pango::EllipsizeMode::None
        };
        child.set_ellipsize(ellipsize_mode);

        child.set_label(&text);
    }

    /// Build the message for the given text in the given room.
    ///
    /// We will try to detect URIs in the text.
    ///
    /// If `detect_at_room` is `true`, we will try to detect `@room` in the
    /// text.
    ///
    /// If `sender_name` is provided, it is added as a prefix. This is used for
    /// emotes.
    fn build_text(&self, text: &str, room: &Room, mut sender_name: Option<&str>) {
        let detect_at_room = self.detect_at_room();
        let mut result = String::with_capacity(text.len());

        result.maybe_append_emote_name(&mut sender_name);

        let mut pills = Vec::new();
        Linkifier::new(&mut result)
            .detect_mentions(room, &mut pills, detect_at_room)
            .linkify(text);

        result.truncate_end_whitespaces();

        if pills.is_empty() {
            self.build_plain_text(result);
            return;
        };

        let ellipsize = self.format() == ContentFormat::Ellipsized;
        pills.iter().for_each(|p| {
            if !p.source().is_some_and(|s| s.is::<AtRoom>()) {
                // Show the profile on click.
                p.set_activatable(true);
            }
        });

        let child = if let Some(child) = self.child().and_downcast::<LabelWithWidgets>() {
            child
        } else {
            let child = LabelWithWidgets::new();
            self.set_child(Some(&child));
            child
        };

        child.set_ellipsize(ellipsize);
        child.set_use_markup(true);
        child.set_label(Some(result));
        child.set_widgets(pills);
    }

    /// Build the message for the given HTML in the given room.
    ///
    /// We will try to detect URIs in the text.
    ///
    /// If `detect_at_room` is `true`, we will try to detect `@room` in the
    /// text.
    ///
    /// If `sender_name` is provided, it is added as a prefix. This is used for
    /// emotes.
    ///
    /// Returns an error if the HTML string doesn't contain any HTML.
    fn build_html(&self, html: &str, room: &Room, mut sender_name: Option<&str>) -> Result<(), ()> {
        let detect_at_room = self.detect_at_room();
        let ellipsize = self.format() == ContentFormat::Ellipsized;

        let html = Html::parse(html.trim_matches('\n'));
        html.sanitize_with(&HTML_MESSAGE_SANITIZER_CONFIG);

        if !html.has_children() {
            return Err(());
        }

        let Some(child) = widget_for_html_nodes(
            html.children(),
            HtmlWidgetConfig {
                room,
                detect_at_room,
                ellipsize,
            },
            false,
            &mut sender_name,
        ) else {
            return Err(());
        };

        self.set_child(Some(&child));

        Ok(())
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

    /// Set whether the message might contain an `@room` mention.
    fn detect_at_room(&self) -> bool {
        self.imp().detect_at_room.get()
    }

    /// Set whether the message might contain an `@room` mention.
    fn set_detect_at_room(&self, detect_at_room: bool) {
        self.imp().detect_at_room.set(detect_at_room);
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

impl Default for MessageText {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the given [`FormattedBody`] contains HTML.
fn formatted_body_is_html(formatted: &FormattedBody) -> bool {
    formatted.format == MessageFormat::Html && !formatted.body.contains("<!-- raw HTML omitted -->")
}

/// All supported inline elements from the Matrix spec.
const SUPPORTED_INLINE_ELEMENTS: &[&str] = &[
    "del", "a", "sup", "sub", "b", "i", "u", "strong", "em", "s", "code", "br", "span",
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
static HTML_MESSAGE_SANITIZER_CONFIG: Lazy<SanitizerConfig> = Lazy::new(|| {
    SanitizerConfig::compat()
        .allow_elements(
            SUPPORTED_INLINE_ELEMENTS
                .iter()
                .chain(SUPPORTED_BLOCK_ELEMENTS.iter())
                .copied(),
            ListBehavior::Override,
        )
        .remove_reply_fallback()
});

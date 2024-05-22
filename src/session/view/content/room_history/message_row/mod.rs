use adw::{prelude::*, subclass::prelude::*};
use gtk::{gdk, glib, glib::clone, CompositeTemplate};
use matrix_sdk::ruma::events::room::message::MessageType;
use tracing::warn;

mod audio;
mod caption;
mod content;
mod file;
mod location;
mod media;
mod message_state_stack;
mod reaction;
mod reaction_list;
mod reply;
mod text;

pub use self::content::{ContentFormat, MessageContent};
use self::{message_state_stack::MessageStateStack, reaction_list::MessageReactionList};
use super::{ReadReceiptsList, SenderAvatar};
use crate::{
    gettext_f, session::model::Event, system_settings::ClockFormat, utils::BoundObject,
    Application, Window,
};

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/mod.ui"
    )]
    #[properties(wrapper_type = super::MessageRow)]
    pub struct MessageRow {
        #[template_child]
        pub avatar: TemplateChild<SenderAvatar>,
        #[template_child]
        pub header: TemplateChild<gtk::Box>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub timestamp: TemplateChild<gtk::Label>,
        #[template_child]
        pub content: TemplateChild<MessageContent>,
        #[template_child]
        pub message_state: TemplateChild<MessageStateStack>,
        #[template_child]
        pub reactions: TemplateChild<MessageReactionList>,
        #[template_child]
        pub read_receipts: TemplateChild<ReadReceiptsList>,
        pub bindings: RefCell<Vec<glib::Binding>>,
        pub system_settings_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// The event that is presented.
        #[property(get, set = Self::set_event, explicit_notify)]
        pub event: BoundObject<Event>,
        /// Whether this item should show its header.
        ///
        /// This is ignored if this event doesn’t have a header.
        #[property(get = Self::show_header, set = Self::set_show_header, explicit_notify)]
        pub show_header: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageRow {
        const NAME: &'static str = "ContentMessageRow";
        type Type = super::MessageRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action("message-row.show-media", None, |obj, _, _| {
                obj.show_media();
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageRow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.content
                .connect_format_notify(clone!(@weak self as imp => move |content|
                    imp.reactions.set_visible(!matches!(
                        content.format(),
                        ContentFormat::Compact | ContentFormat::Ellipsized
                    ));
                ));

            let system_settings = Application::default().system_settings();
            let system_settings_handler =
                system_settings.connect_clock_format_notify(clone!(@weak obj => move |_| {
                    obj.update_timestamp();
                }));
            self.system_settings_handler
                .replace(Some(system_settings_handler));
        }

        fn dispose(&self) {
            for binding in self.bindings.take() {
                binding.unbind();
            }

            if let Some(handler) = self.system_settings_handler.take() {
                Application::default().system_settings().disconnect(handler);
            }
        }
    }

    impl WidgetImpl for MessageRow {}
    impl BinImpl for MessageRow {}

    impl MessageRow {
        /// Whether this item should show its header.
        ///
        /// This is ignored if this event doesn’t have a header.
        fn show_header(&self) -> bool {
            self.avatar.is_visible() && self.header.is_visible()
        }

        /// Set whether this item should show its header.
        fn set_show_header(&self, visible: bool) {
            let obj = self.obj();

            self.avatar.set_visible(visible);
            self.header.set_visible(visible);

            if let Some(row) = obj.parent() {
                if visible {
                    row.add_css_class("has-header");
                } else {
                    row.remove_css_class("has-header");
                }
            }

            obj.notify_show_header();
        }

        /// Set the event that is presented.
        fn set_event(&self, event: Event) {
            let obj = self.obj();

            // Remove signals and bindings from the previous event.
            self.event.disconnect_signals();
            while let Some(binding) = self.bindings.borrow_mut().pop() {
                binding.unbind();
            }

            self.avatar.set_sender(Some(event.sender()));

            let display_name_binding = event
                .sender()
                .bind_property("disambiguated-name", &*self.display_name, "label")
                .sync_create()
                .build();

            let show_header_binding = event
                .bind_property("show-header", &*obj, "show-header")
                .sync_create()
                .build();

            let state_binding = event
                .bind_property("state", &*self.message_state, "state")
                .sync_create()
                .build();

            self.bindings.borrow_mut().append(&mut vec![
                display_name_binding,
                show_header_binding,
                state_binding,
            ]);

            let timestamp_handler = event.connect_timestamp_notify(clone!(@weak obj => move |_| {
                obj.update_timestamp();
            }));

            // Listening to changes in the source might not be enough, there are changes
            // that we display that don't affect the source, like related events.
            let item_changed_handler = event.connect_item_changed(clone!(@weak obj => move |_| {
                obj.update_content();
            }));

            self.reactions
                .set_reaction_list(&event.room().get_or_create_members(), &event.reactions());
            self.read_receipts.set_source(&event.read_receipts());
            self.event
                .set(event, vec![timestamp_handler, item_changed_handler]);
            obj.notify_event();

            obj.update_content();
            obj.update_timestamp();
        }
    }
}

glib::wrapper! {
    /// A row displaying a message in the timeline.
    pub struct MessageRow(ObjectSubclass<imp::MessageRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_content_format(&self, format: ContentFormat) {
        self.imp().content.set_format(format);
    }

    /// Update the displayed timestamp for the current event with the current
    /// clock format setting.
    fn update_timestamp(&self) {
        let Some(event) = self.event() else {
            return;
        };
        let imp = self.imp();

        let datetime = event.timestamp();

        let clock_format = Application::default().system_settings().clock_format();
        let time = if clock_format == ClockFormat::TwelveHours {
            datetime.format("%I∶%M %p").unwrap()
        } else {
            datetime.format("%R").unwrap()
        };

        imp.timestamp.set_label(&time);

        let accessible_label = gettext_f("Sent at {time}", &[("time", &time)]);
        imp.timestamp
            .update_property(&[gtk::accessible::Property::Label(&accessible_label)]);
    }

    fn update_content(&self) {
        let Some(event) = self.event() else {
            return;
        };

        self.imp().content.update_for_event(&event);
    }

    /// Get the texture displayed by this widget, if any.
    pub fn texture(&self) -> Option<gdk::Texture> {
        self.imp().content.texture()
    }

    /// Open the media viewer with the media content of this row.
    fn show_media(&self) {
        let Some(window) = self.root().and_downcast::<Window>() else {
            return;
        };
        let Some(event) = self.event() else {
            return;
        };
        let Some(message) = event.message() else {
            return;
        };

        if matches!(message, MessageType::Image(_) | MessageType::Video(_)) {
            let Some(media_widget) = self.imp().content.media_widget() else {
                warn!("Trying to show media of a non-media message");
                return;
            };

            window.session_view().show_media(&event, &media_widget);
        }
    }
}

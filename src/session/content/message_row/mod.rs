mod file;
mod image;
mod text;

use crate::components::Avatar;
use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    glib, glib::clone, glib::signal::SignalHandlerId, subclass::prelude::*, CompositeTemplate,
};
use log::warn;
use matrix_sdk::ruma::events::{
    room::message::{MessageType, Relation},
    room::redaction::RoomRedactionEventContent,
    AnyMessageEventContent, AnySyncMessageEvent, AnySyncRoomEvent,
};

use self::{file::MessageFile, image::MessageImage, text::MessageText};
use crate::prelude::*;
use crate::session::room::Event;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content-message-row.ui")]
    pub struct MessageRow {
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        #[template_child]
        pub header: TemplateChild<gtk::Box>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub timestamp: TemplateChild<gtk::Label>,
        #[template_child]
        pub content: TemplateChild<adw::Bin>,
        pub relates_to_changed_handler: RefCell<Option<SignalHandlerId>>,
        pub bindings: RefCell<Vec<glib::Binding>>,
        pub event: RefCell<Option<Event>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageRow {
        const NAME: &'static str = "ContentMessageRow";
        type Type = super::MessageRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Avatar::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MessageRow {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_boolean(
                        "show-header",
                        "Show Header",
                        "Whether this item should show a header. This does nothing if this event doesn’t have a header. ",
                        false,
                        glib::ParamFlags::READWRITE,
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
                "show-header" => {
                    let show_header = value.get().unwrap();
                    let _ = obj.set_show_header(show_header);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "show-header" => obj.show_header().to_value(),
                _ => unimplemented!(),
            }
        }
    }
    impl WidgetImpl for MessageRow {}
    impl BinImpl for MessageRow {}
}

glib::wrapper! {
    pub struct MessageRow(ObjectSubclass<imp::MessageRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

//TODO
// - [] Implement widgets to show message events
impl MessageRow {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create MessageRow")
    }

    pub fn show_header(&self) -> bool {
        let priv_ = imp::MessageRow::from_instance(self);
        priv_.avatar.is_visible() && priv_.header.is_visible()
    }

    pub fn set_show_header(&self, visible: bool) {
        let priv_ = imp::MessageRow::from_instance(self);
        priv_.avatar.set_visible(visible);
        priv_.header.set_visible(visible);

        if let Some(list_item) = self.parent().and_then(|w| w.parent()) {
            if visible {
                list_item.set_css_classes(&["has-header"]);
            } else {
                list_item.remove_css_class("has-header");
            }
        }

        self.notify("show-header");
    }

    pub fn set_event(&self, event: Event) {
        let priv_ = imp::MessageRow::from_instance(self);
        // Remove signals and bindings from the previous event
        if let Some(event) = priv_.event.take() {
            if let Some(relates_to_changed_handler) = priv_.relates_to_changed_handler.take() {
                event.disconnect(relates_to_changed_handler);
            }

            while let Some(binding) = priv_.bindings.borrow_mut().pop() {
                binding.unbind();
            }
        }

        priv_.avatar.set_item(Some(event.sender().avatar().clone()));

        let display_name_binding = event
            .sender()
            .bind_property("display-name", &priv_.display_name.get(), "label")
            .flags(glib::BindingFlags::SYNC_CREATE)
            .build()
            .unwrap();

        let show_header_binding = event
            .bind_property("show-header", self, "show-header")
            .flags(glib::BindingFlags::SYNC_CREATE)
            .build()
            .unwrap();

        let timestamp_binding = event
            .bind_property("time", &*priv_.timestamp, "label")
            .flags(glib::BindingFlags::SYNC_CREATE)
            .build()
            .unwrap();

        priv_.bindings.borrow_mut().append(&mut vec![
            display_name_binding,
            show_header_binding,
            timestamp_binding,
        ]);

        priv_
            .relates_to_changed_handler
            .replace(Some(event.connect_relates_to_changed(
                clone!(@weak self as obj => move |event| {
                    obj.update_content(event);
                }),
            )));
        self.update_content(&event);
        priv_.event.replace(Some(event));
    }

    fn find_last_event(&self, event: &Event) -> Event {
        if let Some(replacement_event) = event.relates_to().iter().rev().find(|event| {
            let matrix_event = event.matrix_event();
            match matrix_event {
                Some(AnySyncRoomEvent::Message(AnySyncMessageEvent::RoomMessage(message))) => {
                    message
                        .content
                        .relates_to
                        .filter(|relation| matches!(relation, Relation::Replacement(_)))
                        .is_some()
                }
                Some(AnySyncRoomEvent::Message(AnySyncMessageEvent::RoomRedaction(_))) => true,
                _ => false,
            }
        }) {
            if !replacement_event.relates_to().is_empty() {
                self.find_last_event(replacement_event)
            } else {
                replacement_event.clone()
            }
        } else {
            event.clone()
        }
    }
    /// Find the content we need to display
    fn find_content(&self, event: &Event) -> Option<AnyMessageEventContent> {
        match self.find_last_event(event).matrix_event() {
            Some(AnySyncRoomEvent::Message(message)) => Some(message.content()),
            Some(AnySyncRoomEvent::RedactedMessage(message)) => {
                if let Some(ref redaction_event) = message.unsigned().redacted_because {
                    Some(AnyMessageEventContent::RoomRedaction(
                        redaction_event.content.clone(),
                    ))
                } else {
                    Some(AnyMessageEventContent::RoomRedaction(
                        RoomRedactionEventContent::new(),
                    ))
                }
            }
            Some(AnySyncRoomEvent::RedactedState(state)) => {
                if let Some(ref redaction_event) = state.unsigned().redacted_because {
                    Some(AnyMessageEventContent::RoomRedaction(
                        redaction_event.content.clone(),
                    ))
                } else {
                    Some(AnyMessageEventContent::RoomRedaction(
                        RoomRedactionEventContent::new(),
                    ))
                }
            }
            _ => None,
        }
    }

    fn update_content(&self, event: &Event) {
        let priv_ = imp::MessageRow::from_instance(self);
        let content = self.find_content(event);

        // TODO: create widgets for all event types
        // TODO: display reaction events from event.relates_to()
        // TODO: we should reuse the already present child widgets when possible

        match content {
            Some(AnyMessageEventContent::RoomMessage(message)) => {
                let msgtype = if let Some(Relation::Replacement(replacement)) = message.relates_to {
                    replacement.new_content.msgtype
                } else {
                    message.msgtype
                };
                match msgtype {
                    MessageType::Audio(_message) => {}
                    MessageType::Emote(message) => {
                        let child =
                            MessageText::emote(message.formatted, message.body, event.sender());
                        priv_.content.set_child(Some(&child));
                    }
                    MessageType::File(message) => {
                        let filename = message.filename.unwrap_or(message.body);
                        let child = MessageFile::new(Some(filename));
                        priv_.content.set_child(Some(&child));
                    }
                    MessageType::Image(message) => {
                        let child = MessageImage::image(message, &event.room().session());
                        priv_.content.set_child(Some(&child));
                    }
                    MessageType::Location(_message) => {}
                    MessageType::Notice(message) => {
                        let child = MessageText::markup(message.formatted, message.body);
                        priv_.content.set_child(Some(&child));
                    }
                    MessageType::ServerNotice(message) => {
                        let child = MessageText::text(message.body);
                        priv_.content.set_child(Some(&child));
                    }
                    MessageType::Text(message) => {
                        let child = MessageText::markup(message.formatted, message.body);
                        priv_.content.set_child(Some(&child));
                    }
                    MessageType::Video(_message) => {}
                    MessageType::VerificationRequest(_message) => {}
                    _ => {
                        warn!("Event not supported: {:?}", msgtype)
                    }
                }
            }
            Some(AnyMessageEventContent::RoomEncrypted(content)) => {
                warn!("Couldn't decrypt event {:?}", content);
                let child = MessageText::text(gettext("Fractal couldn't decrypt this message."));
                priv_.content.set_child(Some(&child));
            }
            Some(AnyMessageEventContent::RoomRedaction(_)) => {
                let child = MessageText::text(gettext("This message was removed."));
                priv_.content.set_child(Some(&child));
            }
            _ => {
                let child = MessageText::text(gettext("Unsupported event"));
                priv_.content.set_child(Some(&child));
            }
        }
    }
}

impl Default for MessageRow {
    fn default() -> Self {
        Self::new()
    }
}

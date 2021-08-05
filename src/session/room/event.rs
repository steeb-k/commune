use gtk::{glib, glib::DateTime, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    deserialized_responses::SyncRoomEvent,
    ruma::{
        events::{
            room::message::MessageType, room::message::Relation, AnyMessageEventContent,
            AnyRedactedSyncMessageEvent, AnyRedactedSyncStateEvent, AnySyncMessageEvent,
            AnySyncRoomEvent, AnySyncStateEvent,
        },
        identifiers::{EventId, UserId},
        MilliSecondsSinceUnixEpoch,
    },
};

use crate::session::{Room, User};
use log::warn;

#[derive(Clone, Debug, glib::GBoxed)]
#[gboxed(type_name = "BoxedSyncRoomEvent")]
pub struct BoxedSyncRoomEvent(SyncRoomEvent);

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use once_cell::sync::{Lazy, OnceCell};
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default)]
    pub struct Event {
        /// The deserialized matrix event
        pub event: RefCell<Option<AnySyncRoomEvent>>,
        /// The SDK event containing encryption information and the serialized event as `Raw`
        pub pure_event: RefCell<Option<SyncRoomEvent>>,
        pub relates_to: RefCell<Vec<super::Event>>,
        pub show_header: Cell<bool>,
        pub room: OnceCell<Room>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Event {
        const NAME: &'static str = "RoomEvent";
        type Type = super::Event;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for Event {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder("relates-to-changed", &[], <()>::static_type().into()).build()]
            });
            SIGNALS.as_ref()
        }

        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_boxed(
                        "event",
                        "event",
                        "The matrix event of this Event",
                        BoxedSyncRoomEvent::static_type(),
                        glib::ParamFlags::WRITABLE,
                    ),
                    glib::ParamSpec::new_string(
                        "source",
                        "Source",
                        "The source (JSON) of this Event",
                        None,
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_boolean(
                        "show-header",
                        "Show Header",
                        "Whether this event should show a header. This does nothing if this event doesn’t have a header. ",
                        false,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_boolean(
                        "can-hide-header",
                        "Can hide header",
                        "Whether this event is allowed to hide it's header or not.",
                        false,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "sender",
                        "Sender",
                        "The sender of this matrix event",
                        User::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "room",
                        "Room",
                        "The room containing this event",
                        Room::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "time",
                        "Time",
                        "The locally formatted time of this matrix event",
                        None,
                        glib::ParamFlags::READABLE,
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
                "event" => {
                    let event = value.get::<BoxedSyncRoomEvent>().unwrap();
                    obj.set_matrix_pure_event(event.0);
                }
                "show-header" => {
                    let show_header = value.get().unwrap();
                    let _ = obj.set_show_header(show_header);
                }
                "room" => {
                    let _ = self.room.set(value.get().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "source" => obj.source().to_value(),
                "sender" => obj.sender().to_value(),
                "room" => self.room.get().unwrap().to_value(),
                "show-header" => obj.show_header().to_value(),
                "can-hide-header" => obj.can_hide_header().to_value(),
                "time" => obj.time().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    /// GObject representation of a Matrix room event.
    pub struct Event(ObjectSubclass<imp::Event>);
}

// TODO:
// - [ ] implement operations for events: forward, reply, delete...

impl Event {
    pub fn new(event: SyncRoomEvent, room: &Room) -> Self {
        let event = BoxedSyncRoomEvent(event);
        glib::Object::new(&[("event", &event), ("room", room)]).expect("Failed to create Event")
    }

    pub fn sender(&self) -> User {
        let priv_ = imp::Event::from_instance(&self);
        priv_
            .room
            .get()
            .unwrap()
            .member_by_id(&self.matrix_sender())
    }

    /// Get the matrix event
    ///
    /// If the `SyncRoomEvent` couldn't be deserialized this is `None`
    pub fn matrix_event(&self) -> Option<AnySyncRoomEvent> {
        let priv_ = imp::Event::from_instance(&self);
        priv_.event.borrow().clone()
    }

    pub fn matrix_pure_event(&self) -> SyncRoomEvent {
        let priv_ = imp::Event::from_instance(&self);
        priv_.pure_event.borrow().clone().unwrap()
    }

    pub fn set_matrix_pure_event(&self, event: SyncRoomEvent) {
        let priv_ = imp::Event::from_instance(&self);

        if let Ok(deserialized) = event.event.deserialize() {
            priv_.event.replace(Some(deserialized));
        } else {
            warn!("Failed to deserialize event: {:?}", event);
        }

        priv_.pure_event.replace(Some(event));

        self.notify("event");
    }

    pub fn matrix_sender(&self) -> UserId {
        let priv_ = imp::Event::from_instance(&self);

        if let Some(event) = priv_.event.borrow().as_ref() {
            event.sender().to_owned()
        } else {
            priv_
                .pure_event
                .borrow()
                .as_ref()
                .unwrap()
                .event
                .get_field::<UserId>("sender")
                .unwrap()
                .unwrap()
        }
    }

    pub fn matrix_event_id(&self) -> EventId {
        let priv_ = imp::Event::from_instance(&self);

        if let Some(event) = priv_.event.borrow().as_ref() {
            event.event_id().to_owned()
        } else {
            priv_
                .pure_event
                .borrow()
                .as_ref()
                .unwrap()
                .event
                .get_field::<EventId>("event_id")
                .unwrap()
                .unwrap()
        }
    }

    pub fn source(&self) -> String {
        let priv_ = imp::Event::from_instance(&self);
        serde_json::to_string_pretty(priv_.pure_event.borrow().as_ref().unwrap().event.json())
            .unwrap()
    }

    pub fn timestamp(&self) -> DateTime {
        let priv_ = imp::Event::from_instance(&self);

        let ts = if let Some(event) = priv_.event.borrow().as_ref() {
            event.origin_server_ts().as_secs()
        } else {
            priv_
                .pure_event
                .borrow()
                .as_ref()
                .unwrap()
                .event
                .get_field::<MilliSecondsSinceUnixEpoch>("origin_server_ts")
                .unwrap()
                .unwrap()
                .as_secs()
        };

        DateTime::from_unix_utc(ts.into())
            .and_then(|t| t.to_local())
            .unwrap()
    }

    pub fn time(&self) -> String {
        let datetime = self.timestamp();

        // FIXME Is there a cleaner way to do that?
        let local_time = datetime.format("%X").unwrap().as_str().to_ascii_lowercase();

        if local_time.ends_with("am") || local_time.ends_with("pm") {
            // Use 12h time format (AM/PM)
            datetime.format("%l∶%M %p").unwrap().to_string()
        } else {
            // Use 24 time format
            datetime.format("%R").unwrap().to_string()
        }
    }

    /// Find the related event if any
    pub fn related_matrix_event(&self) -> Option<EventId> {
        let priv_ = imp::Event::from_instance(&self);

        match priv_.event.borrow().as_ref()? {
            AnySyncRoomEvent::Message(ref message) => match message {
                AnySyncMessageEvent::RoomRedaction(event) => Some(event.redacts.clone()),
                _ => match message.content() {
                    AnyMessageEventContent::Reaction(event) => Some(event.relates_to.event_id),
                    AnyMessageEventContent::RoomMessage(event) => match event.relates_to {
                        Some(relates_to) => match relates_to {
                            // TODO: Figure out Relation::Annotation(), Relation::Reference() but they are pre-specs for now
                            // See: https://github.com/uhoreg/matrix-doc/blob/aggregations-reactions/proposals/2677-reactions.md
                            Relation::Reply { in_reply_to } => Some(in_reply_to.event_id),
                            Relation::Replacement(replacement) => Some(replacement.event_id),
                            _ => None,
                        },
                        _ => None,
                    },
                    // TODO: RoomEncrypted needs https://github.com/ruma/ruma/issues/502
                    _ => None,
                },
            },
            _ => None,
        }
    }

    /// Whether this event is hidden from the user or displayed in the room history.
    pub fn is_hidden_event(&self) -> bool {
        let priv_ = imp::Event::from_instance(&self);

        if self.related_matrix_event().is_some() {
            return true;
        }

        if let Some(event) = priv_.event.borrow().as_ref() {
            match event {
                AnySyncRoomEvent::Message(message) => match message {
                    AnySyncMessageEvent::CallAnswer(_) => true,
                    AnySyncMessageEvent::CallInvite(_) => true,
                    AnySyncMessageEvent::CallHangup(_) => true,
                    AnySyncMessageEvent::CallCandidates(_) => true,
                    AnySyncMessageEvent::KeyVerificationReady(_) => true,
                    AnySyncMessageEvent::KeyVerificationStart(_) => true,
                    AnySyncMessageEvent::KeyVerificationCancel(_) => true,
                    AnySyncMessageEvent::KeyVerificationAccept(_) => true,
                    AnySyncMessageEvent::KeyVerificationKey(_) => true,
                    AnySyncMessageEvent::KeyVerificationMac(_) => true,
                    AnySyncMessageEvent::KeyVerificationDone(_) => true,
                    AnySyncMessageEvent::RoomEncrypted(_) => true,
                    AnySyncMessageEvent::RoomMessageFeedback(_) => true,
                    AnySyncMessageEvent::RoomRedaction(_) => true,
                    AnySyncMessageEvent::Sticker(_) => true,
                    _ => false,
                },
                AnySyncRoomEvent::State(state) => match state {
                    AnySyncStateEvent::PolicyRuleRoom(_) => true,
                    AnySyncStateEvent::PolicyRuleServer(_) => true,
                    AnySyncStateEvent::PolicyRuleUser(_) => true,
                    AnySyncStateEvent::RoomAliases(_) => true,
                    AnySyncStateEvent::RoomAvatar(_) => true,
                    AnySyncStateEvent::RoomCanonicalAlias(_) => true,
                    AnySyncStateEvent::RoomEncryption(_) => true,
                    AnySyncStateEvent::RoomJoinRules(_) => true,
                    AnySyncStateEvent::RoomName(_) => true,
                    AnySyncStateEvent::RoomPinnedEvents(_) => true,
                    AnySyncStateEvent::RoomPowerLevels(_) => true,
                    AnySyncStateEvent::RoomServerAcl(_) => true,
                    AnySyncStateEvent::RoomTopic(_) => true,
                    _ => false,
                },
                AnySyncRoomEvent::RedactedMessage(message) => match message {
                    AnyRedactedSyncMessageEvent::CallAnswer(_) => true,
                    AnyRedactedSyncMessageEvent::CallInvite(_) => true,
                    AnyRedactedSyncMessageEvent::CallHangup(_) => true,
                    AnyRedactedSyncMessageEvent::CallCandidates(_) => true,
                    AnyRedactedSyncMessageEvent::KeyVerificationReady(_) => true,
                    AnyRedactedSyncMessageEvent::KeyVerificationStart(_) => true,
                    AnyRedactedSyncMessageEvent::KeyVerificationCancel(_) => true,
                    AnyRedactedSyncMessageEvent::KeyVerificationAccept(_) => true,
                    AnyRedactedSyncMessageEvent::KeyVerificationKey(_) => true,
                    AnyRedactedSyncMessageEvent::KeyVerificationMac(_) => true,
                    AnyRedactedSyncMessageEvent::KeyVerificationDone(_) => true,
                    AnyRedactedSyncMessageEvent::RoomEncrypted(_) => true,
                    AnyRedactedSyncMessageEvent::RoomMessageFeedback(_) => true,
                    AnyRedactedSyncMessageEvent::RoomRedaction(_) => true,
                    AnyRedactedSyncMessageEvent::Sticker(_) => true,
                    _ => false,
                },
                AnySyncRoomEvent::RedactedState(state) => match state {
                    AnyRedactedSyncStateEvent::PolicyRuleRoom(_) => true,
                    AnyRedactedSyncStateEvent::PolicyRuleServer(_) => true,
                    AnyRedactedSyncStateEvent::PolicyRuleUser(_) => true,
                    AnyRedactedSyncStateEvent::RoomAliases(_) => true,
                    AnyRedactedSyncStateEvent::RoomAvatar(_) => true,
                    AnyRedactedSyncStateEvent::RoomCanonicalAlias(_) => true,
                    AnyRedactedSyncStateEvent::RoomEncryption(_) => true,
                    AnyRedactedSyncStateEvent::RoomJoinRules(_) => true,
                    AnyRedactedSyncStateEvent::RoomName(_) => true,
                    AnyRedactedSyncStateEvent::RoomPinnedEvents(_) => true,
                    AnyRedactedSyncStateEvent::RoomPowerLevels(_) => true,
                    AnyRedactedSyncStateEvent::RoomServerAcl(_) => true,
                    AnyRedactedSyncStateEvent::RoomTopic(_) => true,
                    _ => false,
                },
            }
        } else {
            false
        }
    }

    pub fn set_show_header(&self, visible: bool) {
        let priv_ = imp::Event::from_instance(&self);
        if priv_.show_header.get() == visible {
            return;
        }
        priv_.show_header.set(visible);
        self.notify("show-header");
    }

    pub fn show_header(&self) -> bool {
        let priv_ = imp::Event::from_instance(&self);

        priv_.show_header.get()
    }

    pub fn can_hide_header(&self) -> bool {
        if let Some(event) = self.matrix_event() {
            match event {
                AnySyncRoomEvent::Message(ref message) => match message.content() {
                    AnyMessageEventContent::RoomMessage(message) => match message.msgtype {
                        MessageType::Audio(_) => true,
                        MessageType::File(_) => true,
                        MessageType::Image(_) => true,
                        MessageType::Location(_) => true,
                        MessageType::Notice(_) => true,
                        MessageType::Text(_) => true,
                        MessageType::Video(_) => true,
                        _ => false,
                    },
                    _ => false,
                },
                _ => false,
            }
        } else {
            false
        }
    }

    pub fn add_relates_to(&self, events: Vec<Event>) {
        let priv_ = imp::Event::from_instance(&self);
        priv_.relates_to.borrow_mut().extend(events);
        self.emit_by_name("relates-to-changed", &[]).unwrap();
    }

    pub fn relates_to(&self) -> Vec<Event> {
        let priv_ = imp::Event::from_instance(&self);
        priv_.relates_to.borrow().clone()
    }

    pub fn connect_relates_to_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("relates-to-changed", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();

            f(&obj);

            None
        })
        .unwrap()
    }

    pub fn connect_show_header_notify<F: Fn(&Self, &glib::ParamSpec) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_notify_local(Some("show-header"), f)
    }
}

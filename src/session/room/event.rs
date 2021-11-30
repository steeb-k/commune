use gtk::{glib, glib::DateTime, prelude::*, subclass::prelude::*};
use log::warn;
use matrix_sdk::{
    deserialized_responses::SyncRoomEvent,
    ruma::{
        events::{
            room::message::MessageType, room::message::Relation, AnyMessageEventContent,
            AnyRedactedSyncMessageEvent, AnyRedactedSyncStateEvent, AnySyncMessageEvent,
            AnySyncRoomEvent, AnySyncStateEvent, Unsigned,
        },
        identifiers::{EventId, UserId},
        MilliSecondsSinceUnixEpoch,
    },
};

use crate::{
    session::{room::Member, Room},
    spawn_tokio,
};

#[derive(Clone, Debug, glib::GBoxed)]
#[gboxed(type_name = "BoxedSyncRoomEvent")]
pub struct BoxedSyncRoomEvent(SyncRoomEvent);

mod imp {
    use super::*;
    use glib::object::WeakRef;
    use glib::subclass::Signal;
    use once_cell::{sync::Lazy, unsync::OnceCell};
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default)]
    pub struct Event {
        /// The deserialized matrix event
        pub event: RefCell<Option<AnySyncRoomEvent>>,
        /// The SDK event containing encryption information and the serialized event as `Raw`
        pub pure_event: RefCell<Option<SyncRoomEvent>>,
        pub relates_to: RefCell<Vec<super::Event>>,
        pub show_header: Cell<bool>,
        pub room: OnceCell<WeakRef<Room>>,
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
                        Member::static_type(),
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
                    glib::ParamSpec::new_boolean(
                        "can-view-media",
                        "Can View Media",
                        "Whether this is a media event that can be viewed",
                        false,
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
                    self.room
                        .set(value.get::<Room>().unwrap().downgrade())
                        .unwrap();
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "source" => obj.source().to_value(),
                "sender" => obj.sender().to_value(),
                "room" => obj.room().to_value(),
                "show-header" => obj.show_header().to_value(),
                "can-hide-header" => obj.can_hide_header().to_value(),
                "time" => obj.time().to_value(),
                "can-view-media" => obj.can_view_media().to_value(),
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

    pub fn sender(&self) -> Member {
        self.room().members().member_by_id(&self.matrix_sender())
    }

    pub fn room(&self) -> Room {
        let priv_ = imp::Event::from_instance(self);
        priv_.room.get().unwrap().upgrade().unwrap()
    }

    /// Get the matrix event
    ///
    /// If the `SyncRoomEvent` couldn't be deserialized this is `None`
    pub fn matrix_event(&self) -> Option<AnySyncRoomEvent> {
        let priv_ = imp::Event::from_instance(self);
        priv_.event.borrow().clone()
    }

    pub fn matrix_pure_event(&self) -> SyncRoomEvent {
        let priv_ = imp::Event::from_instance(self);
        priv_.pure_event.borrow().clone().unwrap()
    }

    pub fn set_matrix_pure_event(&self, event: SyncRoomEvent) {
        let priv_ = imp::Event::from_instance(self);

        if let Ok(deserialized) = event.event.deserialize() {
            priv_.event.replace(Some(deserialized));
        } else {
            warn!("Failed to deserialize event: {:?}", event);
        }

        priv_.pure_event.replace(Some(event));

        self.notify("event");
        self.notify("can-view-media");
    }

    pub fn matrix_sender(&self) -> UserId {
        let priv_ = imp::Event::from_instance(self);

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
        let priv_ = imp::Event::from_instance(self);

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

    pub fn matrix_transaction_id(&self) -> Option<String> {
        let priv_ = imp::Event::from_instance(self);

        priv_
            .pure_event
            .borrow()
            .as_ref()
            .unwrap()
            .event
            .get_field::<Unsigned>("unsigned")
            .ok()
            .and_then(|opt| opt)
            .and_then(|unsigned| unsigned.transaction_id)
    }

    pub fn source(&self) -> String {
        let priv_ = imp::Event::from_instance(self);

        // We have to convert it to a Value, because a RawValue cannot be pretty-printed.
        let json: serde_json::Value = serde_json::from_str(
            priv_
                .pure_event
                .borrow()
                .as_ref()
                .unwrap()
                .event
                .json()
                .get(),
        )
        .unwrap();

        serde_json::to_string_pretty(&json).unwrap()
    }

    pub fn timestamp(&self) -> DateTime {
        let priv_ = imp::Event::from_instance(self);

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
        let priv_ = imp::Event::from_instance(self);

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
        let priv_ = imp::Event::from_instance(self);

        if self.related_matrix_event().is_some() {
            if let Some(AnySyncRoomEvent::Message(message)) = priv_.event.borrow().as_ref() {
                if let AnyMessageEventContent::RoomMessage(content) = message.content() {
                    if let Some(Relation::Reply { in_reply_to: _ }) = content.relates_to {
                        return false;
                    }
                }
            }
            return true;
        }

        let event = priv_.event.borrow();

        // List of all events to be hidden.
        match event.as_ref() {
            Some(AnySyncRoomEvent::Message(message)) => matches!(
                message,
                AnySyncMessageEvent::CallAnswer(_)
                    | AnySyncMessageEvent::CallInvite(_)
                    | AnySyncMessageEvent::CallHangup(_)
                    | AnySyncMessageEvent::CallCandidates(_)
                    | AnySyncMessageEvent::KeyVerificationReady(_)
                    | AnySyncMessageEvent::KeyVerificationStart(_)
                    | AnySyncMessageEvent::KeyVerificationCancel(_)
                    | AnySyncMessageEvent::KeyVerificationAccept(_)
                    | AnySyncMessageEvent::KeyVerificationKey(_)
                    | AnySyncMessageEvent::KeyVerificationMac(_)
                    | AnySyncMessageEvent::KeyVerificationDone(_)
                    | AnySyncMessageEvent::RoomMessageFeedback(_)
                    | AnySyncMessageEvent::RoomRedaction(_)
            ),
            Some(AnySyncRoomEvent::State(state)) => matches!(
                state,
                AnySyncStateEvent::PolicyRuleRoom(_)
                    | AnySyncStateEvent::PolicyRuleServer(_)
                    | AnySyncStateEvent::PolicyRuleUser(_)
                    | AnySyncStateEvent::RoomAliases(_)
                    | AnySyncStateEvent::RoomAvatar(_)
                    | AnySyncStateEvent::RoomCanonicalAlias(_)
                    | AnySyncStateEvent::RoomEncryption(_)
                    | AnySyncStateEvent::RoomJoinRules(_)
                    | AnySyncStateEvent::RoomName(_)
                    | AnySyncStateEvent::RoomPinnedEvents(_)
                    | AnySyncStateEvent::RoomPowerLevels(_)
                    | AnySyncStateEvent::RoomServerAcl(_)
                    | AnySyncStateEvent::RoomTopic(_)
            ),
            Some(AnySyncRoomEvent::RedactedMessage(message)) => matches!(
                message,
                AnyRedactedSyncMessageEvent::CallAnswer(_)
                    | AnyRedactedSyncMessageEvent::CallInvite(_)
                    | AnyRedactedSyncMessageEvent::CallHangup(_)
                    | AnyRedactedSyncMessageEvent::CallCandidates(_)
                    | AnyRedactedSyncMessageEvent::KeyVerificationReady(_)
                    | AnyRedactedSyncMessageEvent::KeyVerificationStart(_)
                    | AnyRedactedSyncMessageEvent::KeyVerificationCancel(_)
                    | AnyRedactedSyncMessageEvent::KeyVerificationAccept(_)
                    | AnyRedactedSyncMessageEvent::KeyVerificationKey(_)
                    | AnyRedactedSyncMessageEvent::KeyVerificationMac(_)
                    | AnyRedactedSyncMessageEvent::KeyVerificationDone(_)
                    | AnyRedactedSyncMessageEvent::RoomMessageFeedback(_)
                    | AnyRedactedSyncMessageEvent::RoomRedaction(_)
                    | AnyRedactedSyncMessageEvent::Sticker(_)
            ),
            Some(AnySyncRoomEvent::RedactedState(state)) => matches!(
                state,
                AnyRedactedSyncStateEvent::PolicyRuleRoom(_)
                    | AnyRedactedSyncStateEvent::PolicyRuleServer(_)
                    | AnyRedactedSyncStateEvent::PolicyRuleUser(_)
                    | AnyRedactedSyncStateEvent::RoomAliases(_)
                    | AnyRedactedSyncStateEvent::RoomAvatar(_)
                    | AnyRedactedSyncStateEvent::RoomCanonicalAlias(_)
                    | AnyRedactedSyncStateEvent::RoomEncryption(_)
                    | AnyRedactedSyncStateEvent::RoomJoinRules(_)
                    | AnyRedactedSyncStateEvent::RoomName(_)
                    | AnyRedactedSyncStateEvent::RoomPinnedEvents(_)
                    | AnyRedactedSyncStateEvent::RoomPowerLevels(_)
                    | AnyRedactedSyncStateEvent::RoomServerAcl(_)
                    | AnyRedactedSyncStateEvent::RoomTopic(_)
            ),
            _ => false,
        }
    }

    pub fn set_show_header(&self, visible: bool) {
        let priv_ = imp::Event::from_instance(self);
        if priv_.show_header.get() == visible {
            return;
        }
        priv_.show_header.set(visible);
        self.notify("show-header");
    }

    pub fn show_header(&self) -> bool {
        let priv_ = imp::Event::from_instance(self);

        priv_.show_header.get()
    }

    /// The content of this message.
    ///
    /// Returns `None` if this is not a message.
    pub fn message_content(&self) -> Option<AnyMessageEventContent> {
        match self.matrix_event() {
            Some(AnySyncRoomEvent::Message(message)) => Some(message.content()),
            _ => None,
        }
    }

    pub fn can_hide_header(&self) -> bool {
        match self.message_content() {
            Some(AnyMessageEventContent::RoomMessage(message)) => {
                matches!(
                    message.msgtype,
                    MessageType::Audio(_)
                        | MessageType::File(_)
                        | MessageType::Image(_)
                        | MessageType::Location(_)
                        | MessageType::Notice(_)
                        | MessageType::Text(_)
                        | MessageType::Video(_)
                )
            }
            Some(AnyMessageEventContent::Sticker(_)) => true,
            _ => false,
        }
    }

    pub fn add_relates_to(&self, events: Vec<Event>) {
        let priv_ = imp::Event::from_instance(self);
        priv_.relates_to.borrow_mut().extend(events);
        self.emit_by_name("relates-to-changed", &[]).unwrap();
    }

    pub fn relates_to(&self) -> Vec<Event> {
        let priv_ = imp::Event::from_instance(self);
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

    /// The content of a media message.
    ///
    /// Compatible events:
    ///
    /// - File message (`MessageType::File`).
    /// - Image message (`MessageType::Image`).
    ///
    /// Returns `Ok((filename, binary_content))` on success, `Err` if an error occured while
    /// fetching the content. Panics on an incompatible event.
    pub async fn get_media_content(&self) -> Result<(String, Vec<u8>), matrix_sdk::Error> {
        if let AnyMessageEventContent::RoomMessage(content) = self.message_content().unwrap() {
            let client = self.room().session().client();
            match content.msgtype {
                MessageType::File(content) => {
                    let filename = content.filename.clone().unwrap_or(content.body.clone());
                    let handle = spawn_tokio!(async move { client.get_file(content, true).await });
                    let data = handle.await.unwrap()?.unwrap();
                    return Ok((filename, data));
                }
                MessageType::Image(content) => {
                    let filename = content.body.clone();
                    let handle = spawn_tokio!(async move { client.get_file(content, true).await });
                    let data = handle.await.unwrap()?.unwrap();
                    return Ok((filename, data));
                }
                _ => {}
            };
        };

        panic!("Trying to get the media content of an event of incompatible type");
    }

    /// Whether this is a media event that can be viewed.
    pub fn can_view_media(&self) -> bool {
        match self.message_content() {
            Some(AnyMessageEventContent::RoomMessage(message)) => {
                matches!(message.msgtype, MessageType::Image(_))
            }
            _ => false,
        }
    }
}

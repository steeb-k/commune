use std::ops::Deref;

use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::deserialized_responses::TimelineEvent;
use ruma::{
    events::{
        room::message::{MessageType, OriginalSyncRoomMessageEvent, Relation},
        AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
    },
    OwnedEventId,
};

use crate::{
    session::model::Room,
    utils::matrix::{MediaMessage, VisualMediaMessage},
};

/// The types of events that can be displayer in the history viewers.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "HistoryViewerEventType")]
pub enum HistoryViewerEventType {
    #[default]
    File,
    Media,
    Audio,
}

impl HistoryViewerEventType {
    fn with_msgtype(msgtype: &MessageType) -> Option<Self> {
        let event_type = match msgtype {
            MessageType::Audio(_) => Self::Audio,
            MessageType::File(_) => Self::File,
            MessageType::Image(_) | MessageType::Video(_) => Self::Media,
            _ => return None,
        };

        Some(event_type)
    }
}

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "BoxedSyncRoomMessageEvent")]
pub struct BoxedSyncRoomMessageEvent(pub OriginalSyncRoomMessageEvent);

impl Deref for BoxedSyncRoomMessageEvent {
    type Target = OriginalSyncRoomMessageEvent;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

mod imp {
    use std::cell::{Cell, OnceCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::HistoryViewerEvent)]
    pub struct HistoryViewerEvent {
        /// The room containing this event.
        #[property(get, construct_only)]
        pub room: glib::WeakRef<Room>,
        /// The Matrix event.
        #[property(construct_only)]
        pub matrix_event: OnceCell<BoxedSyncRoomMessageEvent>,
        /// The type of the event.
        #[property(get, construct_only, builder(HistoryViewerEventType::default()))]
        pub event_type: Cell<HistoryViewerEventType>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HistoryViewerEvent {
        const NAME: &'static str = "HistoryViewerEvent";
        type Type = super::HistoryViewerEvent;
    }

    #[glib::derived_properties]
    impl ObjectImpl for HistoryViewerEvent {}
}

glib::wrapper! {
    /// An event in the history viewer's timeline.
    pub struct HistoryViewerEvent(ObjectSubclass<imp::HistoryViewerEvent>);
}

impl HistoryViewerEvent {
    /// Constructs a new `HistoryViewerEvent` with the given event, if it is
    /// viewable in one of the history viewers.
    pub fn try_new(room: &Room, event: &TimelineEvent) -> Option<Self> {
        let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncMessageLikeEvent::Original(mut message_event),
        ))) = event.raw().deserialize()
        else {
            return None;
        };

        // Filter out edits, they should be bundled with the original event.
        if matches!(
            message_event.content.relates_to,
            Some(Relation::Replacement(_))
        ) {
            return None;
        }

        // Apply bundled edit.
        if let Some(Relation::Replacement(replacement)) = message_event
            .unsigned
            .relations
            .replace
            .as_ref()
            .and_then(|e| e.content.relates_to.as_ref())
        {
            message_event
                .content
                .apply_replacement(replacement.new_content.clone());
        }

        let event_type = HistoryViewerEventType::with_msgtype(&message_event.content.msgtype)?;

        let obj: Self = glib::Object::builder()
            .property("room", room)
            .property("matrix-event", BoxedSyncRoomMessageEvent(message_event))
            .property("event-type", event_type)
            .build();

        Some(obj)
    }

    /// The Matrix event.
    fn matrix_event(&self) -> &OriginalSyncRoomMessageEvent {
        self.imp().matrix_event.get().unwrap()
    }

    /// The event ID of the inner event.
    pub fn event_id(&self) -> OwnedEventId {
        self.matrix_event().event_id.clone()
    }

    /// The media message content of this event.
    pub(crate) fn media_message(&self) -> MediaMessage {
        MediaMessage::from_message(&self.matrix_event().content.msgtype)
            .expect("HistoryViewerEvents are all media messages")
    }

    /// The visual media message of this event, if any.
    pub(crate) fn visual_media_message(&self) -> Option<VisualMediaMessage> {
        VisualMediaMessage::from_message(&self.matrix_event().content.msgtype)
    }

    /// Get the binary content of this event.
    pub async fn get_file_content(&self) -> Result<Vec<u8>, matrix_sdk::Error> {
        let Some(room) = self.room() else {
            return Err(matrix_sdk::Error::UnknownError(
                "Could not upgrade Room".into(),
            ));
        };
        let Some(session) = room.session() else {
            return Err(matrix_sdk::Error::UnknownError(
                "Could not upgrade Session".into(),
            ));
        };

        let client = session.client();
        self.media_message().into_content(&client).await
    }
}

use std::{borrow::Cow, fmt};

use gtk::{gio, glib, glib::closure_local, prelude::*, subclass::prelude::*};
use indexmap::IndexMap;
use matrix_sdk_ui::timeline::{
    AnyOtherFullStateEventContent, Error as TimelineError, EventSendState, EventTimelineItem,
    RepliedToEvent, TimelineDetails, TimelineItemContent,
};
use ruma::{
    events::{
        receipt::Receipt,
        room::message::{MessageType, OriginalSyncRoomMessageEvent},
        AnySyncTimelineEvent, Mentions,
    },
    serde::Raw,
    EventId, MatrixToUri, MatrixUri, MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedTransactionId,
    OwnedUserId,
};
use serde::Deserialize;
use tracing::error;

mod reaction_group;
mod reaction_list;

pub use self::{reaction_group::ReactionGroup, reaction_list::ReactionList};
use super::{
    timeline::{TimelineItem, TimelineItemImpl},
    Member, Room,
};
use crate::{
    prelude::*,
    spawn_tokio,
    utils::matrix::{raw_eq, MediaMessage},
};

/// The unique key to identify an event in a room.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventKey {
    /// This is the local echo of the event, the key is its transaction ID.
    TransactionId(OwnedTransactionId),

    /// This is the remote echo of the event, the key is its event ID.
    EventId(OwnedEventId),
}

impl fmt::Display for EventKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventKey::TransactionId(txn_id) => write!(f, "transaction_id:{txn_id}"),
            EventKey::EventId(event_id) => write!(f, "event_id:{event_id}"),
        }
    }
}

impl StaticVariantType for EventKey {
    fn static_variant_type() -> Cow<'static, glib::VariantTy> {
        Cow::Borrowed(glib::VariantTy::STRING)
    }
}

impl ToVariant for EventKey {
    fn to_variant(&self) -> glib::Variant {
        self.to_string().to_variant()
    }
}

impl FromVariant for EventKey {
    fn from_variant(variant: &glib::Variant) -> Option<Self> {
        let s = variant.str()?;

        if let Some(s) = s.strip_prefix("transaction_id:") {
            Some(EventKey::TransactionId(s.into()))
        } else if let Some(s) = s.strip_prefix("event_id:") {
            EventId::parse(s).ok().map(EventKey::EventId)
        } else {
            None
        }
    }
}

#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "MessageState")]
pub enum MessageState {
    /// The message has no particular state.
    #[default]
    None,
    /// The message is being sent.
    Sending,
    /// A transient error occurred when sending the message.
    ///
    /// The user can try to send it again.
    RecoverableError,
    /// A permanent error occurred when sending the message.
    ///
    /// The message can only be cancelled.
    PermanentError,
    /// The message was edited.
    Edited,
}

/// A user's read receipt.
#[derive(Clone, Debug)]
pub struct UserReadReceipt {
    pub user_id: OwnedUserId,
    pub receipt: Receipt,
}

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::Signal;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::Event)]
    pub struct Event {
        /// The underlying SDK timeline item.
        pub item: RefCell<Option<EventTimelineItem>>,
        /// The room containing this `Event`.
        #[property(get, set = Self::set_room, construct_only)]
        pub room: OnceCell<Room>,
        /// The reactions on this event.
        #[property(get)]
        pub reactions: ReactionList,
        /// The read receipts on this event.
        #[property(get)]
        pub read_receipts: gio::ListStore,
        /// The state of this event.
        #[property(get, builder(MessageState::default()))]
        pub state: Cell<MessageState>,
        /// The pretty-formatted JSON source for this `Event`, if it has
        /// been echoed back by the server.
        #[property(get = Self::source)]
        pub source: PhantomData<Option<String>>,
        /// Whether we have the JSON source of this event.
        #[property(get = Self::has_source)]
        pub has_source: PhantomData<bool>,
        /// The event ID of this `Event`, if it has been received from the
        /// server, as a string.
        #[property(get = Self::event_id_string)]
        pub event_id_string: PhantomData<Option<String>>,
        /// The ID of the sender of this `Event`, as a string.
        #[property(get = Self::sender_id_string)]
        pub sender_id_string: PhantomData<String>,
        /// The timestamp of this `Event`.
        #[property(get = Self::timestamp)]
        pub timestamp: PhantomData<glib::DateTime>,
        /// The full formatted timestamp of this `Event`.
        #[property(get = Self::timestamp_full)]
        pub timestamp_full: PhantomData<String>,
        /// Whether this `Event` was edited.
        #[property(get = Self::is_edited)]
        pub is_edited: PhantomData<bool>,
        /// The pretty-formatted JSON source for the latest edit of this
        /// `Event`, if any.
        #[property(get = Self::latest_edit_source)]
        pub latest_edit_source: PhantomData<String>,
        /// The ID for the latest edit of this `Event`.
        #[property(get = Self::latest_edit_event_id_string)]
        pub latest_edit_event_id_string: PhantomData<String>,
        /// The timestamp for the latest edit of this `Event`, if any.
        #[property(get = Self::latest_edit_timestamp)]
        pub latest_edit_timestamp: PhantomData<Option<glib::DateTime>>,
        /// The full formatted timestamp for the latest edit of this `Event`.
        #[property(get = Self::latest_edit_timestamp_full)]
        pub latest_edit_timestamp_full: PhantomData<String>,
        /// Whether this `Event` should be highlighted.
        #[property(get = Self::is_highlighted)]
        pub is_highlighted: PhantomData<bool>,
        /// Whether this event has any read receipt.
        #[property(get = Self::has_read_receipts)]
        pub has_read_receipts: PhantomData<bool>,
    }

    impl Default for Event {
        fn default() -> Self {
            Self {
                item: Default::default(),
                room: Default::default(),
                reactions: Default::default(),
                read_receipts: gio::ListStore::new::<glib::BoxedAnyObject>(),
                state: Default::default(),
                source: Default::default(),
                has_source: Default::default(),
                event_id_string: Default::default(),
                sender_id_string: Default::default(),
                timestamp: Default::default(),
                timestamp_full: Default::default(),
                is_edited: Default::default(),
                latest_edit_source: Default::default(),
                latest_edit_event_id_string: Default::default(),
                latest_edit_timestamp: Default::default(),
                latest_edit_timestamp_full: Default::default(),
                is_highlighted: Default::default(),
                has_read_receipts: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Event {
        const NAME: &'static str = "RoomEvent";
        type Type = super::Event;
        type ParentType = TimelineItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Event {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("item-changed").build()]);
            SIGNALS.as_ref()
        }
    }

    impl TimelineItemImpl for Event {
        fn id(&self) -> String {
            format!("Event::{}", self.obj().key())
        }

        fn can_hide_header(&self) -> bool {
            content_can_show_header(&self.obj().content())
        }

        fn event_sender_id(&self) -> Option<OwnedUserId> {
            Some(self.obj().sender_id())
        }

        fn selectable(&self) -> bool {
            true
        }
    }

    impl Event {
        /// Set the underlying SDK timeline item of this `Event`.
        pub fn set_item(&self, item: EventTimelineItem) {
            let obj = self.obj();

            let prev_raw = self.raw();
            let prev_event_id = self.event_id_string();
            let was_edited = self.is_edited();
            let was_highlighted = self.is_highlighted();
            let prev_latest_edit_raw = self.latest_edit_raw();
            let had_source = self.has_source();

            self.reactions.update(item.reactions().clone());
            obj.update_read_receipts(item.read_receipts());

            self.item.replace(Some(item));

            if !raw_eq(prev_raw.as_ref(), self.raw().as_ref()) {
                obj.notify_source();
            }
            if self.event_id_string() != prev_event_id {
                obj.notify_event_id_string();
            }
            if self.is_edited() != was_edited {
                obj.notify_is_edited();
            }
            if self.is_highlighted() != was_highlighted {
                obj.notify_is_highlighted();
            }
            if !raw_eq(
                prev_latest_edit_raw.as_ref(),
                self.latest_edit_raw().as_ref(),
            ) {
                obj.notify_latest_edit_source();
                obj.notify_latest_edit_event_id_string();
                obj.notify_latest_edit_timestamp();
                obj.notify_latest_edit_timestamp_full();
            }
            if self.has_source() != had_source {
                obj.notify_has_source();
            }

            obj.update_state();
            obj.emit_by_name::<()>("item-changed", &[]);
        }

        /// The raw JSON source for this `Event`, if it has been echoed back
        /// by the server.
        pub fn raw(&self) -> Option<Raw<AnySyncTimelineEvent>> {
            self.item.borrow().as_ref()?.original_json().cloned()
        }

        /// The pretty-formatted JSON source for this `Event`, if it has
        /// been echoed back by the server.
        fn source(&self) -> Option<String> {
            self.item
                .borrow()
                .as_ref()?
                .original_json()
                .map(raw_to_pretty_string)
        }

        /// Whether we have the JSON source of this event.
        fn has_source(&self) -> bool {
            self.item
                .borrow()
                .as_ref()
                .is_some_and(|i| i.original_json().is_some())
        }

        /// The event ID of this `Event`, if it has been received from the
        /// server, as a string.
        fn event_id_string(&self) -> Option<String> {
            self.item
                .borrow()
                .as_ref()?
                .event_id()
                .map(ToString::to_string)
        }

        /// The ID of the sender of this `Event`, as a string.
        fn sender_id_string(&self) -> String {
            self.item
                .borrow()
                .as_ref()
                .map(|i| i.sender().to_string())
                .unwrap_or_default()
        }

        /// Set the room that contains this `Event`.
        fn set_room(&self, room: Room) {
            self.room.set(room.clone()).unwrap();

            if let Some(session) = room.session() {
                self.reactions.set_user(session.user().clone());
            }
        }

        /// The timestamp of this `Event`.
        fn timestamp(&self) -> glib::DateTime {
            let ts = self.obj().origin_server_ts();

            glib::DateTime::from_unix_utc(ts.as_secs().into())
                .and_then(|t| t.to_local())
                .unwrap()
        }

        /// The full formatted timestamp of this `Event`.
        fn timestamp_full(&self) -> String {
            self.timestamp()
                .format("%c")
                .map(Into::into)
                .unwrap_or_default()
        }

        /// Whether this `Event` was edited.
        fn is_edited(&self) -> bool {
            let item_ref = self.item.borrow();
            let Some(item) = item_ref.as_ref() else {
                return false;
            };

            match item.content() {
                TimelineItemContent::Message(msg) => msg.is_edited(),
                _ => false,
            }
        }

        /// The JSON source for the latest edit of this `Event`, if any.
        fn latest_edit_raw(&self) -> Option<Raw<AnySyncTimelineEvent>> {
            let borrowed_item = self.item.borrow();
            let item = borrowed_item.as_ref()?;

            if let Some(raw) = item.latest_edit_json() {
                return Some(raw.clone());
            }

            item.original_json()?
                .get_field::<RawUnsigned>("unsigned")
                .ok()
                .flatten()?
                .relations?
                .replace
        }

        /// The pretty-formatted JSON source for the latest edit of this
        /// `Event`.
        fn latest_edit_source(&self) -> String {
            self.latest_edit_raw()
                .as_ref()
                .map(raw_to_pretty_string)
                .unwrap_or_default()
        }

        /// The ID of the latest edit of this `Event`.
        fn latest_edit_event_id_string(&self) -> String {
            self.latest_edit_raw()
                .as_ref()
                .and_then(|r| r.get_field::<String>("event_id").ok().flatten())
                .unwrap_or_default()
        }

        /// The timestamp of the latest edit of this `Event`, if any.
        fn latest_edit_timestamp(&self) -> Option<glib::DateTime> {
            self.latest_edit_raw()
                .as_ref()
                .and_then(|r| {
                    r.get_field::<MilliSecondsSinceUnixEpoch>("origin_server_ts")
                        .ok()
                        .flatten()
                })
                .map(|ts| {
                    glib::DateTime::from_unix_utc(ts.as_secs().into())
                        .and_then(|t| t.to_local())
                        .unwrap()
                })
        }

        /// The full formatted timestamp of the latest edit of this `Event`.
        fn latest_edit_timestamp_full(&self) -> String {
            self.latest_edit_timestamp()
                .and_then(|d| d.format("%c").ok())
                .map(Into::into)
                .unwrap_or_default()
        }

        /// Whether this `Event` should be highlighted.
        fn is_highlighted(&self) -> bool {
            let item_ref = self.item.borrow();
            let Some(item) = item_ref.as_ref() else {
                return false;
            };

            item.is_highlighted()
        }

        /// Whether this event has any read receipt.
        fn has_read_receipts(&self) -> bool {
            self.read_receipts.n_items() > 0
        }
    }
}

glib::wrapper! {
    /// GObject representation of a Matrix room event.
    pub struct Event(ObjectSubclass<imp::Event>) @extends TimelineItem;
}

impl Event {
    /// Create a new `Event` with the given SDK timeline item.
    pub fn new(item: EventTimelineItem, room: &Room) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .build();

        obj.imp().set_item(item);

        obj
    }

    /// Try to update this `Event` with the given SDK timeline item.
    ///
    /// Returns `true` if the update succeeded.
    pub fn try_update_with(&self, item: &EventTimelineItem) -> bool {
        match &self.key() {
            EventKey::TransactionId(txn_id)
                if item.is_local_echo() && item.transaction_id() == Some(txn_id) =>
            {
                self.imp().set_item(item.clone());
                return true;
            }
            EventKey::EventId(event_id)
                if !item.is_local_echo() && item.event_id() == Some(event_id) =>
            {
                self.imp().set_item(item.clone());
                return true;
            }
            _ => {}
        }

        false
    }

    /// The underlying SDK timeline item.
    pub fn item(&self) -> EventTimelineItem {
        self.imp().item.borrow().clone().unwrap()
    }

    /// The raw JSON source for this `Event`, if it has been echoed back
    /// by the server.
    pub fn raw(&self) -> Option<Raw<AnySyncTimelineEvent>> {
        self.imp().raw()
    }

    /// The unique key of this `Event` in the timeline.
    pub fn key(&self) -> EventKey {
        let item_ref = self.imp().item.borrow();
        let item = item_ref.as_ref().unwrap();
        if item.is_local_echo() {
            EventKey::TransactionId(item.transaction_id().unwrap().to_owned())
        } else {
            EventKey::EventId(item.event_id().unwrap().to_owned())
        }
    }

    /// Whether the given key matches this `Event`.
    ///
    /// The result can be different from comparing two `EventKey`s because an
    /// event can have a transaction ID and an event ID.
    pub fn matches_key(&self, key: &EventKey) -> bool {
        let item_ref = self.imp().item.borrow();
        let item = item_ref.as_ref().unwrap();
        match key {
            EventKey::TransactionId(txn_id) => item.transaction_id().is_some_and(|id| id == txn_id),
            EventKey::EventId(event_id) => item.event_id().is_some_and(|id| id == event_id),
        }
    }

    /// The event ID of this `Event`, if it has been received from the server.
    pub fn event_id(&self) -> Option<OwnedEventId> {
        match self.key() {
            EventKey::EventId(event_id) => Some(event_id),
            _ => None,
        }
    }

    /// The transaction ID of this `Event`, if it is still pending.
    pub fn transaction_id(&self) -> Option<OwnedTransactionId> {
        match self.key() {
            EventKey::TransactionId(txn_id) => Some(txn_id),
            _ => None,
        }
    }

    /// The user ID of the sender of this `Event`.
    pub fn sender_id(&self) -> OwnedUserId {
        self.imp()
            .item
            .borrow()
            .as_ref()
            .unwrap()
            .sender()
            .to_owned()
    }

    /// The sender of this `Event`.
    ///
    /// This should only be called when the event's room members list is
    /// available, otherwise it will be created on every call.
    pub fn sender(&self) -> Member {
        self.room()
            .get_or_create_members()
            .get_or_create(self.sender_id())
    }

    /// The timestamp of this `Event` as the number of milliseconds
    /// since Unix Epoch, if it has been echoed back by the server.
    ///
    /// Otherwise it's the local time when this event was created.
    pub fn origin_server_ts(&self) -> MilliSecondsSinceUnixEpoch {
        self.imp().item.borrow().as_ref().unwrap().timestamp()
    }

    /// The timestamp of this `Event` as a `u64`, if it has been echoed back by
    /// the server.
    ///
    /// Otherwise it's the local time when this event was created.
    pub fn origin_server_ts_u64(&self) -> u64 {
        self.origin_server_ts().get().into()
    }

    /// Whether this `Event` is redacted.
    pub fn is_redacted(&self) -> bool {
        matches!(
            self.imp().item.borrow().as_ref().unwrap().content(),
            TimelineItemContent::RedactedMessage
        )
    }

    /// The content to display for this `Event`.
    pub fn content(&self) -> TimelineItemContent {
        self.imp().item.borrow().as_ref().unwrap().content().clone()
    }

    /// The message of this `Event`, if any.
    pub fn message(&self) -> Option<MessageType> {
        match self.imp().item.borrow().as_ref().unwrap().content() {
            TimelineItemContent::Message(msg) => Some(msg.msgtype().clone()),
            _ => None,
        }
    }

    /// The media message of this `Event`, if any.
    pub fn media_message(&self) -> Option<MediaMessage> {
        match self.imp().item.borrow().as_ref().unwrap().content() {
            TimelineItemContent::Message(msg) => MediaMessage::from_message(msg.msgtype()),
            _ => None,
        }
    }

    /// The mentions from this message, if any.
    pub fn mentions(&self) -> Option<Mentions> {
        match self.imp().item.borrow().as_ref().unwrap().content() {
            TimelineItemContent::Message(msg) => msg.mentions().cloned(),
            _ => None,
        }
    }

    /// Whether this event might contain an `@room` mention.
    ///
    /// This means that either it doesn't have intentional mentions, or it has
    /// intentional mentions and `room` is set to `true`.
    pub fn can_contain_at_room(&self) -> bool {
        self.imp()
            .item
            .borrow()
            .as_ref()
            .unwrap()
            .content()
            .can_contain_at_room()
    }

    /// Compute the current state of this `Event`.
    fn compute_state(&self) -> MessageState {
        let item_ref = self.imp().item.borrow();
        let Some(item) = item_ref.as_ref() else {
            return MessageState::None;
        };

        if let Some(send_state) = item.send_state() {
            match send_state {
                EventSendState::NotSentYet => return MessageState::Sending,
                EventSendState::SendingFailed {
                    error,
                    is_recoverable,
                } => {
                    if !matches!(
                        self.state(),
                        MessageState::PermanentError | MessageState::RecoverableError,
                    ) {
                        error!("Could not send message: {error}");
                    }

                    let new_state = if *is_recoverable {
                        MessageState::RecoverableError
                    } else {
                        MessageState::PermanentError
                    };

                    return new_state;
                }
                EventSendState::Sent { .. } => {}
            }
        }

        match item.content() {
            TimelineItemContent::Message(msg) if msg.is_edited() => MessageState::Edited,
            _ => MessageState::None,
        }
    }

    /// Update the state of this `Event`.
    fn update_state(&self) {
        let state = self.compute_state();

        if self.state() == state {
            return;
        }

        self.imp().state.set(state);
        self.notify_state();
    }

    /// Update the read receipts list with the given receipts.
    fn update_read_receipts(&self, new_read_receipts: &IndexMap<OwnedUserId, Receipt>) {
        let read_receipts = &self.imp().read_receipts;
        let old_count = read_receipts.n_items();
        let new_count = new_read_receipts.len() as u32;

        if old_count == new_count {
            let mut is_all_same = true;
            for (i, new_user_id) in new_read_receipts.keys().enumerate() {
                let Some(old_receipt) = read_receipts
                    .item(i as u32)
                    .and_downcast::<glib::BoxedAnyObject>()
                else {
                    is_all_same = false;
                    break;
                };

                if old_receipt.borrow::<UserReadReceipt>().user_id != *new_user_id {
                    is_all_same = false;
                    break;
                }
            }

            if is_all_same {
                return;
            }
        }

        let new_read_receipts = new_read_receipts
            .into_iter()
            .map(|(user_id, receipt)| {
                glib::BoxedAnyObject::new(UserReadReceipt {
                    user_id: user_id.clone(),
                    receipt: receipt.clone(),
                })
            })
            .collect::<Vec<_>>();
        read_receipts.splice(0, old_count, &new_read_receipts);

        let had_read_receipts = old_count > 0;
        let has_read_receipts = new_count > 0;

        if had_read_receipts != has_read_receipts {
            self.notify_has_read_receipts();
        }
    }

    /// Get the ID of the event this `Event` replies to, if any.
    pub fn reply_to_id(&self) -> Option<OwnedEventId> {
        match self.imp().item.borrow().as_ref().unwrap().content() {
            TimelineItemContent::Message(message) => {
                message.in_reply_to().map(|d| d.event_id.clone())
            }
            _ => None,
        }
    }

    /// Whether this `Event` is a reply to another event.
    pub fn is_reply(&self) -> bool {
        self.reply_to_id().is_some()
    }

    /// Get the details of the event this `Event` replies to, if any.
    ///
    /// Returns `None(_)` if this event is not a reply.
    pub fn reply_to_event_content(&self) -> Option<TimelineDetails<Box<RepliedToEvent>>> {
        match self.imp().item.borrow().as_ref().unwrap().content() {
            TimelineItemContent::Message(message) => message.in_reply_to().map(|d| d.event.clone()),
            _ => None,
        }
    }

    /// Get the event this `Event` replies to, if any.
    ///
    /// Returns `None(_)` if this event is not a reply or if the event was not
    /// found locally.
    pub fn reply_to_event(&self) -> Option<Event> {
        let event_id = self.reply_to_id()?;
        self.room()
            .timeline()
            .event_by_key(&EventKey::EventId(event_id))
    }

    /// Fetch missing details for this event.
    ///
    /// This is a no-op if called for a local event.
    pub async fn fetch_missing_details(&self) -> Result<(), TimelineError> {
        let Some(event_id) = self.event_id() else {
            return Ok(());
        };

        let timeline = self.room().timeline().matrix_timeline();
        spawn_tokio!(async move { timeline.fetch_details_for_event(&event_id).await })
            .await
            .unwrap()
    }

    /// Fetch the content of the media message in this `Event`.
    ///
    /// Compatible events:
    ///
    /// - File message (`MessageType::File`).
    /// - Image message (`MessageType::Image`).
    /// - Video message (`MessageType::Video`).
    /// - Audio message (`MessageType::Audio`).
    ///
    /// Returns `Ok(binary_content)` on success.
    ///
    /// Returns `Err` if an error occurred while fetching the content. Panics on
    /// an incompatible event.
    pub async fn get_media_content(&self) -> Result<Vec<u8>, matrix_sdk::Error> {
        let Some(session) = self.room().session() else {
            return Err(matrix_sdk::Error::UnknownError(
                "Could not upgrade Session".into(),
            ));
        };
        let Some(message) = self.media_message() else {
            panic!("Trying to get the media content of an event of incompatible type");
        };

        let client = session.client();
        message.content(client).await
    }

    /// Whether this `Event` is considered a message.
    pub fn is_message(&self) -> bool {
        matches!(
            self.content(),
            TimelineItemContent::Message(_) | TimelineItemContent::Sticker(_)
        )
    }

    /// Deserialize this `Event` as an `OriginalSyncRoomMessageEvent`, if
    /// possible.
    pub fn as_message(&self) -> Option<OriginalSyncRoomMessageEvent> {
        self.raw()?
            .deserialize_as::<OriginalSyncRoomMessageEvent>()
            .ok()
    }

    /// Whether this `Event` can count as an unread message.
    ///
    /// This follows the algorithm in [MSC2654], excluding events that we don't
    /// show in the timeline.
    ///
    /// [MSC2654]: https://github.com/matrix-org/matrix-spec-proposals/pull/2654
    pub fn counts_as_unread(&self) -> bool {
        count_as_unread(self.imp().item.borrow().as_ref().unwrap().content())
    }

    /// The `matrix.to` URI representation for this event.
    ///
    /// Returns `None` if we don't have the ID of the event.
    pub async fn matrix_to_uri(&self) -> Option<MatrixToUri> {
        Some(self.room().matrix_to_event_uri(self.event_id()?).await)
    }

    /// The `matrix:` URI representation for this event.
    ///
    /// Returns `None` if we don't have the ID of the event.
    pub async fn matrix_uri(&self) -> Option<MatrixUri> {
        Some(self.room().matrix_event_uri(self.event_id()?).await)
    }

    /// Listen to the signal emitted when the SDK item changed.
    pub fn connect_item_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "item-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

/// Whether the given event can count as an unread message.
///
/// This follows the algorithm in [MSC2654], excluding events that we don't
/// show in the timeline.
///
/// [MSC2654]: https://github.com/matrix-org/matrix-spec-proposals/pull/2654
pub fn count_as_unread(content: &TimelineItemContent) -> bool {
    match content {
        TimelineItemContent::Message(message) => {
            !matches!(message.msgtype(), MessageType::Notice(_))
        }
        TimelineItemContent::Sticker(_) => true,
        TimelineItemContent::OtherState(state) => matches!(
            state.content(),
            AnyOtherFullStateEventContent::RoomTombstone(_)
        ),
        _ => false,
    }
}

/// Whether we can show the header for the given content.
pub fn content_can_show_header(content: &TimelineItemContent) -> bool {
    match content {
        TimelineItemContent::Message(message) => {
            matches!(
                message.msgtype(),
                MessageType::Audio(_)
                    | MessageType::File(_)
                    | MessageType::Image(_)
                    | MessageType::Location(_)
                    | MessageType::Notice(_)
                    | MessageType::Text(_)
                    | MessageType::Video(_)
            )
        }
        TimelineItemContent::Sticker(_) => true,
        _ => false,
    }
}

/// Convert raw JSON to a pretty-formatted JSON string.
fn raw_to_pretty_string<T>(raw: &Raw<T>) -> String {
    // We have to convert it to a Value, because a RawValue cannot be
    // pretty-printed.
    let json = serde_json::to_value(raw).unwrap();

    serde_json::to_string_pretty(&json).unwrap()
}

/// Raw unsigned event data.
///
/// Used as a fallback to get the latest edit's JSON.
#[derive(Debug, Clone, Deserialize)]
struct RawUnsigned {
    #[serde(rename = "m.relations")]
    relations: Option<RawBundledRelations>,
}

/// Raw bundled event relations.
///
/// Used as a fallback to get the latest edit's JSON.
#[derive(Debug, Clone, Deserialize)]
struct RawBundledRelations {
    #[serde(rename = "m.replace")]
    replace: Option<Raw<AnySyncTimelineEvent>>,
}

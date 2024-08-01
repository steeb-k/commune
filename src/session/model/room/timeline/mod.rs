mod timeline_item;
mod virtual_item;

use std::{collections::HashMap, sync::Arc};

use futures_util::StreamExt;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::Error as MatrixError;
use matrix_sdk_ui::{
    eyeball_im::VectorDiff,
    timeline::{
        default_event_filter, AnyOtherFullStateEventContent, LiveBackPaginationStatus, RoomExt,
        Timeline as SdkTimeline, TimelineItem as SdkTimelineItem, TimelineItemContent,
    },
};
use ruma::{
    events::{
        room::message::MessageType, AnySyncMessageLikeEvent, AnySyncStateEvent,
        AnySyncTimelineEvent, SyncMessageLikeEvent, TimelineEventType,
    },
    OwnedEventId, UserId,
};
use serde::{de::IgnoredAny, Deserialize};
use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

pub use self::{
    timeline_item::{TimelineItem, TimelineItemExt, TimelineItemImpl},
    virtual_item::{VirtualItem, VirtualItemKind},
};
use super::{Event, EventKey, Room};
use crate::{prelude::*, spawn, spawn_tokio};

/// List of events that should not be redacted to avoid bricking a room.
const NON_REDACTABLE_EVENTS: &[TimelineEventType] = &[
    TimelineEventType::RoomCreate,
    TimelineEventType::RoomEncryption,
    TimelineEventType::RoomServerAcl,
];

#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "TimelineState")]
pub enum TimelineState {
    #[default]
    Initial,
    Loading,
    Ready,
    Error,
    Complete,
}

impl From<LiveBackPaginationStatus> for TimelineState {
    fn from(value: LiveBackPaginationStatus) -> Self {
        match value {
            LiveBackPaginationStatus::Idle {
                hit_start_of_timeline: false,
            } => Self::Ready,
            LiveBackPaginationStatus::Idle {
                hit_start_of_timeline: true,
            } => Self::Complete,
            LiveBackPaginationStatus::Paginating => Self::Loading,
        }
    }
}

const MAX_BATCH_SIZE: u16 = 20;

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::Timeline)]
    pub struct Timeline {
        /// The room containing this timeline.
        #[property(get, set = Self::set_room, construct_only)]
        pub room: glib::WeakRef<Room>,
        /// The underlying SDK timeline.
        pub timeline: OnceCell<Arc<SdkTimeline>>,
        /// Items added at the start of the timeline.
        pub start_items: gio::ListStore,
        /// Items provided by the SDK timeline.
        pub sdk_items: gio::ListStore,
        /// Items added at the end of the timeline.
        pub end_items: gio::ListStore,
        /// The `GListModel` containing all the timeline items.
        #[property(get)]
        pub items: gtk::FlattenListModel,
        /// A Hashmap linking `EventKey` to corresponding `Event`
        pub event_map: RefCell<HashMap<EventKey, Event>>,
        /// The state of the timeline.
        #[property(get, builder(TimelineState::default()))]
        pub state: Cell<TimelineState>,
        /// Whether this timeline has a typing row.
        pub has_typing: Cell<bool>,
        pub diff_handle: OnceCell<AbortHandle>,
        pub back_pagination_status_handle: OnceCell<AbortHandle>,
        /// Whether the timeline is empty.
        #[property(get = Self::is_empty)]
        pub empty: PhantomData<bool>,
        /// Whether the timeline has the `m.room.create` event of the room.
        #[property(get)]
        pub has_room_create: Cell<bool>,
    }

    impl Default for Timeline {
        fn default() -> Self {
            let start_items = gio::ListStore::new::<TimelineItem>();
            let sdk_items = gio::ListStore::new::<TimelineItem>();
            let end_items = gio::ListStore::new::<TimelineItem>();

            let model_list = gio::ListStore::new::<gio::ListModel>();
            model_list.append(&start_items);
            model_list.append(&sdk_items);
            model_list.append(&end_items);

            Self {
                room: Default::default(),
                timeline: Default::default(),
                start_items,
                sdk_items,
                end_items,
                items: gtk::FlattenListModel::new(Some(model_list)),
                event_map: Default::default(),
                state: Default::default(),
                has_typing: Default::default(),
                diff_handle: Default::default(),
                back_pagination_status_handle: Default::default(),
                empty: Default::default(),
                has_room_create: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Timeline {
        const NAME: &'static str = "Timeline";
        type Type = super::Timeline;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Timeline {
        fn dispose(&self) {
            if let Some(handle) = self.diff_handle.get() {
                handle.abort();
            }
            if let Some(handle) = self.back_pagination_status_handle.get() {
                handle.abort();
            }
        }
    }

    impl Timeline {
        /// Set the room containing this timeline.
        fn set_room(&self, room: Option<Room>) {
            let obj = self.obj();
            self.room.set(room.as_ref());

            if let Some(room) = room {
                room.typing_list().connect_items_changed(clone!(
                    #[weak]
                    obj,
                    move |list, _, _, _| {
                        if !list.is_empty() {
                            obj.add_typing_row();
                        }
                    }
                ));
            }

            spawn!(clone!(
                #[weak]
                obj,
                async move {
                    obj.setup_timeline().await;
                }
            ));
        }

        /// Whether the timeline is empty.
        fn is_empty(&self) -> bool {
            self.sdk_items.n_items() == 0
        }

        /// Set whether the timeline has the `m.room.create` event of the room.
        pub(super) fn set_has_room_create(&self, has_room_create: bool) {
            if self.has_room_create.get() == has_room_create {
                return;
            }

            self.has_room_create.set(has_room_create);
            self.obj().notify_has_room_create();
        }
    }
}

glib::wrapper! {
    /// All loaded items in a room.
    ///
    /// There is no strict message ordering enforced by the Timeline; items
    /// will be appended/prepended to existing items in the order they are
    /// received by the server.
    pub struct Timeline(ObjectSubclass<imp::Timeline>);
}

impl Timeline {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// The `GListModel` containing only the items provided by the SDK.
    pub fn sdk_items(&self) -> &gio::ListModel {
        self.imp().sdk_items.upcast_ref()
    }

    /// Update this `Timeline` with the given diff.
    fn update(&self, diff: VectorDiff<Arc<SdkTimelineItem>>) {
        let Some(room) = self.room() else {
            return;
        };
        let imp = self.imp();
        let sdk_items = &imp.sdk_items;
        let was_empty = self.empty();

        match diff {
            VectorDiff::Append { values } => {
                let new_list = values
                    .into_iter()
                    .map(|item| self.create_item(&item))
                    .collect::<Vec<_>>();

                let pos = sdk_items.n_items();
                let added = new_list.len() as u32;

                sdk_items.extend_from_slice(&new_list);
                self.update_items_headers(pos, added.max(1));

                // Try to update the latest unread message.
                room.update_latest_activity(
                    new_list.iter().filter_map(|i| i.downcast_ref::<Event>()),
                );
            }
            VectorDiff::Clear => {
                imp.sdk_items.remove_all();
                imp.event_map.borrow_mut().clear();
                imp.set_has_room_create(false);
            }
            VectorDiff::PushFront { value } => {
                let item = self.create_item(&value);

                sdk_items.insert(0, &item);
                self.update_items_headers(0, 1);

                // Try to update the latest unread message.
                if let Some(event) = item.downcast_ref::<Event>() {
                    room.update_latest_activity([event]);
                }
            }
            VectorDiff::PushBack { value } => {
                let item = self.create_item(&value);
                let pos = sdk_items.n_items();
                sdk_items.append(&item);
                self.update_items_headers(pos, 1);

                // Try to update the latest unread message.
                if let Some(event) = item.downcast_ref::<Event>() {
                    room.update_latest_activity([event]);
                }
            }
            VectorDiff::PopFront => {
                let item = sdk_items.item(0).and_downcast().unwrap();
                self.remove_item(&item);

                sdk_items.remove(0);
                self.update_items_headers(0, 1);
            }
            VectorDiff::PopBack => {
                let pos = sdk_items.n_items() - 1;
                let item = sdk_items.item(pos).and_downcast().unwrap();
                self.remove_item(&item);

                sdk_items.remove(pos);
            }
            VectorDiff::Insert { index, value } => {
                let pos = index as u32;
                let item = self.create_item(&value);

                sdk_items.insert(pos, &item);
                self.update_items_headers(pos, 1);

                // Try to update the latest unread message.
                if let Some(event) = item.downcast_ref::<Event>() {
                    room.update_latest_activity([event]);
                }
            }
            VectorDiff::Set { index, value } => {
                let pos = index as u32;
                let prev_item = sdk_items.item(pos).and_downcast::<TimelineItem>().unwrap();

                let item = if !prev_item.try_update_with(&value) {
                    self.remove_item(&prev_item);
                    let item = self.create_item(&value);

                    sdk_items.splice(pos, 1, &[item.clone()]);

                    item
                } else {
                    prev_item
                };

                // The item's header visibility might have changed.
                self.update_items_headers(pos, 1);

                // Try to update the latest unread message.
                if let Some(event) = item.downcast_ref::<Event>() {
                    room.update_latest_activity([event]);
                }
            }
            VectorDiff::Remove { index } => {
                let pos = index as u32;
                let item = sdk_items.item(pos).and_downcast().unwrap();
                self.remove_item(&item);

                sdk_items.remove(pos);
                self.update_items_headers(pos, 1);
            }
            VectorDiff::Truncate { length } => {
                let new_len = length as u32;
                let old_len = sdk_items.n_items();

                for pos in new_len..old_len {
                    let item = sdk_items.item(pos).and_downcast().unwrap();
                    self.remove_item(&item);
                }

                sdk_items.splice(new_len, old_len - new_len, &[] as &[glib::Object]);
            }
            VectorDiff::Reset { values } => {
                // Reset the state.
                imp.event_map.borrow_mut().clear();
                imp.set_has_room_create(false);

                let new_list = values
                    .into_iter()
                    .map(|item| self.create_item(&item))
                    .collect::<Vec<_>>();

                let removed = sdk_items.n_items();
                let added = new_list.len() as u32;

                sdk_items.splice(0, removed, &new_list);
                self.update_items_headers(0, added.max(1));

                // Try to update the latest unread message.
                room.update_latest_activity(
                    new_list.iter().filter_map(|i| i.downcast_ref::<Event>()),
                );
            }
        }

        if self.empty() != was_empty {
            self.notify_empty();
        }
    }

    /// Update `nb` items' headers starting at `pos`.
    fn update_items_headers(&self, pos: u32, nb: u32) {
        let sdk_items = &self.imp().sdk_items;

        let mut previous_sender = if pos > 0 {
            sdk_items
                .item(pos - 1)
                .and_downcast::<TimelineItem>()
                .filter(|item| item.can_hide_header())
                .and_then(|item| item.event_sender_id())
        } else {
            None
        };

        // Update the headers of changed events plus the first event after them.
        for current_pos in pos..pos + nb + 1 {
            let Some(current) = sdk_items.item(current_pos).and_downcast::<TimelineItem>() else {
                break;
            };

            let current_sender = current.event_sender_id();

            if !current.can_hide_header() {
                current.set_show_header(false);
                previous_sender = None;
            } else if current_sender != previous_sender {
                current.set_show_header(true);
                previous_sender = current_sender;
            } else {
                current.set_show_header(false);
            }
        }
    }

    /// Create a `TimelineItem` in this `Timeline` from the given SDK timeline
    /// item.
    fn create_item(&self, item: &SdkTimelineItem) -> TimelineItem {
        let room = self.room().unwrap();
        let item = TimelineItem::new(item, &room);

        if let Some(event) = item.downcast_ref::<Event>() {
            let imp = self.imp();

            imp.event_map
                .borrow_mut()
                .insert(event.key(), event.clone());

            // Keep track of the activity of the sender.
            if event.counts_as_unread() {
                if let Some(members) = room.members() {
                    let member = members.get_or_create(event.sender_id());
                    member.set_latest_activity(event.origin_server_ts_u64());
                }
            }

            if is_room_create_event(event) {
                imp.set_has_room_create(true);
            }
        }

        item
    }

    /// Remove the given item from this `Timeline`.
    fn remove_item(&self, item: &TimelineItem) {
        if let Some(event) = item.downcast_ref::<Event>() {
            let imp = self.imp();

            imp.event_map.borrow_mut().remove(&event.key());

            if is_room_create_event(event) {
                imp.set_has_room_create(false);
            }
        }
    }

    /// Whether it's possible to load more events with the current state of the
    /// timeline.
    pub fn can_load(&self) -> bool {
        // We don't want to load twice at the same time, and it's useless to try to load
        // more history before the timeline is ready or when we reached the
        // start.
        !matches!(
            self.state(),
            TimelineState::Initial | TimelineState::Loading | TimelineState::Complete
        )
    }

    /// Load events at the start of the timeline.
    pub async fn load(&self) {
        if !self.can_load() {
            return;
        }

        self.set_state(TimelineState::Loading);

        let matrix_timeline = self.matrix_timeline();
        let handle =
            spawn_tokio!(async move { matrix_timeline.paginate_backwards(MAX_BATCH_SIZE).await });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not load timeline: {error}");
            self.set_state(TimelineState::Error);
        }
    }

    /// Get the event with the given key from this `Timeline`.
    ///
    /// Use this method if you are sure the event has already been received.
    /// Otherwise use `fetch_event_by_id`.
    pub fn event_by_key(&self, key: &EventKey) -> Option<Event> {
        self.imp().event_map.borrow().get(key).cloned()
    }

    /// Get the position of the event with the given key in this `Timeline`.
    pub fn find_event_position(&self, key: &EventKey) -> Option<usize> {
        for (pos, item) in self
            .items()
            .iter::<glib::Object>()
            .map(|o| o.ok().and_downcast::<TimelineItem>())
            .enumerate()
        {
            let Some(item) = item else {
                break;
            };

            if let Some(event) = item.downcast_ref::<Event>() {
                if event.key() == *key {
                    return Some(pos);
                }
            }
        }

        None
    }

    /// Fetch the event with the given id.
    ///
    /// If the event can't be found locally, a request will be made to the
    /// homeserver.
    ///
    /// Use this method if you are not sure the event has already been received.
    /// Otherwise use `event_by_id`.
    pub async fn fetch_event_by_id(
        &self,
        event_id: OwnedEventId,
    ) -> Result<AnySyncTimelineEvent, MatrixError> {
        if let Some(event) = self.event_by_key(&EventKey::EventId(event_id.clone())) {
            event.raw().unwrap().deserialize().map_err(Into::into)
        } else {
            let Some(room) = self.room() else {
                return Err(MatrixError::UnknownError("Could not upgrade Room".into()));
            };
            let matrix_room = room.matrix_room().clone();
            let event_id_clone = event_id.clone();
            let handle = spawn_tokio!(async move { matrix_room.event(&event_id_clone).await });
            match handle.await.unwrap() {
                Ok(room_event) => room_event.event.deserialize_as().map_err(Into::into),
                Err(error) => {
                    // TODO: Retry on connection error?
                    warn!("Could not fetch event {event_id}: {error}");
                    Err(error)
                }
            }
        }
    }

    /// Setup the underlying SDK timeline.
    async fn setup_timeline(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let imp = self.imp();
        let room_id = room.room_id().to_owned();
        let matrix_room = room.matrix_room().clone();

        let handle = spawn_tokio!(async move {
            matrix_room
                .timeline_builder()
                .event_filter(|any, room_version| {
                    // Make sure we don't try to show events that can't be shown.
                    if !default_event_filter(any, room_version) {
                        return false;
                    }

                    // Only show events we want.
                    match any {
                        AnySyncTimelineEvent::MessageLike(msg) => match msg {
                            AnySyncMessageLikeEvent::RoomMessage(
                                SyncMessageLikeEvent::Original(ev),
                            ) => {
                                matches!(
                                    ev.content.msgtype,
                                    MessageType::Audio(_)
                                        | MessageType::Emote(_)
                                        | MessageType::File(_)
                                        | MessageType::Image(_)
                                        | MessageType::Location(_)
                                        | MessageType::Notice(_)
                                        | MessageType::ServerNotice(_)
                                        | MessageType::Text(_)
                                        | MessageType::Video(_)
                                )
                            }
                            AnySyncMessageLikeEvent::Sticker(SyncMessageLikeEvent::Original(_))
                            | AnySyncMessageLikeEvent::RoomEncrypted(
                                SyncMessageLikeEvent::Original(_),
                            ) => true,
                            _ => false,
                        },
                        AnySyncTimelineEvent::State(state) => matches!(
                            state,
                            AnySyncStateEvent::RoomMember(_)
                                | AnySyncStateEvent::RoomCreate(_)
                                | AnySyncStateEvent::RoomEncryption(_)
                                | AnySyncStateEvent::RoomThirdPartyInvite(_)
                                | AnySyncStateEvent::RoomTombstone(_)
                        ),
                    }
                })
                .add_failed_to_parse(false)
                .build()
                .await
        });

        let matrix_timeline = match handle.await.unwrap() {
            Ok(t) => t,
            Err(error) => {
                error!("Could not create timeline: {error}");
                return;
            }
        };

        let matrix_timeline = Arc::new(matrix_timeline);
        imp.timeline.set(matrix_timeline.clone()).unwrap();

        let (values, timeline_stream) = matrix_timeline.subscribe().await;

        if !values.is_empty() {
            self.update(VectorDiff::Append { values });
        }

        let obj_weak = glib::SendWeakRef::from(self.downgrade());
        let fut = timeline_stream.for_each(move |diff| {
            let obj_weak = obj_weak.clone();
            let room_id = room_id.clone();
            async move {
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    spawn!(async move {
                        if let Some(obj) = obj_weak.upgrade() {
                            obj.update(diff);
                        } else {
                            error!("Could not send timeline diff for room {room_id}: could not upgrade weak reference");
                        }
                    });
                });
            }
        });

        let diff_handle = spawn_tokio!(fut);
        imp.diff_handle.set(diff_handle.abort_handle()).unwrap();

        self.setup_back_pagination_status().await;
    }

    /// Setup the back-pagination status.
    async fn setup_back_pagination_status(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let room_id = room.room_id().to_owned();
        let matrix_timeline = self.matrix_timeline();

        let (status, stream) = matrix_timeline
            .live_back_pagination_status()
            .await
            .expect("Timeline should be in live mode");

        self.set_state(status.into());

        let obj_weak = glib::SendWeakRef::from(self.downgrade());
        let fut = stream.for_each(move |status| {
            let obj_weak = obj_weak.clone();
            let room_id = room_id.clone();
            async move {
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    spawn!(async move {
                        if let Some(obj) = obj_weak.upgrade() {
                            obj.set_state(status.into());
                        } else {
                            error!("Could not send timeline back-pagination status for room {room_id}: could not upgrade weak reference");
                        }
                    });
                });
            }
        });

        let back_pagination_status_handle = spawn_tokio!(fut);
        self.imp()
            .back_pagination_status_handle
            .set(back_pagination_status_handle.abort_handle())
            .unwrap();
    }

    /// The underlying SDK timeline.
    pub fn matrix_timeline(&self) -> Arc<SdkTimeline> {
        self.imp().timeline.get().unwrap().clone()
    }

    fn set_state(&self, state: TimelineState) {
        let imp = self.imp();
        let prev_state = self.state();

        if state == prev_state {
            return;
        }

        imp.state.set(state);

        let start_items = &imp.start_items;
        let removed = start_items.n_items();

        match state {
            TimelineState::Loading => start_items.splice(0, removed, &[VirtualItem::spinner()]),
            TimelineState::Complete => {
                start_items.splice(0, removed, &[VirtualItem::timeline_start()])
            }
            _ => start_items.remove_all(),
        }

        self.notify_state();
    }

    fn has_typing_row(&self) -> bool {
        self.imp().end_items.n_items() > 0
    }

    fn add_typing_row(&self) {
        if self.has_typing_row() {
            return;
        }

        self.imp().end_items.append(&VirtualItem::typing());
    }

    pub fn remove_empty_typing_row(&self) {
        if !self.has_typing_row() || !self.room().is_some_and(|r| r.typing_list().is_empty()) {
            return;
        }

        self.imp().end_items.remove_all();
    }

    /// Whether this timeline has unread messages.
    ///
    /// Returns `None` if it is not possible to know, for example if there are
    /// no events in the Timeline.
    pub async fn has_unread_messages(&self) -> Option<bool> {
        let room = self.room()?;
        let session = room.session()?;
        let own_user_id = session.user_id().clone();
        let matrix_timeline = self.matrix_timeline();

        let (actual_receipt_event_id, user_receipt_item) = spawn_tokio!(async move {
            let actual_receipt_event_id = matrix_timeline
                .latest_user_read_receipt(&own_user_id)
                .await
                .map(|(event_id, _)| event_id);
            let user_receipt_item = matrix_timeline
                .latest_user_read_receipt_timeline_event_id(&own_user_id)
                .await;
            (actual_receipt_event_id, user_receipt_item)
        })
        .await
        .unwrap();

        tracing::trace!(
            "{}::has_unread_messages: Read receipt at actual event {actual_receipt_event_id:?}, visible at timeline event {user_receipt_item:?}",
            room.human_readable_id(),
        );

        let sdk_items = &self.imp().sdk_items;
        let count = sdk_items.n_items();

        for pos in (0..count).rev() {
            let Some(event) = sdk_items.item(pos).and_downcast::<Event>() else {
                continue;
            };

            if user_receipt_item.is_some() && event.event_id() == user_receipt_item {
                // The event is the oldest one, we have read it all.
                tracing::trace!(
                    "{}::has_unread_messages: Got event {:?} from read receipt",
                    room.human_readable_id(),
                    event.key()
                );
                return Some(false);
            }
            if event.counts_as_unread() {
                // There is at least one unread event.
                tracing::trace!(
                    "{}::has_unread_messages: Event {:?} is unread",
                    room.human_readable_id(),
                    event.key()
                );
                return Some(true);
            }
        }

        // This should only happen if we do not have a read receipt item in the
        // timeline, and there are not enough events in the timeline to know if there
        // are unread messages.
        None
    }

    /// The IDs of redactable events sent by the given user in this timeline.
    pub fn redactable_events_for(&self, user_id: &UserId) -> Vec<OwnedEventId> {
        let mut events = vec![];

        for item in self.imp().sdk_items.iter::<glib::Object>() {
            let Ok(item) = item else {
                // The iterator is broken.
                break;
            };
            let Ok(event) = item.downcast::<Event>() else {
                continue;
            };

            if event.sender_id() != user_id {
                continue;
            }

            if is_event_redactable(&event) {
                if let Some(event_id) = event.event_id() {
                    events.push(event_id);
                }
            }
        }

        events
    }
}

/// Whether the given event is an `m.room.create` event.
fn is_room_create_event(event: &Event) -> bool {
    match event.content() {
        TimelineItemContent::OtherState(other_state) => matches!(
            other_state.content(),
            AnyOtherFullStateEventContent::RoomCreate(_)
        ),
        _ => false,
    }
}

/// Whether the given event can be redacted.
fn is_event_redactable(event: &Event) -> bool {
    let Some(raw) = event.raw() else {
        // Events without raw JSON are already redacted events, and events that are not
        // sent yet, we can ignore them.
        return false;
    };

    let is_redacted = match raw.get_field::<UnsignedDeHelper>("unsigned") {
        Ok(Some(unsigned)) => unsigned.redacted_because.is_some(),
        Ok(None) => {
            debug!("Missing unsigned field in event");
            false
        }
        Err(error) => {
            error!("Could not deserialize unsigned field in event: {error}");
            false
        }
    };
    if is_redacted {
        // There is no point in redacting it twice.
        return false;
    }

    match raw.get_field::<TimelineEventType>("type") {
        Ok(Some(t)) => !NON_REDACTABLE_EVENTS.contains(&t),
        Ok(None) => {
            debug!("Missing type field in event");
            true
        }
        Err(error) => {
            error!("Could not deserialize type field in event: {error}");
            true
        }
    }
}

#[derive(Deserialize)]
struct UnsignedDeHelper {
    redacted_because: Option<IgnoredAny>,
}

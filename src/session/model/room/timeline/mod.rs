mod timeline_item;
mod virtual_item;

use std::{collections::HashMap, ops::ControlFlow, sync::Arc};

use futures_util::StreamExt;
use gtk::{
    gio, glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk_ui::{
    eyeball_im::VectorDiff,
    timeline::{
        default_event_filter, RoomExt, Timeline as SdkTimeline, TimelineEventItemId,
        TimelineItem as SdkTimelineItem,
    },
};
use ruma::{
    events::{
        room::message::MessageType, AnySyncMessageLikeEvent, AnySyncStateEvent,
        AnySyncTimelineEvent, SyncMessageLikeEvent, SyncStateEvent,
    },
    OwnedEventId, RoomVersionId, UserId,
};
use tokio::task::AbortHandle;
use tracing::error;

pub(crate) use self::{
    timeline_item::{TimelineItem, TimelineItemImpl},
    virtual_item::{VirtualItem, VirtualItemKind},
};
use super::{Event, Room};
use crate::{prelude::*, spawn, spawn_tokio};

/// The possible states of the timeline.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "TimelineState")]
pub enum TimelineState {
    /// The timeline is not initialized yet.
    #[default]
    Initial,
    /// The timeline is currently loading.
    Loading,
    /// The timeline has been initialized and there is no ongoing action.
    Ready,
    /// An error occurred with the timeline.
    Error,
    /// We have reached the beginning of the timeline.
    Complete,
}

/// The number of events to request when loading more history.
const MAX_BATCH_SIZE: u16 = 20;

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::Timeline)]
    pub struct Timeline {
        /// The room containing this timeline.
        #[property(get, set = Self::set_room, construct_only)]
        room: glib::WeakRef<Room>,
        /// The underlying SDK timeline.
        matrix_timeline: OnceCell<Arc<SdkTimeline>>,
        /// Items added at the start of the timeline.
        start_items: gio::ListStore,
        /// Items provided by the SDK timeline.
        pub(super) sdk_items: gio::ListStore,
        /// Items added at the end of the timeline.
        end_items: gio::ListStore,
        /// The `GListModel` containing all the timeline items.
        #[property(get)]
        items: gtk::FlattenListModel,
        /// A Hashmap linking a `TimelineEventItemId` to the corresponding
        /// `Event`.
        pub(super) event_map: RefCell<HashMap<TimelineEventItemId, Event>>,
        /// The state of the timeline.
        #[property(get, builder(TimelineState::default()))]
        state: Cell<TimelineState>,
        /// Whether the timeline is empty.
        #[property(get = Self::is_empty)]
        is_empty: PhantomData<bool>,
        /// Whether the timeline has the `m.room.create` event of the room.
        #[property(get)]
        has_room_create: Cell<bool>,
        diff_handle: OnceCell<AbortHandle>,
        back_pagination_status_handle: OnceCell<AbortHandle>,
        read_receipts_changed_handle: OnceCell<AbortHandle>,
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
                matrix_timeline: Default::default(),
                start_items,
                sdk_items,
                end_items,
                items: gtk::FlattenListModel::new(Some(model_list)),
                event_map: Default::default(),
                state: Default::default(),
                is_empty: Default::default(),
                has_room_create: Default::default(),
                diff_handle: Default::default(),
                back_pagination_status_handle: Default::default(),
                read_receipts_changed_handle: Default::default(),
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
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("read-change-trigger").build()]);
            SIGNALS.as_ref()
        }

        fn dispose(&self) {
            if let Some(handle) = self.diff_handle.get() {
                handle.abort();
            }
            if let Some(handle) = self.back_pagination_status_handle.get() {
                handle.abort();
            }
            if let Some(handle) = self.read_receipts_changed_handle.get() {
                handle.abort();
            }
        }
    }

    impl Timeline {
        /// Set the room containing this timeline.
        fn set_room(&self, room: &Room) {
            self.room.set(Some(room));

            room.typing_list().connect_is_empty_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |list| {
                    if !list.is_empty() {
                        imp.add_typing_row();
                    }
                }
            ));

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.init_matrix_timeline().await;
                }
            ));
        }

        /// Initialize the underlying SDK timeline.
        async fn init_matrix_timeline(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let room_id = room.room_id().to_owned();
            let matrix_room = room.matrix_room().clone();

            let handle = spawn_tokio!(async move {
                matrix_room
                    .timeline_builder()
                    .event_filter(show_in_timeline)
                    .add_failed_to_parse(false)
                    .build()
                    .await
            });

            let matrix_timeline = match handle.await.expect("task was not aborted") {
                Ok(timeline) => timeline,
                Err(error) => {
                    error!("Could not create timeline: {error}");
                    return;
                }
            };

            let matrix_timeline = Arc::new(matrix_timeline);
            self.matrix_timeline
                .set(matrix_timeline.clone())
                .expect("matrix timeline was uninitialized");

            let (values, timeline_stream) = matrix_timeline.subscribe().await;

            if !values.is_empty() {
                self.update(VectorDiff::Append { values });
            }

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let fut = timeline_stream.for_each(move |diff| {
                let obj_weak = obj_weak.clone();
                let room_id = room_id.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.imp().update(diff);
                            } else {
                                error!(
                                    "Could not send timeline diff for room {room_id}: \
                                     could not upgrade weak reference"
                                );
                            }
                        });
                    });
                }
            });

            let diff_handle = spawn_tokio!(fut);
            self.diff_handle.set(diff_handle.abort_handle()).unwrap();

            self.watch_read_receipts().await;
            self.set_state(TimelineState::Ready);
        }

        /// The underlying SDK timeline.
        pub(super) fn matrix_timeline(&self) -> &Arc<SdkTimeline> {
            self.matrix_timeline
                .get()
                .expect("matrix timeline is initialized")
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

        /// Update this `Timeline` with the given diff.
        #[allow(clippy::too_many_lines)]
        fn update(&self, diff: VectorDiff<Arc<SdkTimelineItem>>) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let sdk_items = &self.sdk_items;
            let was_empty = self.is_empty();

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
                    sdk_items.remove_all();
                    self.event_map.borrow_mut().clear();
                    self.set_has_room_create(false);
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

                    let item = if prev_item.try_update_with(&value) {
                        if let Some(event) = prev_item.downcast_ref::<Event>() {
                            // Update the identifier in the event map, in case we switched from a
                            // transaction ID to an event ID.
                            self.event_map
                                .borrow_mut()
                                .insert(event.identifier(), event.clone());
                        }

                        prev_item
                    } else {
                        self.remove_item(&prev_item);
                        let item = self.create_item(&value);

                        sdk_items.splice(pos, 1, &[item.clone()]);

                        item
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
                    self.event_map.borrow_mut().clear();
                    self.set_has_room_create(false);

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

            let obj = self.obj();
            if self.is_empty() != was_empty {
                obj.notify_is_empty();
            }

            obj.emit_read_change_trigger();
        }

        /// Update `nb` items' headers starting at `pos`.
        fn update_items_headers(&self, pos: u32, nb: u32) {
            let sdk_items = &self.sdk_items;

            let mut previous_sender = if pos > 0 {
                sdk_items
                    .item(pos - 1)
                    .and_downcast::<TimelineItem>()
                    .filter(TimelineItem::can_hide_header)
                    .and_then(|item| item.event_sender_id())
            } else {
                None
            };

            // Update the headers of changed events plus the first event after them.
            for current_pos in pos..=pos + nb {
                let Some(current) = sdk_items.item(current_pos).and_downcast::<TimelineItem>()
                else {
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

        /// Create a `TimelineItem` in this `Timeline` from the given SDK
        /// timeline item.
        fn create_item(&self, item: &SdkTimelineItem) -> TimelineItem {
            let room = self
                .room
                .upgrade()
                .expect("there is a strong reference to the Room");
            let item = TimelineItem::new(item, &room);

            if let Some(event) = item.downcast_ref::<Event>() {
                self.event_map
                    .borrow_mut()
                    .insert(event.identifier(), event.clone());

                // Keep track of the activity of the sender.
                if event.counts_as_unread() {
                    if let Some(members) = room.members() {
                        let member = members.get_or_create(event.sender_id());
                        member.set_latest_activity(u64::from(event.origin_server_ts().get()));
                    }
                }

                if event.is_room_create_event() {
                    self.set_has_room_create(true);
                }
            }

            item
        }

        /// Remove the given item from this `Timeline`.
        fn remove_item(&self, item: &TimelineItem) {
            if let Some(event) = item.downcast_ref::<Event>() {
                // We need to remove both the transaction ID and the event ID.
                if let Some(txn_id) = event.transaction_id() {
                    self.event_map
                        .borrow_mut()
                        .remove(&TimelineEventItemId::TransactionId(txn_id));
                }
                if let Some(event_id) = event.event_id() {
                    self.event_map
                        .borrow_mut()
                        .remove(&TimelineEventItemId::EventId(event_id));
                }

                if event.is_room_create_event() {
                    self.set_has_room_create(false);
                }
            }
        }

        /// Load more events at the start of the timeline.
        ///
        /// Returns `true` if more events can be loaded.
        pub(super) async fn load(&self) -> bool {
            let matrix_timeline = self.matrix_timeline().clone();
            let handle =
                spawn_tokio!(
                    async move { matrix_timeline.paginate_backwards(MAX_BATCH_SIZE).await }
                );

            match handle.await.expect("task was not aborted") {
                Ok(reached_start) => {
                    if reached_start {
                        self.set_state(TimelineState::Complete);
                    }

                    !reached_start
                }
                Err(error) => {
                    error!("Could not load timeline: {error}");
                    self.set_state(TimelineState::Error);
                    false
                }
            }
        }

        /// Set the state of the timeline.
        pub(super) fn set_state(&self, state: TimelineState) {
            if self.state.get() == state {
                return;
            }

            self.state.set(state);

            let start_items = &self.start_items;
            let removed = start_items.n_items();

            match state {
                TimelineState::Loading => start_items.splice(0, removed, &[VirtualItem::spinner()]),
                TimelineState::Complete => {
                    start_items.splice(0, removed, &[VirtualItem::timeline_start()]);
                }
                _ => start_items.remove_all(),
            }

            self.obj().notify_state();
        }

        /// Whether the timeline has a typing row.
        fn has_typing_row(&self) -> bool {
            self.end_items.n_items() > 0
        }

        /// Add the typing row to the timeline, if it isn't present already.
        fn add_typing_row(&self) {
            if self.has_typing_row() {
                return;
            }

            self.end_items.append(&VirtualItem::typing());
        }

        /// Remove the typing row from the timeline.
        pub fn remove_empty_typing_row(&self) {
            if !self.has_typing_row()
                || !self
                    .room
                    .upgrade()
                    .is_some_and(|r| r.typing_list().is_empty())
            {
                return;
            }

            self.end_items.remove_all();
        }

        /// Listen to read receipts changes.
        async fn watch_read_receipts(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let room_id = room.room_id().to_owned();
            let matrix_timeline = self.matrix_timeline();

            let stream = matrix_timeline
                .subscribe_own_user_read_receipts_changed()
                .await;

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let fut = stream.for_each(move |()| {
                let obj_weak = obj_weak.clone();
                let room_id = room_id.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.emit_read_change_trigger();
                            } else {
                                error!(
                                    "Could not emit read change trigger for room {room_id}: \
                                     could not upgrade weak reference"
                                );
                            }
                        });
                    });
                }
            });

            let handle = spawn_tokio!(fut);
            self.read_receipts_changed_handle
                .set(handle.abort_handle())
                .unwrap();
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
    /// Construct a new `Timeline` for the given room.
    pub(crate) fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// The underlying SDK timeline.
    pub(crate) fn matrix_timeline(&self) -> Arc<SdkTimeline> {
        self.imp().matrix_timeline().clone()
    }

    /// Whether we can load more events with the current state of the timeline.
    fn can_load(&self) -> bool {
        // We don't want to load twice at the same time, and it's useless to try to load
        // more history before the timeline is ready or when we reached the
        // start.
        !matches!(
            self.state(),
            TimelineState::Initial | TimelineState::Loading | TimelineState::Complete
        )
    }

    /// Load more events at the start of the timeline until the given function
    /// tells us to stop.
    pub(crate) async fn load<F>(&self, continue_fn: F)
    where
        F: Fn() -> ControlFlow<()>,
    {
        if !self.can_load() {
            return;
        }

        let imp = self.imp();
        imp.set_state(TimelineState::Loading);

        loop {
            if !imp.load().await {
                return;
            }

            if continue_fn().is_break() {
                imp.set_state(TimelineState::Ready);
                return;
            }
        }
    }

    /// Get the event with the given identifier from this `Timeline`.
    ///
    /// Use this method if you are sure the event has already been received.
    /// Otherwise use `fetch_event_by_id`.
    pub(crate) fn event_by_identifier(&self, identifier: &TimelineEventItemId) -> Option<Event> {
        self.imp().event_map.borrow().get(identifier).cloned()
    }

    /// Get the position of the event with the given identifier in this
    /// `Timeline`.
    pub(crate) fn find_event_position(&self, identifier: &TimelineEventItemId) -> Option<usize> {
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
                if event.matches_identifier(identifier) {
                    return Some(pos);
                }
            }
        }

        None
    }

    /// Remove the typing row from the timeline.
    pub(crate) fn remove_empty_typing_row(&self) {
        self.imp().remove_empty_typing_row();
    }

    /// Whether this timeline has unread messages.
    ///
    /// Returns `None` if it is not possible to know, for example if there are
    /// no events in the Timeline.
    pub(crate) async fn has_unread_messages(&self) -> Option<bool> {
        let room = self.room()?;
        let session = room.session()?;
        let own_user_id = session.user_id().clone();
        let matrix_timeline = self.matrix_timeline();

        let user_receipt_item = spawn_tokio!(async move {
            matrix_timeline
                .latest_user_read_receipt_timeline_event_id(&own_user_id)
                .await
        })
        .await
        .unwrap();

        let sdk_items = &self.imp().sdk_items;
        let count = sdk_items.n_items();

        for pos in (0..count).rev() {
            let Some(event) = sdk_items.item(pos).and_downcast::<Event>() else {
                continue;
            };

            if user_receipt_item.is_some() && event.event_id() == user_receipt_item {
                // The event is the oldest one, we have read it all.
                return Some(false);
            }
            if event.counts_as_unread() {
                // There is at least one unread event.
                return Some(true);
            }
        }

        // This should only happen if we do not have a read receipt item in the
        // timeline, and there are not enough events in the timeline to know if there
        // are unread messages.
        None
    }

    /// The IDs of redactable events sent by the given user in this timeline.
    pub(crate) fn redactable_events_for(&self, user_id: &UserId) -> Vec<OwnedEventId> {
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

            if event.can_be_redacted() {
                if let Some(event_id) = event.event_id() {
                    events.push(event_id);
                }
            }
        }

        events
    }

    /// Emit the trigger that a read change might have occurred.
    fn emit_read_change_trigger(&self) {
        self.emit_by_name::<()>("read-change-trigger", &[]);
    }

    /// Connect to the trigger emitted when a read change might have occurred.
    pub(crate) fn connect_read_change_trigger<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "read-change-trigger",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

/// Whether the given event should be shown in the timeline.
fn show_in_timeline(any: &AnySyncTimelineEvent, room_version: &RoomVersionId) -> bool {
    // Make sure we do not show events that cannot be shown.
    if !default_event_filter(any, room_version) {
        return false;
    }

    // Only show events we want.
    match any {
        AnySyncTimelineEvent::MessageLike(msg) => match msg {
            AnySyncMessageLikeEvent::RoomMessage(SyncMessageLikeEvent::Original(ev)) => {
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
            | AnySyncMessageLikeEvent::RoomEncrypted(SyncMessageLikeEvent::Original(_)) => true,
            _ => false,
        },
        AnySyncTimelineEvent::State(AnySyncStateEvent::RoomMember(SyncStateEvent::Original(
            member_event,
        ))) => {
            // Do not show member events if the content that we support has not
            // changed. This avoids duplicate "user has joined" events in the
            // timeline which are confusing and wrong.
            !member_event
                .unsigned
                .prev_content
                .as_ref()
                .is_some_and(|prev_content| {
                    prev_content.membership == member_event.content.membership
                        && prev_content.displayname == member_event.content.displayname
                        && prev_content.avatar_url == member_event.content.avatar_url
                })
        }
        AnySyncTimelineEvent::State(state) => matches!(
            state,
            AnySyncStateEvent::RoomMember(_)
                | AnySyncStateEvent::RoomCreate(_)
                | AnySyncStateEvent::RoomEncryption(_)
                | AnySyncStateEvent::RoomThirdPartyInvite(_)
                | AnySyncStateEvent::RoomTombstone(_)
        ),
    }
}

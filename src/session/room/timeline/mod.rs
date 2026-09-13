use std::{collections::HashMap, ops::ControlFlow, sync::Arc};

use commune_core::session::{MAX_BATCH_SIZE, Timeline as CoreTimeline, TimelineFocusKind};
use futures_util::StreamExt;
use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use matrix_sdk_ui::{
    eyeball_im::VectorDiff,
    timeline::{Timeline as SdkTimeline, TimelineEventItemId, TimelineItem as SdkTimelineItem},
};
use ruma::{
    OwnedEventId, UserId, api::client::receipt::create_receipt::v3::ReceiptType as ApiReceiptType,
};
use tokio::task::AbortHandle;
use tracing::error;

mod event;
mod timeline_diff_minimizer;
mod timeline_item;
mod virtual_item;

use self::timeline_diff_minimizer::{TimelineDiff, TimelineDiffItemStore};
pub(crate) use self::{
    event::*,
    timeline_item::{TimelineItem, TimelineItemExt, TimelineItemImpl},
    virtual_item::{VirtualItem, VirtualItemKind},
};
use super::{ReceiptPosition, Room};
use crate::{
    core_bridge::ObjectWatcher,
    spawn, spawn_tokio,
    utils::{LoadingState, SingleItemListModel},
};

/// The maximum time between contiguous events before we show their header, in
/// milliseconds.
///
/// This matches 20 minutes.
const MAX_TIME_BETWEEN_HEADERS: u64 = 20 * 60 * 1000;

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
        sync::LazyLock,
    };

    use glib::clone;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Timeline)]
    pub struct Timeline {
        /// The room containing this timeline.
        #[property(get, set = Self::set_room, construct_only)]
        room: OnceCell<Room>,
        /// The timeline, as the core keeps it.
        core: OnceCell<CoreTimeline>,
        /// The underlying SDK timeline.
        matrix_timeline: OnceCell<Arc<SdkTimeline>>,
        /// Items added at the start of the timeline.
        ///
        /// Currently this can only contain one item at a time.
        start_items: OnceCell<SingleItemListModel>,
        /// Items provided by the SDK timeline.
        sdk_items: OnceCell<gio::ListStore>,
        /// Filter for the list of items provided by the SDK timeline.
        filter: gtk::CustomFilter,
        /// Filtered list of items provided by the SDK timeline.
        filtered_sdk_items: gtk::FilterListModel,
        /// The spinner shown at the end of the timeline, when loading events
        /// forwards.
        end_spinner_items: OnceCell<SingleItemListModel>,
        /// Items added at the end of the timeline.
        ///
        /// Currently this can only contain one item at a time.
        end_items: OnceCell<SingleItemListModel>,
        /// The `GListModel` containing all the timeline items.
        #[property(get = Self::items)]
        items: OnceCell<gtk::FlattenListModel>,
        /// A Hashmap linking a `TimelineEventItemId` to the corresponding
        /// `Event`.
        pub(super) event_map: RefCell<HashMap<TimelineEventItemId, Event>>,
        /// The loading state of the timeline.
        #[property(get, builder(LoadingState::default()))]
        state: Cell<LoadingState>,
        /// Whether this timeline is focused on a single event.
        ///
        /// If this is set, this is not the live timeline of the room: it is a
        /// timeline centered on a single event, e.g. a search result or a
        /// permalink. Such a timeline never receives new events from sync.
        #[property(get = Self::is_focused)]
        is_focused: PhantomData<bool>,
        /// Whether this timeline shows the events pinned in the room.
        ///
        /// Such a timeline is not live, and the SDK refuses to paginate it:
        /// the pinned events are the whole of it.
        #[property(get = Self::is_pinned)]
        is_pinned: PhantomData<bool>,
        /// Whether this timeline shows a single thread.
        ///
        /// Such a timeline holds only the events of that thread, starting at
        /// its root. It receives new thread events from sync and can be
        /// paginated backwards, but its bottom is the present, so it never
        /// paginates forwards.
        #[property(get = Self::is_thread)]
        is_thread: PhantomData<bool>,
        /// Whether we are loading events at the start of the timeline.
        #[property(get)]
        is_loading_start: Cell<bool>,
        /// Whether we are loading events at the end of the timeline.
        #[property(get)]
        is_loading_end: Cell<bool>,
        /// Whether the timeline is empty.
        #[property(get = Self::is_empty)]
        is_empty: PhantomData<bool>,
        /// Whether the timeline should be pre-loaded when it is ready.
        #[property(get, set = Self::set_preload, explicit_notify)]
        preload: Cell<bool>,
        /// Whether we have reached the start of the timeline.
        #[property(get)]
        has_reached_start: Cell<bool>,
        /// Whether we have reached the end of the timeline.
        ///
        /// This is always `true` for the live timeline, which is by definition
        /// at the end of the room's history.
        #[property(get)]
        has_reached_end: Cell<bool>,
        /// Whether we have the `m.room.create` event in the timeline.
        #[property(get)]
        has_room_create: Cell<bool>,
        /// The task following the core's timeline.
        watch_handle: RefCell<Option<AbortHandle>>,
        diff_handle: OnceCell<AbortHandle>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Timeline {
        const NAME: &'static str = "Timeline";
        type Type = super::Timeline;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Timeline {
        fn constructed(&self) {
            self.parent_constructed();

            self.filter.set_filter_func(clone!(
                #[weak(rename_to = imp)]
                self,
                #[upgrade_or]
                true,
                move |obj| {
                    // Hide the timeline start item if we have the `m.room.create` event too.
                    if let Some(item) = obj.downcast_ref::<VirtualItem>() {
                        return !(imp.has_room_create.get()
                            && item.kind() == VirtualItemKind::TimelineStart);
                    }

                    // Unparsable events arrive because an invalid
                    // `m.room.policy` is the unset gesture and draws as such;
                    // any other parse failure has nothing to say.
                    obj.downcast_ref::<Event>().is_none_or(|event| {
                        !event.failed_to_parse() || event.is_unparsed_policy_server_change()
                    })
                }
            ));
            self.filtered_sdk_items.set_filter(Some(&self.filter));
        }

        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
            if let Some(handle) = self.diff_handle.get() {
                handle.abort();
            }
        }
    }

    impl Timeline {
        /// Set the room containing this timeline.
        fn set_room(&self, room: Room) {
            let room = self.room.get_or_init(|| room);

            room.typing_list().connect_is_empty_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |list| {
                    if !list.is_empty() {
                        imp.add_typing_row();
                    }
                }
            ));
        }

        /// The room containing this timeline.
        fn room(&self) -> &Room {
            self.room.get().expect("room should be initialized")
        }

        /// Set the timeline, as the core keeps it.
        pub(super) fn set_core(&self, core: CoreTimeline) {
            self.core.set(core).expect("core should be uninitialized");
        }

        /// The timeline, as the core keeps it.
        pub(super) fn core(&self) -> &CoreTimeline {
            self.core.get().expect("core should be initialized")
        }

        /// Follow the core's timeline: build the SDK timeline through it,
        /// mirror its state and present its items.
        pub(super) async fn init_matrix_timeline(&self) {
            type T = super::Timeline;

            let core = self.core().clone();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe_state(), |obj: &T, state| {
                    obj.imp().set_state(state.into());
                })
                .follow(core.subscribe_is_loading_start(), |obj: &T, is_loading| {
                    obj.imp().set_loading_start(is_loading);
                })
                .follow(core.subscribe_is_loading_end(), |obj: &T, is_loading| {
                    obj.imp().set_loading_end(is_loading);
                })
                .follow(core.subscribe_has_reached_start(), |obj: &T, reached| {
                    obj.imp().set_has_reached_start(reached);
                })
                .follow(core.subscribe_has_reached_end(), |obj: &T, reached| {
                    obj.imp().set_has_reached_end(reached);
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            let core_clone = core.clone();
            let matrix_timeline = spawn_tokio!(async move { core_clone.matrix_timeline().await })
                .await
                .expect("task was not aborted");

            let Some(matrix_timeline) = matrix_timeline else {
                // Already logged by the core; its state says so too.
                self.set_state(LoadingState::Error);
                return;
            };

            self.matrix_timeline
                .set(matrix_timeline)
                .expect("matrix timeline is uninitialized");

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.set_has_reached_start(core.has_reached_start());
            self.set_has_reached_end(core.has_reached_end());

            let core_clone = core.clone();
            let subscription = spawn_tokio!(async move { core_clone.subscribe_items().await })
                .await
                .expect("task was not aborted");

            let Some((values, timeline_stream)) = subscription else {
                self.set_state(LoadingState::Error);
                return;
            };

            if *IS_AT_TRACE_LEVEL {
                tracing::trace!(
                    room = self.room().human_readable_id(),
                    items = ?sdk_items_to_log(&values),
                    "Initial timeline items",
                );
            }

            if !values.is_empty() {
                // Through the list update, so that `is-empty` is notified: the
                // core reports a focused timeline ready before its initial
                // events get here, and the view only leaves its spinner on that
                // notification. The backwards walk that used to hide this by
                // changing the state again no longer runs before the jump lands.
                self.update_with_diff_list(vec![VectorDiff::Append { values }]);
            }

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let room_id = self.room().room_id().to_owned();
            let fut = timeline_stream.for_each(move |diff_list| {
                let obj_weak = obj_weak.clone();
                let room_id = room_id.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.imp().update_with_diff_list(diff_list);
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
            self.diff_handle
                .set(diff_handle.abort_handle())
                .expect("handle should be uninitialized");

            if self.is_live() && self.preload.get() {
                self.preload().await;
            }

            self.set_state(core.state().into());
        }

        /// The event this timeline is focused on, if any.
        pub(super) fn focused_event_id(&self) -> Option<OwnedEventId> {
            match self.core().focus() {
                TimelineFocusKind::Event { target } => Some(target.clone()),
                _ => None,
            }
        }

        /// Whether this timeline is focused on a single event.
        fn is_focused(&self) -> bool {
            matches!(self.core().focus(), TimelineFocusKind::Event { .. })
        }

        /// Whether this timeline shows the events pinned in the room.
        fn is_pinned(&self) -> bool {
            matches!(self.core().focus(), TimelineFocusKind::Pinned)
        }

        /// The root event of the thread this timeline shows, if any.
        pub(super) fn thread_root(&self) -> Option<OwnedEventId> {
            match self.core().focus() {
                TimelineFocusKind::Thread { root } => Some(root.clone()),
                _ => None,
            }
        }

        /// Whether this timeline shows a single thread.
        fn is_thread(&self) -> bool {
            matches!(self.core().focus(), TimelineFocusKind::Thread { .. })
        }

        /// Whether this is the live timeline of the room.
        ///
        /// A live timeline is the only one that shows who is typing, or is
        /// preloaded. A thread timeline receives new events from sync too,
        /// but its receipts are the thread's, not the room's.
        fn is_live(&self) -> bool {
            matches!(self.core().focus(), TimelineFocusKind::Live)
        }

        /// The underlying SDK timeline.
        pub(super) fn matrix_timeline(&self) -> &Arc<SdkTimeline> {
            self.matrix_timeline
                .get()
                .expect("matrix timeline should be initialized")
        }

        /// Items added at the start of the timeline.
        fn start_items(&self) -> &SingleItemListModel {
            self.start_items.get_or_init(|| {
                let model = SingleItemListModel::new(Some(&VirtualItem::spinner(&self.obj())));
                model.set_is_hidden(true);
                model
            })
        }

        /// Items provided by the SDK timeline.
        pub(super) fn sdk_items(&self) -> &gio::ListStore {
            self.sdk_items.get_or_init(|| {
                let sdk_items = gio::ListStore::new::<TimelineItem>();
                self.filtered_sdk_items.set_model(Some(&sdk_items));
                sdk_items
            })
        }

        /// The spinner shown at the end of the timeline.
        fn end_spinner_items(&self) -> &SingleItemListModel {
            self.end_spinner_items.get_or_init(|| {
                let model = SingleItemListModel::new(Some(&VirtualItem::spinner_end(&self.obj())));
                model.set_is_hidden(true);
                model
            })
        }

        /// Items added at the end of the timeline.
        fn end_items(&self) -> &SingleItemListModel {
            self.end_items.get_or_init(|| {
                let model = SingleItemListModel::new(Some(&VirtualItem::typing(&self.obj())));
                model.set_is_hidden(true);
                model
            })
        }

        /// The `GListModel` containing all the timeline items.
        fn items(&self) -> gtk::FlattenListModel {
            self.items
                .get_or_init(|| {
                    let model_list = gio::ListStore::new::<gio::ListModel>();
                    model_list.append(self.start_items());
                    model_list.append(&self.filtered_sdk_items);
                    model_list.append(self.end_spinner_items());
                    model_list.append(self.end_items());
                    gtk::FlattenListModel::new(Some(model_list))
                })
                .clone()
        }

        /// Whether the timeline is empty.
        fn is_empty(&self) -> bool {
            self.filtered_sdk_items.n_items() == 0
        }

        /// Set the loading state of the timeline.
        fn set_state(&self, state: LoadingState) {
            if self.state.get() == state {
                return;
            }

            self.state.set(state);

            self.obj().notify_state();
        }

        /// Set whether we are loading events at the start of the timeline.
        fn set_loading_start(&self, is_loading_start: bool) {
            if self.is_loading_start.get() == is_loading_start {
                return;
            }

            self.is_loading_start.set(is_loading_start);

            self.start_items().set_is_hidden(!is_loading_start);
            self.obj().notify_is_loading_start();
        }

        /// Set whether we are loading events at the end of the timeline.
        fn set_loading_end(&self, is_loading_end: bool) {
            if self.is_loading_end.get() == is_loading_end {
                return;
            }

            self.is_loading_end.set(is_loading_end);

            self.end_spinner_items().set_is_hidden(!is_loading_end);
            self.obj().notify_is_loading_end();
        }

        /// Set whether we have reached the end of the timeline.
        fn set_has_reached_end(&self, has_reached_end: bool) {
            if self.has_reached_end.get() == has_reached_end {
                // Nothing to do.
                return;
            }

            self.has_reached_end.set(has_reached_end);

            self.obj().notify_has_reached_end();
        }

        /// Set whether we have reached the start of the timeline.
        fn set_has_reached_start(&self, has_reached_start: bool) {
            if self.has_reached_start.get() == has_reached_start {
                // Nothing to do.
                return;
            }

            self.has_reached_start.set(has_reached_start);

            self.obj().notify_has_reached_start();
        }

        /// Set whether the timeline has the `m.room.create` event of the room.
        fn set_has_room_create(&self, has_room_create: bool) {
            if self.has_room_create.get() == has_room_create {
                return;
            }

            self.has_room_create.set(has_room_create);

            let change = if has_room_create {
                gtk::FilterChange::MoreStrict
            } else {
                gtk::FilterChange::LessStrict
            };
            self.filter.changed(change);

            self.obj().notify_has_room_create();
        }

        /// Clear the state of the timeline.
        ///
        /// This doesn't handle removing items in `sdk_items` because it can be
        /// optimized by the caller of the function. What was known about the
        /// ends of the history is the core's to forget.
        fn clear(&self) {
            // `RefCell::take` releases the borrow before the retired events drop, unlike
            // `borrow_mut().clear()`.
            self.event_map.take();
            self.set_has_room_create(false);
        }

        /// Set whether the timeline should be pre-loaded when it is ready.
        fn set_preload(&self, preload: bool) {
            if self.preload.get() == preload {
                return;
            }

            self.preload.set(preload);
            self.obj().notify_preload();

            if preload && self.can_paginate_backwards() {
                spawn!(
                    glib::Priority::DEFAULT_IDLE,
                    clone!(
                        #[weak(rename_to = imp)]
                        self,
                        async move {
                            imp.preload().await;
                        }
                    )
                );
            }
        }

        /// Preload the timeline, if there are not enough items.
        async fn preload(&self) {
            if self.filtered_sdk_items.n_items() < u32::from(MAX_BATCH_SIZE) {
                self.paginate_backwards(|| ControlFlow::Break(())).await;
            }
        }

        /// Update this timeline with the given diff list.
        fn update_with_diff_list(&self, diff_list: Vec<VectorDiff<Arc<SdkTimelineItem>>>) {
            if *IS_AT_TRACE_LEVEL {
                self.log_diff_list(&diff_list);
            }

            let was_empty = self.is_empty();

            if let Some(diff_list) = self.try_minimize_diff_list(diff_list) {
                // The diff could not be minimized, handle it manually.
                for diff in diff_list {
                    self.update_with_single_diff(diff);
                }
            }

            if *IS_AT_TRACE_LEVEL {
                self.log_items();
            }

            if self.is_empty() != was_empty {
                self.obj().notify_is_empty();
            }
        }

        /// Attempt to minimize the given list of diffs.
        ///
        /// This is necessary because the SDK diffs are not always optimized,
        /// e.g. an item is removed then re-added, which creates jumps in the
        /// room history.
        ///
        /// Returns the list of diffs if it could not be minimized.
        fn try_minimize_diff_list(
            &self,
            diff_list: Vec<VectorDiff<Arc<SdkTimelineItem>>>,
        ) -> Option<Vec<VectorDiff<Arc<SdkTimelineItem>>>> {
            if !self.can_minimize_diff_list(&diff_list) {
                return Some(diff_list);
            }

            self.minimize_diff_list(diff_list);

            None
        }

        /// Update this timeline with the given diff.
        fn update_with_single_diff(&self, diff: VectorDiff<Arc<SdkTimelineItem>>) {
            match diff {
                VectorDiff::Append { values } => {
                    let new_list = values
                        .into_iter()
                        .map(|item| self.create_item(&item))
                        .collect::<Vec<_>>();

                    self.update_items(self.sdk_items().n_items(), 0, &new_list);
                }
                VectorDiff::Clear => {
                    self.sdk_items().remove_all();
                    self.clear();
                }
                VectorDiff::PushFront { value } => {
                    let item = self.create_item(&value);
                    self.update_items(0, 0, &[item]);
                }
                VectorDiff::PushBack { value } => {
                    let item = self.create_item(&value);
                    self.update_items(self.sdk_items().n_items(), 0, &[item]);
                }
                VectorDiff::PopFront => {
                    self.update_items(0, 1, &[]);
                }
                VectorDiff::PopBack => {
                    self.update_items(self.sdk_items().n_items().saturating_sub(1), 1, &[]);
                }
                VectorDiff::Insert { index, value } => {
                    let item = self.create_item(&value);
                    self.update_items(index as u32, 0, &[item]);
                }
                VectorDiff::Set { index, value } => {
                    let pos = index as u32;
                    let item = self
                        .item_at(pos)
                        .expect("there should be an item at the given position");

                    if item.timeline_id() == value.unique_id().0 {
                        // This is the same item, update it.
                        self.update_item(&item, &value);
                        // The header visibility might have changed.
                        self.update_items_headers(pos, 1);
                    } else {
                        let item = self.create_item(&value);
                        self.update_items(pos, 1, &[item]);
                    }
                }
                VectorDiff::Remove { index } => {
                    self.update_items(index as u32, 1, &[]);
                }
                VectorDiff::Truncate { length } => {
                    let length = length as u32;
                    let old_len = self.sdk_items().n_items();
                    self.update_items(length, old_len.saturating_sub(length), &[]);
                }
                VectorDiff::Reset { values } => {
                    // Reset the state.
                    self.clear();

                    let removed = self.sdk_items().n_items();
                    let new_list = values
                        .into_iter()
                        .map(|item| self.create_item(&item))
                        .collect::<Vec<_>>();

                    self.update_items(0, removed, &new_list);
                }
            }
        }

        /// Get the item at the given position.
        fn item_at(&self, pos: u32) -> Option<TimelineItem> {
            self.sdk_items().item(pos).and_downcast()
        }

        /// Update the items at the given position by removing the given number
        /// of items and adding the given items.
        fn update_items(&self, pos: u32, n_removals: u32, additions: &[TimelineItem]) {
            for i in pos..pos + n_removals {
                let Some(item) = self.item_at(i) else {
                    // This should not happen.
                    error!("Timeline item at position {i} not found");
                    break;
                };

                self.remove_item(&item);
            }

            self.sdk_items().splice(pos, n_removals, additions);

            // Update the header visibility of all the new additions, and the first item
            // after this batch.
            self.update_items_headers(pos, additions.len() as u32);
        }

        /// Update the headers of the item at the given position and the given
        /// number of items after it.
        fn update_items_headers(&self, pos: u32, nb: u32) {
            let sdk_items = self.sdk_items();

            let (mut previous_sender, mut previous_timestamp) = if pos > 0 {
                sdk_items
                    .item(pos - 1)
                    .and_downcast::<Event>()
                    .filter(Event::can_show_header)
                    .map(|event| (event.sender_id(), event.origin_server_ts()))
            } else {
                None
            }
            .unzip();

            // Update the headers of changed events plus the first event after them.
            for i in pos..=pos + nb {
                let Some(current) = self.item_at(i) else {
                    break;
                };
                let Ok(current) = current.downcast::<Event>() else {
                    previous_sender = None;
                    continue;
                };

                let current_sender = current.sender_id();

                if !current.can_show_header() {
                    current.set_header_state(EventHeaderState::Hidden);
                    previous_sender = None;
                    previous_timestamp = None;
                    continue;
                }

                let header_state = if previous_sender
                    .as_ref()
                    .is_none_or(|previous_sender| current_sender != *previous_sender)
                {
                    // The sender is different, show the full header.
                    EventHeaderState::Full
                } else if previous_timestamp
                    .and_then(|ts| current.origin_server_ts().0.checked_sub(ts.0))
                    .is_some_and(|elapsed| u64::from(elapsed) >= MAX_TIME_BETWEEN_HEADERS)
                {
                    // Too much time has passed, show the timestamp.
                    EventHeaderState::TimestampOnly
                } else {
                    // Do not show header.
                    EventHeaderState::Hidden
                };

                current.set_header_state(header_state);
                previous_sender = Some(current_sender);
                previous_timestamp = Some(current.origin_server_ts());
            }
        }

        /// Remove the given item from this `Timeline`.
        fn remove_item(&self, item: &TimelineItem) {
            if let Some(event) = item.downcast_ref::<Event>() {
                // `set_has_room_create` below emits `filter.changed`/`notify_has_room_create`,
                // so it must run after the `event_map` borrow is released.
                let removed_from_map = {
                    let mut event_map = self.event_map.borrow_mut();

                    // We need to remove both the transaction ID and the event ID.
                    let identifiers = event
                        .transaction_id()
                        .map(TimelineEventItemId::TransactionId)
                        .into_iter()
                        .chain(event.event_id().map(TimelineEventItemId::EventId));

                    let mut removed_from_map = false;
                    for id in identifiers {
                        // We check if we are removing the right event, in case we receive a diff
                        // that adds an existing event to another place, making us create a new
                        // event, before another diff that removes it from its old place, making
                        // us remove the old event.
                        let found = event_map.get(&id).is_some_and(|e| e == event);

                        if found {
                            event_map.remove(&id);
                            removed_from_map = true;
                        }
                    }

                    removed_from_map
                };

                if removed_from_map && event.is_room_create() {
                    self.set_has_room_create(false);
                }
            }
        }

        /// Whether we can load more events at the start of the timeline with
        /// the current state.
        pub(super) fn can_paginate_backwards(&self) -> bool {
            self.core().can_paginate_backwards()
        }

        /// Load more events at the start of the timeline until the given
        /// function tells us to stop.
        pub(super) async fn paginate_backwards<F>(&self, continue_fn: F)
        where
            F: Fn() -> ControlFlow<()>,
        {
            self.core()
                .paginate_backwards_while(|| continue_fn().is_continue())
                .await;
        }

        /// Whether we can load more events at the end of the timeline with the
        /// current state.
        pub(super) fn can_paginate_forwards(&self) -> bool {
            self.core().can_paginate_forwards()
        }

        /// Load more events at the end of the timeline until the given function
        /// tells us to stop.
        pub(super) async fn paginate_forwards<F>(&self, continue_fn: F)
        where
            F: Fn() -> ControlFlow<()>,
        {
            self.core()
                .paginate_forwards_while(|| continue_fn().is_continue())
                .await;
        }

        /// Add the typing row to the timeline, if it isn't present already.
        fn add_typing_row(&self) {
            if !self.is_live() {
                // Only the live timeline shows the typing status.
                return;
            }

            self.end_items().set_is_hidden(false);
        }

        /// Remove the typing row from the timeline.
        pub(super) fn remove_empty_typing_row(&self) {
            if !self.room().typing_list().is_empty() {
                return;
            }

            self.end_items().set_is_hidden(true);
        }
    }

    impl TimelineDiffItemStore for Timeline {
        type Item = TimelineItem;
        type Data = Arc<SdkTimelineItem>;

        fn items(&self) -> Vec<TimelineItem> {
            self.sdk_items()
                .snapshot()
                .into_iter()
                .map(|obj| {
                    obj.downcast::<TimelineItem>()
                        .expect("SDK items are TimelineItems")
                })
                .collect()
        }

        fn create_item(&self, data: &Arc<SdkTimelineItem>) -> TimelineItem {
            let item = TimelineItem::new(data, &self.obj());

            if let Some(event) = item.downcast_ref::<Event>() {
                self.event_map
                    .borrow_mut()
                    .insert(event.identifier(), event.clone());

                // Keep track of the activity of the sender.
                if event.counts_as_unread()
                    && let Some(members) = self.room().members()
                {
                    let member = members.get_or_create(event.sender_id());
                    member.set_latest_activity(u64::from(event.origin_server_ts().get()));
                }

                if event.is_room_create() {
                    self.set_has_room_create(true);
                }
            }

            item
        }

        fn update_item(&self, item: &TimelineItem, data: &Arc<SdkTimelineItem>) {
            item.update_with(data);

            if let Some(event) = item.downcast_ref::<Event>() {
                // Update the identifier in the event map, in case we switched from a
                // transaction ID to an event ID.
                self.event_map
                    .borrow_mut()
                    .insert(event.identifier(), event.clone());
            }
        }

        fn apply_item_diff_list(&self, item_diff_list: Vec<TimelineDiff<TimelineItem>>) {
            for item_diff in item_diff_list {
                match item_diff {
                    TimelineDiff::Splice(splice) => {
                        self.update_items(splice.pos, splice.n_removals, &splice.additions);
                    }
                    TimelineDiff::Update(update) => {
                        self.update_items_headers(update.pos, update.n_items);
                    }
                }
            }
        }
    }

    /// The default log filter initialized with the `RUST_LOG` environment
    /// variable.
    ///
    /// Used to know if we are likely to need to log the diff.
    static IS_AT_TRACE_LEVEL: LazyLock<bool> = LazyLock::new(|| {
        tracing_subscriber::EnvFilter::try_from_default_env()
            // If the env variable is not set, we know that we are not at trace level.
            .ok()
            .and_then(|filter| filter.max_level_hint())
            .is_some_and(|max| max == tracing::level_filters::LevelFilter::TRACE)
    });

    /// Temporary methods to debug items in the timeline.
    impl Timeline {
        /// Log the given diff list.
        fn log_diff_list(&self, diff_list: &[VectorDiff<Arc<SdkTimelineItem>>]) {
            let mut log_list = Vec::with_capacity(diff_list.len());

            for diff in diff_list {
                let log = match diff {
                    VectorDiff::Append { values } => {
                        format!("append: {:?}", sdk_items_to_log(values))
                    }
                    VectorDiff::Clear => "clear".to_owned(),
                    VectorDiff::PushFront { value } => {
                        format!("push_front: {}", sdk_item_to_log(value))
                    }
                    VectorDiff::PushBack { value } => {
                        format!("push_back: {}", sdk_item_to_log(value))
                    }
                    VectorDiff::PopFront => "pop_front".to_owned(),
                    VectorDiff::PopBack => "pop_back".to_owned(),
                    VectorDiff::Insert { index, value } => {
                        format!("insert at {index}: {}", sdk_item_to_log(value))
                    }
                    VectorDiff::Set { index, value } => {
                        format!("set at {index}: {}", sdk_item_to_log(value))
                    }
                    VectorDiff::Remove { index } => format!("remove at {index}"),
                    VectorDiff::Truncate { length } => format!("truncate at {length}"),
                    VectorDiff::Reset { values } => {
                        format!("reset: {:?}", sdk_items_to_log(values))
                    }
                };

                log_list.push(log);
            }

            tracing::trace!(
                room = self.room().human_readable_id(),
                "Diff list: {log_list:#?}"
            );
        }

        /// Log the items in this timeline.
        fn log_items(&self) {
            let items = self
                .sdk_items()
                .iter::<TimelineItem>()
                .filter_map(|item| item.as_ref().map(item_to_log).ok())
                .collect::<Vec<_>>();

            tracing::trace!(
                room = self.room().human_readable_id(),
                "Timeline: {items:#?}"
            );
        }
    }

    // Helper methods for logging items.
    fn sdk_items_to_log(
        items: &matrix_sdk_ui::eyeball_im::Vector<Arc<SdkTimelineItem>>,
    ) -> Vec<String> {
        items.iter().map(|item| sdk_item_to_log(item)).collect()
    }

    fn sdk_item_to_log(item: &SdkTimelineItem) -> String {
        match item.kind() {
            matrix_sdk_ui::timeline::TimelineItemKind::Event(event) => {
                format!("event::{:?}", event.identifier())
            }
            matrix_sdk_ui::timeline::TimelineItemKind::Virtual(virtual_item) => {
                format!("virtual::{virtual_item:?}")
            }
        }
    }

    fn item_to_log(item: &TimelineItem) -> String {
        if let Some(virtual_item) = item.downcast_ref::<VirtualItem>() {
            format!("virtual::{:?}", virtual_item.kind())
        } else if let Some(event) = item.downcast_ref::<Event>() {
            format!("event::{:?}", event.identifier())
        } else {
            "Unknown item".to_owned()
        }
    }
}

glib::wrapper! {
    /// All loaded items in a room.
    ///
    /// There is no strict message ordering enforced by the Timeline; items
    /// will be appended/prepended to existing items in the order they are
    /// received by the server. The timeline is the core's; this presents its
    /// items as `GObject`s, with the headers, the virtual rows and the diff
    /// minimizing a `GListModel` wants.
    pub struct Timeline(ObjectSubclass<imp::Timeline>);
}

impl Timeline {
    /// Construct a new `Timeline` for the given room.
    pub(crate) fn new(room: &Room) -> Self {
        Self::construct(room, room.core().live_timeline())
    }

    /// Construct a new `Timeline` for the given room, focused on the event with
    /// the given ID.
    ///
    /// Such a timeline is centered on a single event and can be paginated in
    /// both directions, but it never receives new events from sync, so it
    /// cannot replace the live timeline of the room.
    pub(crate) fn new_focused(room: &Room, event_id: OwnedEventId) -> Self {
        Self::construct(room, room.core().focused_timeline(event_id))
    }

    /// Construct a new `Timeline` showing the events pinned in the given room.
    ///
    /// The SDK builds this one from the room's `m.room.pinned_events`, so it
    /// follows that state event as it changes. It cannot be paginated — the
    /// pinned events are the whole of it — and it never receives new events
    /// from sync.
    pub(crate) fn new_pinned(room: &Room) -> Self {
        Self::construct(room, room.core().pinned_timeline())
    }

    /// Construct a new `Timeline` showing the thread rooted at the event with
    /// the given ID.
    ///
    /// Such a timeline holds the thread and nothing else, starting at its
    /// root. It receives new thread events from sync, so its bottom is the
    /// present; older thread events are loaded by paginating backwards.
    /// Anything sent through it carries the thread relation, and a read
    /// receipt sent through it is the thread's, not the room's.
    pub(crate) fn new_threaded(room: &Room, root_event_id: OwnedEventId) -> Self {
        Self::construct(room, room.core().thread_timeline(root_event_id))
    }

    /// Construct a new `Timeline` for the given room, presenting the given
    /// timeline of the core.
    fn construct(room: &Room, core: CoreTimeline) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .build();

        let imp = obj.imp();
        imp.set_core(core);

        spawn!(glib::clone!(
            #[weak]
            imp,
            async move {
                imp.init_matrix_timeline().await;
            }
        ));

        obj
    }

    /// The event this timeline is focused on, if any.
    pub(crate) fn focused_event_id(&self) -> Option<OwnedEventId> {
        self.imp().focused_event_id()
    }

    /// The root event of the thread this timeline shows, if any.
    pub(crate) fn thread_root(&self) -> Option<OwnedEventId> {
        self.imp().thread_root()
    }

    /// Send the given receipt through this timeline.
    ///
    /// The SDK scopes the receipt to what the timeline shows: sent through a
    /// thread timeline, it is a receipt for that thread, not for the room.
    pub(crate) async fn send_receipt(
        &self,
        receipt_type: ApiReceiptType,
        position: ReceiptPosition,
    ) {
        let Some(session) = self.room().session() else {
            return;
        };
        let send_public_receipt = session.settings().public_read_receipts_enabled();

        let receipt_type = match receipt_type {
            ApiReceiptType::Read if !send_public_receipt => ApiReceiptType::ReadPrivate,
            t => t,
        };

        let core = self.imp().core().clone();
        spawn_tokio!(async move {
            core.send_receipt_resolved(receipt_type, position.into())
                .await;
        })
        .await
        .expect("task was not aborted");
    }

    /// The underlying SDK timeline.
    pub(crate) fn matrix_timeline(&self) -> Arc<SdkTimeline> {
        self.imp().matrix_timeline().clone()
    }

    /// Load more events at the start of the timeline until the given function
    /// tells us to stop.
    pub(crate) async fn paginate_backwards<F>(&self, continue_fn: F)
    where
        F: Fn() -> ControlFlow<()>,
    {
        let imp = self.imp();

        if !imp.can_paginate_backwards() {
            return;
        }

        imp.paginate_backwards(continue_fn).await;
    }

    /// Load more events at the end of the timeline until the given function
    /// tells us to stop.
    pub(crate) async fn paginate_forwards<F>(&self, continue_fn: F)
    where
        F: Fn() -> ControlFlow<()>,
    {
        let imp = self.imp();

        if !imp.can_paginate_forwards() {
            return;
        }

        imp.paginate_forwards(continue_fn).await;
    }

    /// Get the event with the given identifier from this `Timeline`.
    ///
    /// Use this method if you are sure the event has already been received.
    /// Otherwise use `fetch_event_by_id`.
    pub(crate) fn event_by_identifier(&self, identifier: &TimelineEventItemId) -> Option<Event> {
        self.imp().event_map.borrow().get(identifier).cloned()
    }

    /// Whether this `Timeline` holds the event with the given identifier.
    ///
    /// A lookup in the map of events, so it costs the same however long the
    /// timeline is, unlike [`Self::find_event_position()`].
    pub(crate) fn has_event(&self, identifier: &TimelineEventItemId) -> bool {
        self.imp().event_map.borrow().contains_key(identifier)
    }

    /// Get the position of the event with the given identifier in this
    /// `Timeline`.
    ///
    /// This walks the items, so it is for the one-off scroll to an event that
    /// is known to be there; [`Self::has_event()`] answers whether it is.
    pub(crate) fn find_event_position(&self, identifier: &TimelineEventItemId) -> Option<usize> {
        if !self.has_event(identifier) {
            return None;
        }

        self.items()
            .iter::<glib::Object>()
            .enumerate()
            .find_map(|(index, item)| {
                item.ok()
                    .and_downcast::<Event>()
                    .is_some_and(|event| event.matches_identifier(identifier))
                    .then_some(index)
            })
    }

    /// Remove the typing row from the timeline.
    pub(crate) fn remove_empty_typing_row(&self) {
        self.imp().remove_empty_typing_row();
    }

    /// The IDs of redactable events sent by the given user in this timeline.
    pub(crate) fn redactable_events_for(&self, user_id: &UserId) -> Vec<OwnedEventId> {
        let mut events = vec![];

        for item in self.imp().sdk_items().iter::<glib::Object>() {
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

            if event.can_be_redacted()
                && let Some(event_id) = event.event_id()
            {
                events.push(event_id);
            }
        }

        events
    }
}

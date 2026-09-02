use commune_core::session::ThreadList as CoreThreadList;
use futures_util::StreamExt;
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk_ui::{
    eyeball_im::VectorDiff,
    timeline::{
        MsgLikeKind, TimelineItemContent,
        thread_list_service::{ThreadListItem, ThreadListItemEvent},
    },
};
use ruma::OwnedEventId;
use tokio::task::AbortHandle;

use super::{Member, Room};
use crate::{
    core_bridge::ObjectWatcher,
    spawn, spawn_tokio,
    utils::{LoadingState, matrix::timestamp_to_date},
};

/// A one-line preview of the given content, for a thread list row.
///
/// The list has no room for the real widgets, so everything becomes a
/// sentence: a message keeps its body, everything else says what it is.
fn content_preview(content: Option<&TimelineItemContent>) -> String {
    match content {
        Some(TimelineItemContent::MsgLike(msg_like)) => match &msg_like.kind {
            MsgLikeKind::Message(message) => message.msgtype().body().to_owned(),
            MsgLikeKind::Sticker(sticker) => sticker.content().body.clone(),
            MsgLikeKind::Redacted => gettext("This message was removed."),
            MsgLikeKind::UnableToDecrypt(_) => gettext("Could not decrypt this message"),
            _ => gettext("Unsupported event"),
        },
        _ => gettext("Unsupported event"),
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ThreadList)]
    pub struct ThreadList {
        /// The room the threads of which are listed.
        #[property(get, set = Self::set_room, construct_only)]
        room: OnceCell<Room>,
        /// The list, as the core loads it.
        core: OnceCell<CoreThreadList>,
        /// The list of threads.
        #[property(get = Self::list_owned)]
        list: OnceCell<gio::ListStore>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// Whether the whole thread list was loaded.
        #[property(get)]
        has_reached_end: Cell<bool>,
        /// The task following the core's state.
        watch_handle: RefCell<Option<AbortHandle>>,
        /// The handle watching the core's items for live updates.
        diff_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThreadList {
        const NAME: &'static str = "ThreadList";
        type Type = super::ThreadList;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ThreadList {
        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
            if let Some(handle) = self.diff_handle.take() {
                handle.abort();
            }
        }
    }

    impl ThreadList {
        /// Set the room the threads of which are listed.
        fn set_room(&self, room: Room) {
            type L = super::ThreadList;

            let room = self.room.get_or_init(|| room);

            let core = self.core.get_or_init(|| room.core().thread_list()).clone();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe_loading_state(), |obj: &L, state| {
                    obj.imp().set_loading_state(state.into());
                })
                .follow(
                    core.subscribe_has_reached_end(),
                    |obj: &L, has_reached_end| {
                        obj.imp().set_has_reached_end(has_reached_end);
                    },
                )
                .spawn();
            self.watch_handle.replace(Some(handle));
        }

        /// The room the threads of which are listed.
        fn room(&self) -> &Room {
            self.room.get().expect("room should be initialized")
        }

        /// The list, as the core loads it.
        pub(super) fn core(&self) -> &CoreThreadList {
            self.core.get().expect("core list should be initialized")
        }

        /// The list of threads.
        pub(super) fn list(&self) -> &gio::ListStore {
            self.list
                .get_or_init(gio::ListStore::new::<super::ThreadListEntry>)
        }

        /// The owned list of threads.
        fn list_owned(&self) -> gio::ListStore {
            self.list().clone()
        }

        /// Whether the list is empty.
        pub(super) fn is_empty(&self) -> bool {
            self.list().n_items() == 0
        }

        /// Set the loading state of the list.
        fn set_loading_state(&self, state: LoadingState) {
            if self.loading_state.get() == state {
                return;
            }

            self.loading_state.set(state);
            self.obj().notify_loading_state();
        }

        /// Set whether the whole thread list was loaded.
        fn set_has_reached_end(&self, has_reached_end: bool) {
            if self.has_reached_end.get() == has_reached_end {
                return;
            }

            self.has_reached_end.set(has_reached_end);
            self.obj().notify_has_reached_end();
        }

        /// Whether more threads can be loaded.
        pub(super) fn can_load_more(&self) -> bool {
            self.core().can_load_more()
        }

        /// Follow the core's items, if they are not followed already.
        ///
        /// The service appends pages as they are fetched and rewrites an item
        /// in place when a new thread event arrives from sync; both reach the
        /// `GListModel` through here.
        async fn ensure_watching_items(&self) {
            if self.diff_handle.borrow().is_some() {
                return;
            }

            let core = self.core().clone();
            let (initial, stream) = spawn_tokio!(async move { core.subscribe_items().await })
                .await
                .expect("task was not aborted");

            if self.diff_handle.borrow().is_some() {
                // Subscribed twice concurrently; keep the first, whose items
                // are already being watched.
                return;
            }

            if !initial.is_empty() {
                self.apply_diff(VectorDiff::Append { values: initial });
            }

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let fut = stream.for_each(move |diff_list| {
                let obj_weak = obj_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                for diff in diff_list {
                                    obj.imp().apply_diff(diff);
                                }
                            }
                        });
                    });
                }
            });

            let diff_handle = spawn_tokio!(fut);
            self.diff_handle.replace(Some(diff_handle.abort_handle()));
        }

        /// Apply the given diff to the list.
        fn apply_diff(&self, diff: VectorDiff<ThreadListItem>) {
            let room = self.room().clone();
            let list = self.list();
            let entry = |item: &ThreadListItem| super::ThreadListEntry::new(&room, item.clone());

            match diff {
                VectorDiff::Append { values } => {
                    let entries = values.iter().map(entry).collect::<Vec<_>>();
                    list.extend_from_slice(&entries);
                }
                VectorDiff::PushBack { value } => {
                    list.append(&entry(&value));
                }
                VectorDiff::PushFront { value } => {
                    list.insert(0, &entry(&value));
                }
                VectorDiff::Insert { index, value } => {
                    list.insert(index as u32, &entry(&value));
                }
                VectorDiff::Set { index, value } => {
                    list.splice(index as u32, 1, &[entry(&value)]);
                }
                VectorDiff::Remove { index } => {
                    list.remove(index as u32);
                }
                VectorDiff::PopBack => {
                    if list.n_items() > 0 {
                        list.remove(list.n_items() - 1);
                    }
                }
                VectorDiff::PopFront => {
                    if list.n_items() > 0 {
                        list.remove(0);
                    }
                }
                VectorDiff::Truncate { length } => {
                    let length = length as u32;
                    while list.n_items() > length {
                        list.remove(list.n_items() - 1);
                    }
                }
                VectorDiff::Clear => {
                    list.remove_all();
                }
                VectorDiff::Reset { values } => {
                    let entries = values.iter().map(entry).collect::<Vec<_>>();
                    list.splice(0, list.n_items(), &entries);
                }
            }
        }

        /// Load the next page of threads.
        pub(super) async fn load(&self) {
            self.ensure_watching_items().await;

            let core = self.core().clone();
            // A failure is logged by the core, whose state says so.
            let _ = spawn_tokio!(async move { core.load_more().await })
                .await
                .expect("task was not aborted");
        }
    }
}

glib::wrapper! {
    /// The list of threads of a room.
    ///
    /// It is loaded from the `/threads` endpoint page by page, most recent
    /// activity first, and the SDK keeps the reply count and latest event of
    /// each listed thread current as new thread events arrive from sync. The
    /// list is the core's; this presents it.
    pub struct ThreadList(ObjectSubclass<imp::ThreadList>);
}

impl ThreadList {
    /// Construct a new `ThreadList` for the given room.
    pub(crate) fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// Whether the list is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.imp().is_empty()
    }

    /// Whether more threads can be loaded.
    pub(crate) fn can_load_more(&self) -> bool {
        self.imp().can_load_more()
    }

    /// Load more threads.
    pub(crate) fn load_more(&self) {
        let imp = self.imp();

        if !imp.can_load_more() {
            return;
        }

        spawn!(clone!(
            #[weak]
            imp,
            async move {
                imp.load().await;
            }
        ));
    }
}

mod entry_imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ThreadListEntry)]
    pub struct ThreadListEntry {
        /// The room containing this thread.
        #[property(get, construct_only)]
        pub(super) room: glib::WeakRef<Room>,
        /// The SDK item.
        item: OnceCell<ThreadListItem>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThreadListEntry {
        const NAME: &'static str = "ThreadListEntry";
        type Type = super::ThreadListEntry;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ThreadListEntry {}

    impl ThreadListEntry {
        /// Set the SDK item.
        pub(super) fn set_item(&self, item: ThreadListItem) {
            self.item.set(item).expect("item should be uninitialized");
        }

        /// The SDK item.
        pub(super) fn item(&self) -> &ThreadListItem {
            self.item.get().expect("item should be initialized")
        }
    }
}

glib::wrapper! {
    /// A thread in the list of threads of a room.
    pub struct ThreadListEntry(ObjectSubclass<entry_imp::ThreadListEntry>);
}

impl ThreadListEntry {
    /// Construct a new `ThreadListEntry` for the given thread.
    fn new(room: &Room, item: ThreadListItem) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .build();
        obj.imp().set_item(item);
        obj
    }

    /// The member for the sender of the given event.
    fn member_for(&self, event: &ThreadListItemEvent) -> Option<Member> {
        let room = self.room()?;
        Some(
            room.get_or_create_members()
                .get_or_create(event.sender.clone()),
        )
    }

    /// The ID of the root event of this thread.
    pub(crate) fn root_event_id(&self) -> OwnedEventId {
        self.imp().item().root_event.event_id.clone()
    }

    /// The sender of the root event of this thread.
    pub(crate) fn sender(&self) -> Option<Member> {
        self.member_for(&self.imp().item().root_event)
    }

    /// The timestamp of the root event of this thread, as a `GDateTime`.
    pub(crate) fn timestamp(&self) -> glib::DateTime {
        timestamp_to_date(self.imp().item().root_event.timestamp)
    }

    /// A one-line preview of the root event of this thread.
    pub(crate) fn body(&self) -> String {
        content_preview(self.imp().item().root_event.content.as_ref())
    }

    /// The number of replies in this thread.
    pub(crate) fn num_replies(&self) -> u32 {
        self.imp().item().num_replies
    }

    /// The sender of the latest reply in this thread, if it is known.
    pub(crate) fn latest_sender(&self) -> Option<Member> {
        let imp = self.imp();
        self.member_for(imp.item().latest_event.as_ref()?)
    }

    /// A one-line preview of the latest reply in this thread, if it is known.
    pub(crate) fn latest_body(&self) -> Option<String> {
        let imp = self.imp();
        let latest = imp.item().latest_event.as_ref()?;
        Some(content_preview(latest.content.as_ref()))
    }
}

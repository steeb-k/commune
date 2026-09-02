use commune_core::session::{RoomSearch as CoreRoomSearch, SearchResult};
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{OwnedEventId, OwnedUserId};
use tokio::task::AbortHandle;

use super::{Member, Room};
use crate::{
    core_bridge::ObjectWatcher,
    spawn, spawn_tokio,
    utils::{LoadingState, matrix::timestamp_to_date},
};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomSearch)]
    pub struct RoomSearch {
        /// The room to search in.
        #[property(get, set = Self::set_room, construct_only)]
        room: OnceCell<Room>,
        /// The search, as the core runs it.
        core: OnceCell<CoreRoomSearch>,
        /// The list of results for the current search.
        #[property(get = Self::list_owned)]
        list: OnceCell<gio::ListStore>,
        /// The term that is currently searched.
        #[property(get)]
        search_term: RefCell<String>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// Whether all the results of the current search were loaded.
        #[property(get)]
        has_reached_end: Cell<bool>,
        /// The members of the room.
        ///
        /// A strong reference is kept so that all results use the same list.
        room_members: RefCell<Option<crate::session::MemberList>>,
        /// The abort handle for the current request.
        abort_handle: RefCell<Option<AbortHandle>>,
        /// The task following the core's loading state.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomSearch {
        const NAME: &'static str = "RoomSearch";
        type Type = super::RoomSearch;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomSearch {
        fn dispose(&self) {
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl RoomSearch {
        /// Set the room to search in.
        fn set_room(&self, room: Room) {
            let room = self.room.get_or_init(|| room);
            self.room_members
                .replace(Some(room.get_or_create_members()));

            let core = self
                .core
                .get_or_init(|| CoreRoomSearch::new(room.core()))
                .clone();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(
                    core.subscribe_loading_state(),
                    |obj: &super::RoomSearch, state| {
                        obj.imp().set_loading_state(state.into());
                    },
                )
                .spawn();
            self.watch_handle.replace(Some(handle));
        }

        /// The room to search in.
        fn room(&self) -> &Room {
            self.room.get().expect("room should be initialized")
        }

        /// The search, as the core runs it.
        pub(super) fn core(&self) -> &CoreRoomSearch {
            self.core.get().expect("core search should be initialized")
        }

        /// The list of results for the current search.
        fn list(&self) -> &gio::ListStore {
            self.list
                .get_or_init(gio::ListStore::new::<super::RoomSearchResult>)
        }

        /// The owned list of results for the current search.
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

        /// Mirror whether all the results of the current search were loaded.
        fn update_has_reached_end(&self) {
            let has_reached_end = self.core().has_reached_end();

            if self.has_reached_end.get() == has_reached_end {
                return;
            }

            self.has_reached_end.set(has_reached_end);
            self.obj().notify_has_reached_end();
        }

        /// Set the term to search.
        pub(super) fn set_search_term(&self, search_term: &str) {
            let search_term = search_term.trim().to_owned();

            if *self.search_term.borrow() == search_term {
                return;
            }

            self.core().set_search_term(&search_term);
            self.search_term.replace(search_term);
            self.obj().notify_search_term();

            self.restart_search();
        }

        /// Present the current term's search again, from scratch.
        ///
        /// The core has already dropped what it had; this drops the rows and
        /// asks for the first page.
        fn restart_search(&self) {
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }

            self.list().remove_all();
            self.update_has_reached_end();

            if self.search_term.borrow().is_empty() {
                return;
            }

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }

        /// Add the messages that are loaded in the room to its local search
        /// index, then search again.
        pub(super) async fn reindex(&self) {
            let core = self.core().clone();
            let handle = spawn_tokio!(async move { core.reindex().await });

            if handle.await.expect("task was not aborted").is_err() {
                // Already logged by the core; its state says so too.
                return;
            }

            // Search again, so that the messages that were just added are
            // presented.
            self.core().set_search_term("");
            let search_term = self.search_term.borrow().clone();
            self.core().set_search_term(&search_term);
            self.restart_search();
        }

        /// Whether we can load more results with the current search.
        pub(super) fn can_load_more(&self) -> bool {
            self.core().can_load_more()
        }

        /// Load more results.
        pub(super) async fn load(&self) {
            let core = self.core().clone();
            let handle = spawn_tokio!(async move { core.load_more().await });
            self.abort_handle.replace(Some(handle.abort_handle()));

            let Ok(result) = handle.await else {
                // The request was aborted.
                self.abort_handle.take();
                return;
            };

            self.abort_handle.take();

            // A page that is not the current search's comes back empty, and
            // a failure is logged by the core, whose state says so.
            let Ok(results) = result else {
                return;
            };

            let room = self.room();
            let results = results
                .into_iter()
                .map(|result| super::RoomSearchResult::new(room, result))
                .collect::<Vec<_>>();

            self.list().extend_from_slice(&results);
            self.update_has_reached_end();
        }
    }
}

glib::wrapper! {
    /// The search of the messages of a room.
    ///
    /// Messages of an encrypted room can only be searched in the local search
    /// index, since the server cannot read their content. Other rooms are
    /// searched on the server, which knows the whole history of the room. The
    /// search is the core's; this presents its results.
    pub struct RoomSearch(ObjectSubclass<imp::RoomSearch>);
}

impl RoomSearch {
    /// Construct a new `RoomSearch` for the given room.
    pub(crate) fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// Whether the list of results is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.imp().is_empty()
    }

    /// Set the term to search.
    ///
    /// This restarts the search from scratch.
    pub(crate) fn set_search_term(&self, search_term: &str) {
        self.imp().set_search_term(search_term);
    }

    /// Whether more results can be loaded with the current search.
    pub(crate) fn can_load_more(&self) -> bool {
        self.imp().can_load_more()
    }

    /// Load more results.
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

    /// Add the messages that are loaded in the room to its local search index,
    /// then search again.
    pub(crate) fn reindex(&self) {
        if self.loading_state() == LoadingState::Loading {
            return;
        }

        let imp = self.imp();

        spawn!(clone!(
            #[weak]
            imp,
            async move {
                imp.reindex().await;
            }
        ));
    }
}

mod result_imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomSearchResult)]
    pub struct RoomSearchResult {
        /// The room containing this message.
        #[property(get, construct_only)]
        pub(super) room: glib::WeakRef<Room>,
        /// The result, as the core found it.
        core: OnceCell<SearchResult>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomSearchResult {
        const NAME: &'static str = "RoomSearchResult";
        type Type = super::RoomSearchResult;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomSearchResult {}

    impl RoomSearchResult {
        /// Set the result this presents.
        pub(super) fn set_result(&self, result: SearchResult) {
            self.core
                .set(result)
                .expect("result should be uninitialized");
        }

        /// The result, as the core found it.
        pub(super) fn core(&self) -> &SearchResult {
            self.core.get().expect("result should be initialized")
        }
    }
}

glib::wrapper! {
    /// A message matching a search in a room.
    pub struct RoomSearchResult(ObjectSubclass<result_imp::RoomSearchResult>);
}

impl RoomSearchResult {
    /// Construct a new `RoomSearchResult` for the given result of the core.
    fn new(room: &Room, result: SearchResult) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .build();
        obj.imp().set_result(result);
        obj
    }

    /// The ID of the event of this result.
    pub(crate) fn event_id(&self) -> OwnedEventId {
        self.imp().core().event_id()
    }

    /// The ID of the sender of this message.
    pub(crate) fn sender_id(&self) -> OwnedUserId {
        self.imp().core().sender_id()
    }

    /// The sender of this message.
    pub(crate) fn sender(&self) -> Option<Member> {
        let room = self.room()?;
        Some(room.get_or_create_members().get_or_create(self.sender_id()))
    }

    /// The timestamp of this message, as a `GDateTime`.
    pub(crate) fn timestamp(&self) -> glib::DateTime {
        timestamp_to_date(self.imp().core().timestamp())
    }

    /// The textual content of this message.
    pub(crate) fn body(&self) -> String {
        self.imp().core().body()
    }
}

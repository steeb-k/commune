use std::collections::HashMap;

use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{
    OwnedRoomId, OwnedServerName, RoomId, api::client::space::get_hierarchy, assign,
    events::space::child::HierarchySpaceChildEvent, serde::Raw, uint,
};
use tokio::task::AbortHandle;
use tracing::{error, warn};

use super::RemoteRoom;
use crate::{
    session::Session,
    spawn, spawn_tokio,
    utils::{LoadingState, matrix::MatrixRoomIdUri},
};

/// The maximum number of rooms to ask for at a time.
const BATCH_SIZE: u32 = 20;

/// The maximum number of batches to walk through for one space.
///
/// The endpoint paginates and a space can hold thousands of rooms, so
/// something has to stop. When this is reached the list says so rather than
/// pretending to be complete.
const MAX_BATCHES: usize = 10;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SpaceChildren)]
    pub struct SpaceChildren {
        /// The rooms that are inside the space.
        #[property(get = Self::list_owned)]
        list: OnceCell<gio::ListStore>,
        /// The session to make the requests with.
        session: glib::WeakRef<Session>,
        /// The ID of the space these rooms are inside.
        room_id: RefCell<Option<OwnedRoomId>>,
        /// The token to continue the listing with, if there is more of it.
        next_batch: RefCell<Option<String>>,
        /// The servers to try for each room, from the space's `m.space.child`
        /// events.
        via: RefCell<HashMap<OwnedRoomId, Vec<OwnedServerName>>>,
        /// Whether the listing stopped before the end of the space.
        #[property(get)]
        is_truncated: Cell<bool>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// The abort handle for the current request.
        abort_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpaceChildren {
        const NAME: &'static str = "SpaceChildren";
        type Type = super::SpaceChildren;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SpaceChildren {}

    impl SpaceChildren {
        /// The rooms that are inside the space.
        fn list(&self) -> &gio::ListStore {
            self.list.get_or_init(gio::ListStore::new::<RemoteRoom>)
        }

        /// The owned list of rooms that are inside the space.
        fn list_owned(&self) -> gio::ListStore {
            self.list().clone()
        }

        /// Set the space to list the rooms of.
        pub(super) fn set_space(&self, session: &Session, room_id: OwnedRoomId) {
            if self.room_id.borrow().as_ref() == Some(&room_id) {
                return;
            }

            self.session.set(Some(session));
            self.room_id.replace(Some(room_id));

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }

        /// Set the loading state.
        fn set_loading_state(&self, state: LoadingState) {
            if self.loading_state.get() == state {
                return;
            }

            self.loading_state.set(state);
            self.obj().notify_loading_state();
        }

        /// Set whether the listing stopped before the end of the space.
        fn set_is_truncated(&self, is_truncated: bool) {
            if self.is_truncated.get() == is_truncated {
                return;
            }

            self.is_truncated.set(is_truncated);
            self.obj().notify_is_truncated();
        }

        /// List the rooms inside the current space, from the beginning.
        pub(super) async fn load(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let Some(room_id) = self.room_id.borrow().clone() else {
                return;
            };

            // Abort any listing of a previous space and forget what it found.
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }
            self.list().remove_all();
            self.next_batch.take();
            self.via.borrow_mut().clear();
            self.set_is_truncated(false);
            self.set_loading_state(LoadingState::Loading);

            for _ in 0..MAX_BATCHES {
                if !self.load_batch(&session, &room_id).await {
                    // The request failed, was aborted, or the space changed.
                    return;
                }

                if self.next_batch.borrow().is_none() {
                    self.set_loading_state(LoadingState::Ready);
                    return;
                }
            }

            warn!("Stopped listing the rooms in space `{room_id}` after {MAX_BATCHES} batches");
            self.set_is_truncated(true);
            self.set_loading_state(LoadingState::Ready);
        }

        /// Request one batch of rooms inside the given space and add them to
        /// the list.
        ///
        /// Returns `false` if there is no point asking for another one.
        async fn load_batch(&self, session: &Session, room_id: &RoomId) -> bool {
            let from = self.next_batch.borrow().clone();
            let request = assign!(get_hierarchy::v1::Request::new(room_id.to_owned()), {
                from,
                limit: Some(BATCH_SIZE.into()),
                // The space itself and the rooms directly inside it, and no
                // deeper: there is no tree here to put another level into.
                max_depth: Some(uint!(1)),
            });

            let client = session.client();
            let handle = spawn_tokio!(async move { client.send(request).await });
            self.abort_handle.replace(Some(handle.abort_handle()));

            let Ok(result) = handle.await else {
                // The request was aborted.
                return false;
            };

            self.abort_handle.take();

            if self.room_id.borrow().as_deref() != Some(room_id) {
                // We are listing a different space now, ignore the response.
                return false;
            }

            match result {
                Ok(response) => {
                    self.add_rooms(session, room_id, response);
                    true
                }
                Err(error) => {
                    error!("Could not list the rooms in space `{room_id}`: {error}");
                    self.set_loading_state(LoadingState::Error);
                    false
                }
            }
        }

        /// Add the rooms from the given response to this list.
        fn add_rooms(
            &self,
            session: &Session,
            room_id: &RoomId,
            response: get_hierarchy::v1::Response,
        ) {
            self.next_batch.replace(response.next_batch);

            let mut new_rooms = Vec::new();

            for chunk in response.rooms {
                if chunk.summary.room_id == room_id {
                    // The first room is the space itself. It is not inside
                    // itself, but its `m.space.child` events are the only place
                    // that says which servers to try for the rooms that are.
                    self.remember_via(chunk.children_state);
                    continue;
                }

                let summary = chunk.summary;
                let id = summary
                    .canonical_alias
                    .clone()
                    .map_or_else(|| summary.room_id.clone().into(), Into::into);
                let via = self
                    .via
                    .borrow()
                    .get(&summary.room_id)
                    .cloned()
                    .unwrap_or_default();

                new_rooms.push(RemoteRoom::with_data(
                    session,
                    MatrixRoomIdUri { id, via },
                    summary,
                ));
            }

            self.list().extend_from_slice(&new_rooms);
        }

        /// Remember the servers named by the given `m.space.child` events.
        fn remember_via(&self, children_state: Vec<Raw<HierarchySpaceChildEvent>>) {
            let mut via = self.via.borrow_mut();

            for raw_event in children_state {
                let Ok(event) = raw_event.deserialize() else {
                    warn!("Could not deserialize `m.space.child` event");
                    continue;
                };

                via.insert(event.state_key, event.content.via);
            }
        }
    }
}

glib::wrapper! {
    /// The list of rooms that are inside a space.
    ///
    /// These are remote rooms: the point of the list is to show rooms that
    /// might not have been joined yet, so there is no local `Room` for most of
    /// them.
    ///
    /// Only the rooms directly inside the space are listed. A subspace appears
    /// as a row like any other room, and the rooms inside *it* are listed when
    /// it is opened, rather than nested here.
    pub struct SpaceChildren(ObjectSubclass<imp::SpaceChildren>);
}

impl SpaceChildren {
    /// Construct a new empty `SpaceChildren`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// List the rooms inside the given space.
    pub(crate) fn set_space(&self, session: &Session, room_id: OwnedRoomId) {
        self.imp().set_space(session, room_id);
    }

    /// List the rooms inside the current space again, from the beginning.
    pub(crate) fn reload(&self) {
        spawn!(clone!(
            #[weak(rename_to = imp)]
            self.imp(),
            async move {
                imp.load().await;
            }
        ));
    }
}

impl Default for SpaceChildren {
    fn default() -> Self {
        Self::new()
    }
}

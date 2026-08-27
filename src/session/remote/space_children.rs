use std::collections::HashMap;

use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{
    OwnedRoomId, OwnedServerName, RoomId,
    api::client::space::get_hierarchy,
    assign,
    events::space::child::{HierarchySpaceChildEvent, SpaceChildOrd},
    room::RoomSummary,
    serde::Raw,
};
use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

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

/// One room inside a space, as the space describes it.
///
/// The `via` servers and the suggestion belong to the `m.space.child` event
/// rather than to the room, so a room reachable from two spaces can be
/// described differently by each.
#[derive(Debug, Clone)]
struct SpaceEdge {
    /// The room the space points at.
    room_id: OwnedRoomId,
    /// The servers to reach it through.
    via: Vec<OwnedServerName>,
    /// Whether the space recommends it.
    suggested: bool,
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SpaceChildren)]
    pub struct SpaceChildren {
        /// The rooms directly inside the space.
        #[property(get = Self::list_owned)]
        list: OnceCell<gio::ListStore>,
        /// The session to make the requests with.
        session: glib::WeakRef<Session>,
        /// The ID of the space these rooms are inside.
        room_id: RefCell<Option<OwnedRoomId>>,
        /// The token to continue the listing with, if there is more of it.
        next_batch: RefCell<Option<String>>,
        /// What every room in the hierarchy is.
        summaries: RefCell<HashMap<OwnedRoomId, RoomSummary>>,
        /// What every space in the hierarchy holds, in the order the
        /// specification asks for.
        edges: RefCell<HashMap<OwnedRoomId, Vec<SpaceEdge>>>,
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
    impl ObjectImpl for SpaceChildren {
        fn dispose(&self) {
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }
        }
    }

    impl SpaceChildren {
        /// The rooms directly inside the space.
        fn list(&self) -> &gio::ListStore {
            self.list
                .get_or_init(gio::ListStore::new::<super::SpaceChild>)
        }

        /// The owned list of rooms directly inside the space.
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

            self.reload();
        }

        /// List the rooms inside the current space again, from the beginning.
        pub(super) fn reload(&self) {
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

        /// Walk the whole hierarchy of the current space.
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
            self.summaries.borrow_mut().clear();
            self.edges.borrow_mut().clear();
            self.set_is_truncated(false);
            self.set_loading_state(LoadingState::Loading);

            let mut truncated = true;

            for _ in 0..MAX_BATCHES {
                if !self.load_batch(&session, &room_id).await {
                    // The request failed, was aborted, or the space changed.
                    return;
                }

                if self.next_batch.borrow().is_none() {
                    truncated = false;
                    break;
                }
            }

            if truncated {
                warn!(
                    "Stopped walking the hierarchy of space `{room_id}` after {MAX_BATCHES} batches"
                );
                self.set_is_truncated(true);
            }

            // The rows are built once the whole walk is done. A space whose own
            // chunk has not arrived yet looks like a space with nothing in it,
            // and `GtkTreeListModel` remembers the first answer it is given
            // about whether a row can be expanded.
            self.build_list(&session, &room_id);
            self.set_loading_state(LoadingState::Ready);
        }

        /// Request one batch of the hierarchy and remember what it says.
        ///
        /// Returns `false` if there is no point asking for another one.
        async fn load_batch(&self, session: &Session, room_id: &RoomId) -> bool {
            let from = self.next_batch.borrow().clone();
            let request = assign!(get_hierarchy::v1::Request::new(room_id.to_owned()), {
                from,
                limit: Some(BATCH_SIZE.into()),
                // No `max_depth`: the whole tree is asked for at once, so
                // expanding a subspace costs nothing and never waits. The
                // batch cap below is what bounds it.
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
                    self.remember(response);
                    true
                }
                Err(error) => {
                    error!("Could not walk the hierarchy of space `{room_id}`: {error}");
                    self.set_loading_state(LoadingState::Error);
                    false
                }
            }
        }

        /// Remember what the given response says about the hierarchy.
        fn remember(&self, response: get_hierarchy::v1::Response) {
            self.next_batch.replace(response.next_batch);

            let mut summaries = self.summaries.borrow_mut();
            let mut edges = self.edges.borrow_mut();

            for chunk in response.rooms {
                let room_id = chunk.summary.room_id.clone();

                if !chunk.children_state.is_empty() {
                    edges.insert(room_id.clone(), space_edges(chunk.children_state));
                }

                summaries.insert(room_id, chunk.summary);
            }
        }

        /// Build the rows for the rooms directly inside the space.
        fn build_list(&self, session: &Session, room_id: &RoomId) {
            let children = self.children_of(session, room_id, &[]);

            if children.is_empty() {
                debug!("Nothing to list in the hierarchy of space `{room_id}`");
            }

            self.list().extend_from_slice(&children);
        }

        /// The rooms directly inside the given space, given the spaces already
        /// walked through to reach it.
        pub(super) fn children_of(
            &self,
            session: &Session,
            room_id: &RoomId,
            ancestors: &[OwnedRoomId],
        ) -> Vec<super::SpaceChild> {
            let edges = self.edges.borrow();
            let summaries = self.summaries.borrow();

            let Some(edges) = edges.get(room_id) else {
                return Vec::new();
            };

            edges
                .iter()
                .filter_map(|edge| {
                    // A room the server could not reach has no summary, and
                    // there is nothing to draw for it.
                    let summary = summaries.get(&edge.room_id)?.clone();

                    let id = summary
                        .canonical_alias
                        .clone()
                        .map_or_else(|| summary.room_id.clone().into(), Into::into);
                    let room = RemoteRoom::with_data(
                        session,
                        MatrixRoomIdUri {
                            id,
                            via: edge.via.clone(),
                        },
                        summary,
                    );
                    room.set_is_suggested(edge.suggested);

                    Some(super::SpaceChild::new(&self.obj(), &room, ancestors))
                })
                .collect()
        }

        /// Whether the given space has anything in it that could be shown.
        pub(super) fn holds_rooms(&self, room_id: &RoomId) -> bool {
            self.edges
                .borrow()
                .get(room_id)
                .is_some_and(|edges| !edges.is_empty())
        }
    }

    /// The rooms named by the given `m.space.child` events, in the order the
    /// specification defines.
    ///
    /// The order is `order`, then the time the event was sent, then the room
    /// ID. The server sorts the rooms it returns the same way, but the events
    /// are a set, so this has to sort them itself.
    fn space_edges(children_state: Vec<Raw<HierarchySpaceChildEvent>>) -> Vec<SpaceEdge> {
        let mut events = children_state
            .into_iter()
            .filter_map(|raw_event| match raw_event.deserialize() {
                Ok(event) => Some(event),
                Err(error) => {
                    warn!("Could not deserialize `m.space.child` event: {error}");
                    None
                }
            })
            // A child with no servers to reach it through is not a child. That
            // is how the relationship is undone.
            .filter(|event| !event.content.via.is_empty())
            .collect::<Vec<_>>();

        events.sort_by(SpaceChildOrd::cmp_space_child);

        events
            .into_iter()
            .map(|event| SpaceEdge {
                room_id: event.state_key,
                via: event.content.via,
                suggested: event.content.suggested,
            })
            .collect()
    }
}

glib::wrapper! {
    /// The hierarchy of a space, as far as its homeserver will describe it.
    ///
    /// The whole tree is asked for at once — `/hierarchy` walks it depth-first
    /// and returns every room with the `m.space.child` events of the spaces
    /// among them — so opening a subspace costs no request and never waits.
    /// The listing stops after a fixed number of batches and says so.
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
        self.imp().reload();
    }
}

impl Default for SpaceChildren {
    fn default() -> Self {
        Self::new()
    }
}

mod child_imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SpaceChild)]
    pub struct SpaceChild {
        /// The room this is.
        #[property(get = Self::room_owned)]
        room: OnceCell<RemoteRoom>,
        /// The hierarchy this room was found in.
        ///
        /// A weak reference: the hierarchy owns the rows, directly or through
        /// the rows above them.
        pub(super) hierarchy: glib::WeakRef<super::SpaceChildren>,
        /// The spaces walked through to reach this room, the outermost first.
        pub(super) ancestors: OnceCell<Vec<OwnedRoomId>>,
        /// The rooms inside this one, if it is a space that holds any.
        ///
        /// Asked for once: `GtkTreeListModel` remembers the first answer it
        /// gets about whether a row can be opened.
        pub(super) children: OnceCell<Option<gio::ListStore>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpaceChild {
        const NAME: &'static str = "SpaceChild";
        type Type = super::SpaceChild;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SpaceChild {}

    impl SpaceChild {
        /// Set what this row is.
        pub(super) fn init(
            &self,
            hierarchy: &super::SpaceChildren,
            room: &RemoteRoom,
            ancestors: Vec<OwnedRoomId>,
        ) {
            self.hierarchy.set(Some(hierarchy));
            let _ = self.room.set(room.clone());
            let _ = self.ancestors.set(ancestors);
        }

        /// The room this row is.
        pub(super) fn room(&self) -> &RemoteRoom {
            self.room.get().expect("room should be initialized")
        }

        /// The owned room this row is.
        fn room_owned(&self) -> RemoteRoom {
            self.room().clone()
        }
    }
}

glib::wrapper! {
    /// One room inside a space, and the way down to it.
    ///
    /// The way down is what stops a hierarchy that points back at itself from
    /// being opened forever: a space that is already above this row is not
    /// offered again.
    pub struct SpaceChild(ObjectSubclass<child_imp::SpaceChild>);
}

impl SpaceChild {
    /// Construct a new `SpaceChild` for the given room.
    fn new(hierarchy: &SpaceChildren, room: &RemoteRoom, ancestors: &[OwnedRoomId]) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp().init(hierarchy, room, ancestors.to_owned());
        obj
    }

    /// The rooms inside this one, if it is a space that holds any.
    ///
    /// Returns `None` for a room that is not a space, for a space the walk
    /// found nothing in, and for a space that is already one of the ones
    /// walked through to get here.
    pub(crate) fn children(&self) -> Option<gio::ListStore> {
        let imp = self.imp();

        if let Some(children) = imp.children.get() {
            return children.clone();
        }

        let children = self.build_children();
        let _ = imp.children.set(children.clone());

        children
    }

    /// Build the list of rooms inside this one.
    fn build_children(&self) -> Option<gio::ListStore> {
        let imp = self.imp();
        let room = imp.room();

        if !room.is_space() {
            return None;
        }

        let room_id = room.room_id()?;
        let ancestors = imp.ancestors.get()?;

        if ancestors.contains(&room_id) {
            // A space inside itself, however many steps around. Opening it
            // again would go round the same loop.
            return None;
        }

        let hierarchy = imp.hierarchy.upgrade()?;
        let session = room.session()?;

        if !hierarchy.imp().holds_rooms(&room_id) {
            return None;
        }

        let mut child_ancestors = ancestors.clone();
        child_ancestors.push(room_id.clone());

        let children = hierarchy
            .imp()
            .children_of(&session, &room_id, &child_ancestors);

        if children.is_empty() {
            return None;
        }

        let list = gio::ListStore::new::<Self>();
        list.extend_from_slice(&children);

        Some(list)
    }
}

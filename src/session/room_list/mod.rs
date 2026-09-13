use std::{cell::Cell, collections::HashMap, rc::Rc, time::Duration};

use commune_core::{
    VectorDiff,
    session::{Room as CoreRoom, RoomList as CoreRoomList},
};
use gtk::{
    gio, glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use indexmap::IndexMap;
use ruma::{OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName, RoomId, RoomOrAliasId, UserId};
use tokio::task::AbortHandle;
use tracing::error;

mod room_info;

pub use self::room_info::RoomListRoomInfo;
use crate::{
    core_bridge::{ObjectWatcher, list_model::apply_diff},
    gettext_f,
    prelude::*,
    session::{Room, Session},
    spawn_tokio,
};

mod imp {
    use std::{cell::RefCell, sync::LazyLock};

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomList)]
    pub struct RoomList {
        /// The rooms, in the core's order.
        ///
        /// Also the wrapper cache: the same `Room` for the same room ID
        /// across every diff, so that identity survives for the sidebar's
        /// filter and sort stacks and every binding.
        pub(super) list: RefCell<IndexMap<OwnedRoomId, Room>>,
        /// The current session.
        #[property(get, construct_only)]
        session: glib::WeakRef<Session>,
        /// The task following the core's list.
        watch_handle: RefCell<Option<AbortHandle>>,
        pub(super) get_wait_source: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomList {
        const NAME: &'static str = "RoomList";
        type Type = super::RoomList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomList {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("joining-rooms-changed").build()]);
            SIGNALS.as_ref()
        }

        fn dispose(&self) {
            if let Some(source) = self.get_wait_source.take() {
                source.remove();
            }

            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl ListModelImpl for RoomList {
        fn item_type(&self) -> glib::Type {
            Room::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get_index(position as usize)
                .map(|(_, v)| v.upcast_ref::<glib::Object>())
                .cloned()
        }
    }

    impl RoomList {
        /// The core's room list, if the session is still around.
        pub(super) fn core(&self) -> Option<CoreRoomList> {
            self.session
                .upgrade()
                .map(|session| session.core().room_list().clone())
        }

        /// Get the room with the given room ID, if any.
        pub(super) fn get(&self, room_id: &RoomId) -> Option<Room> {
            self.list.borrow().get(room_id).cloned()
        }

        /// Follow the core's list.
        pub(super) fn load(&self) {
            let Some(core) = self.core() else {
                return;
            };

            let (rooms, stream) = core.subscribe_entries();

            // The core's list is complete before the sync starts, so this
            // is every room the store knew, in one change.
            let wrapped = rooms
                .into_iter()
                .map(|room| self.wrap(room))
                .collect::<Vec<_>>();
            let added = wrapped.len();
            self.list.borrow_mut().extend(wrapped);
            self.obj().items_changed(0, 0, added as u32);

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(stream, |obj: &super::RoomList, diff| {
                    obj.imp().apply_diff(diff);
                })
                .follow(
                    core.subscribe_joining_rooms(),
                    |obj: &super::RoomList, _| {
                        obj.emit_by_name::<()>("joining-rooms-changed", &[]);
                    },
                )
                .spawn();
            self.watch_handle.replace(Some(handle));
        }

        /// The wrapper for the given core room: the one this list has, or a
        /// new one.
        fn wrap(&self, room: CoreRoom) -> (OwnedRoomId, Room) {
            let room_id = room.room_id().to_owned();

            if let Some(existing) = self.get(&room_id) {
                return (room_id, existing);
            }

            let session = self
                .session
                .upgrade()
                .expect("a room list outlives no session");
            (room_id, Room::new(&session, room))
        }

        /// Apply one change from the core's list.
        fn apply_diff(&self, diff: VectorDiff<CoreRoom>) {
            // Wrap first, outside the borrow: a room's constructor may look
            // the list up.
            let diff = diff.map(|room| self.wrap(room));

            let applied = apply_diff(&mut self.list.borrow_mut(), diff);

            let obj = self.obj();
            for change in &applied.changes {
                obj.items_changed(change.position, change.removed, change.added);
            }

            // The retired rooms drop here, after the borrow above is
            // released and items_changed has been emitted: a room's
            // finalize can re-enter this list (the sidebar's filter and
            // sort models watch it), and a live RefCell borrow there
            // aborts the process.
            drop(applied.retired);
        }

        /// Join the room with the given identifier.
        pub(super) async fn join_by_id_or_alias(
            &self,
            identifier: OwnedRoomOrAliasId,
            via: Vec<OwnedServerName>,
        ) -> Result<OwnedRoomId, String> {
            let Some(core) = self.core() else {
                return Err("Could not upgrade Session".to_owned());
            };

            let identifier_clone = identifier.clone();
            let handle =
                spawn_tokio!(async move { core.join_by_id_or_alias(identifier_clone, via).await });

            match handle.await.expect("task was not aborted") {
                Ok(room_id) => Ok(room_id),
                Err(error) => {
                    error!("Joining room {identifier} failed: {error}");

                    let error = gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}', this is a
                        // variable name.
                        "Could not join room {room_name}",
                        &[("room_name", identifier.as_str())],
                    );

                    Err(error)
                }
            }
        }

        /// Request an invite.
        pub(super) async fn knock(
            &self,
            identifier: OwnedRoomOrAliasId,
            via: Vec<OwnedServerName>,
        ) -> Result<OwnedRoomId, String> {
            let Some(core) = self.core() else {
                return Err("Could not upgrade Session".to_owned());
            };

            let identifier_clone = identifier.clone();
            let handle = spawn_tokio!(async move { core.knock(identifier_clone, via).await });

            match handle.await.expect("task was not aborted") {
                Ok(room_id) => Ok(room_id),
                Err(error) => {
                    error!("Invite request for room {identifier} failed: {error}");

                    let error = gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}', this is a
                        // variable name.
                        "Could not request an invite to room {room_name}",
                        &[("room_name", identifier.as_str())],
                    );

                    Err(error)
                }
            }
        }
    }
}

glib::wrapper! {
    /// List of all rooms known by the user.
    ///
    /// This is the parent `GListModel` of the sidebar from which all other models
    /// are derived.
    ///
    /// The `RoomList` also takes care of, so called *pending rooms*, i.e.
    /// rooms the user requested to join, but received no response from the
    /// server yet.
    ///
    /// The list itself is the core's; this presents it, one `Room` per core
    /// room, in the core's order.
    pub struct RoomList(ObjectSubclass<imp::RoomList>)
        @implements gio::ListModel;
}

impl RoomList {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Follow the core's list of rooms.
    pub(crate) fn load(&self) {
        self.imp().load();
    }

    /// Get a snapshot of the rooms list.
    pub(crate) fn snapshot(&self) -> Vec<Room> {
        self.imp().list.borrow().values().cloned().collect()
    }

    /// Whether we are currently joining the room with the given identifier.
    pub(crate) fn is_joining_room(&self, identifier: &RoomOrAliasId) -> bool {
        self.imp()
            .core()
            .is_some_and(|core| core.is_joining_room(identifier))
    }

    /// Get the room with the given room ID, if any.
    pub(crate) fn get(&self, room_id: &RoomId) -> Option<Room> {
        self.imp().get(room_id)
    }

    /// Get the room with the given identifier, if any.
    pub(crate) fn get_by_identifier(&self, identifier: &RoomOrAliasId) -> Option<Room> {
        let room_alias = match <&RoomId>::try_from(identifier) {
            Ok(room_id) => return self.get(room_id),
            Err(room_alias) => room_alias,
        };

        let mut matches = self
            .imp()
            .list
            .borrow()
            .iter()
            .filter(|(_, room)| {
                // We don't want a room that is not joined, it might not be the proper room for
                // the given alias anymore.
                if !room.is_joined() {
                    return false;
                }

                let matrix_room = room.matrix_room();
                matrix_room.canonical_alias().as_deref() == Some(room_alias)
                    || matrix_room.alt_aliases().iter().any(|a| a == room_alias)
            })
            .map(|(room_id, room)| (room_id.clone(), room.clone()))
            .collect::<HashMap<_, _>>();

        if matches.len() <= 1 {
            return matches.into_values().next();
        }

        // The alias is shared between upgraded rooms. We want the latest room, so
        // filter out those that are predecessors.
        let predecessors = matches
            .values()
            .filter_map(|room| room.predecessor_id().cloned())
            .collect::<Vec<_>>();
        for room_id in predecessors {
            matches.remove(&room_id);
        }

        if matches.len() <= 1 {
            return matches.into_values().next();
        }

        // Ideally this should not happen, return the one with the latest activity.
        matches
            .into_values()
            .fold(None::<Room>, |latest_room, room| {
                latest_room
                    .filter(|r| r.latest_activity() >= room.latest_activity())
                    .or(Some(room))
            })
    }

    /// Wait till the room with the given ID becomes available.
    pub(crate) async fn get_wait(
        &self,
        room_id: &RoomId,
        timeout: Option<Duration>,
    ) -> Option<Room> {
        if let Some(room) = self.get(room_id) {
            return Some(room);
        }

        let imp = self.imp();
        let (sender, receiver) = futures_channel::oneshot::channel();

        let room_id = room_id.to_owned();
        let sender_cell = Rc::new(Cell::new(Some(sender)));

        let handler_id = self.connect_items_changed(clone!(
            #[strong]
            sender_cell,
            move |obj, _, _, _| {
                if let Some(room) = obj.get(&room_id)
                    && let Some(sender) = sender_cell.take()
                {
                    let _ = sender.send(Some(room));
                }
            }
        ));

        if let Some(timeout) = timeout {
            let get_wait_source = glib::timeout_add_local_once(timeout, move || {
                if let Some(sender) = sender_cell.take() {
                    let _ = sender.send(None);
                }
            });
            imp.get_wait_source.replace(Some(get_wait_source));
        }

        let room = receiver.await.ok().flatten();

        self.disconnect(handler_id);

        // Remove the source if we got a room.
        if let Some(source) = imp.get_wait_source.take().filter(|_| room.is_some()) {
            source.remove();
        }

        room
    }

    /// Get the joined room that is a direct chat with the user with the given
    /// ID.
    ///
    /// If several rooms are found, returns the room with the latest activity.
    pub(crate) fn direct_chat(&self, user_id: &UserId) -> Option<Room> {
        self.imp()
            .list
            .borrow()
            .values()
            .filter(|r| {
                // A joined room where the direct member is the given user.
                r.is_joined() && r.direct_member().as_ref().map(|m| &**m.user_id()) == Some(user_id)
            })
            // Take the room with the latest activity.
            .max_by(|x, y| x.latest_activity().cmp(&y.latest_activity()))
            .cloned()
    }

    /// Join the room with the given identifier.
    pub(crate) async fn join_by_id_or_alias(
        &self,
        identifier: OwnedRoomOrAliasId,
        via: Vec<OwnedServerName>,
    ) -> Result<OwnedRoomId, String> {
        self.imp().join_by_id_or_alias(identifier, via).await
    }

    /// Request an invite to the room with the given identifier.
    pub(crate) async fn knock(
        &self,
        identifier: OwnedRoomOrAliasId,
        via: Vec<OwnedServerName>,
    ) -> Result<OwnedRoomId, String> {
        self.imp().knock(identifier, via).await
    }

    /// Connect to the signal emitted when the list of rooms we are currently
    /// joining changed.
    pub fn connect_joining_rooms_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "joining-rooms-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

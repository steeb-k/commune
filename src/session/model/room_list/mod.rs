use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
};

use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use indexmap::map::IndexMap;
use matrix_sdk::{
    ruma::{OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName, RoomId, RoomOrAliasId},
    sync::Rooms as ResponseRooms,
    RoomMemberships,
};
use ruma::UserId;
use tracing::{error, warn};

mod room_list_metainfo;

use self::room_list_metainfo::RoomListMetainfo;
pub use self::room_list_metainfo::RoomMetainfo;
use crate::{
    gettext_f,
    prelude::*,
    session::model::{Room, Session},
    spawn_tokio,
    utils::matrix::MatrixRoomId,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::Signal;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomList)]
    pub struct RoomList {
        /// The list of rooms.
        pub list: RefCell<IndexMap<OwnedRoomId, Room>>,
        /// The list of rooms we are currently joining.
        pub pending_rooms: RefCell<HashSet<OwnedRoomOrAliasId>>,
        /// The list of rooms that were upgraded and for which we haven't joined
        /// the successor yet.
        pub tombstoned_rooms: RefCell<HashSet<OwnedRoomId>>,
        /// The current session.
        #[property(get, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// The rooms metainfo that allow to restore the RoomList in its
        /// previous state.
        ///
        /// This is in a Mutex because updating the data in the store is async
        /// and we don't want to overwrite newer data with older data.
        pub metainfo: RoomListMetainfo,
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
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("pending-rooms-changed").build()]);
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();
            self.metainfo.set_room_list(&self.obj());
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
}

glib::wrapper! {
    /// List of all joined rooms of the user.
    ///
    /// This is the parent ListModel of the sidebar from which all other models
    /// are derived.
    ///
    /// The `RoomList` also takes care of all so called *pending rooms*, i.e.
    /// rooms the user requested to join, but received no response from the
    /// server yet.
    pub struct RoomList(ObjectSubclass<imp::RoomList>)
        @implements gio::ListModel;
}

impl RoomList {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Get a snapshot of the rooms list.
    pub fn snapshot(&self) -> Vec<Room> {
        self.imp().list.borrow().values().cloned().collect()
    }

    pub fn is_pending_room(&self, identifier: &RoomOrAliasId) -> bool {
        self.imp().pending_rooms.borrow().contains(identifier)
    }

    fn pending_rooms_remove(&self, identifier: &RoomOrAliasId) {
        self.imp().pending_rooms.borrow_mut().remove(identifier);
        self.emit_by_name::<()>("pending-rooms-changed", &[]);
    }

    fn pending_rooms_insert(&self, identifier: OwnedRoomOrAliasId) {
        self.imp().pending_rooms.borrow_mut().insert(identifier);
        self.emit_by_name::<()>("pending-rooms-changed", &[]);
    }

    fn pending_rooms_replace_or_remove(&self, identifier: &RoomOrAliasId, room_id: &RoomId) {
        {
            let mut pending_rooms = self.imp().pending_rooms.borrow_mut();
            pending_rooms.remove(identifier);
            if !self.contains_key(room_id) {
                pending_rooms.insert(room_id.to_owned().into());
            }
        }
        self.emit_by_name::<()>("pending-rooms-changed", &[]);
    }

    /// Get the room with the given room ID, if any.
    pub fn get(&self, room_id: &RoomId) -> Option<Room> {
        self.imp().list.borrow().get(room_id).cloned()
    }

    /// Get the room with the given identifier, if any.
    pub fn get_by_identifier(&self, identifier: &MatrixRoomId) -> Option<Room> {
        match identifier {
            MatrixRoomId::Id(room_id) => self.get(room_id),
            MatrixRoomId::Alias(room_alias) => self
                .imp()
                .list
                .borrow()
                .values()
                .find(|room| room.matrix_room().canonical_alias().as_ref() == Some(room_alias))
                .cloned(),
        }
    }

    /// Waits till the Room becomes available
    pub async fn get_wait(&self, room_id: &RoomId) -> Option<Room> {
        if let Some(room) = self.get(room_id) {
            Some(room)
        } else {
            let (sender, receiver) = futures_channel::oneshot::channel();

            let room_id = room_id.to_owned();
            let sender = Cell::new(Some(sender));
            // FIXME: add a timeout
            let handler_id = self.connect_items_changed(move |obj, _, _, _| {
                if let Some(room) = obj.get(&room_id) {
                    if let Some(sender) = sender.take() {
                        sender.send(Some(room)).unwrap();
                    }
                }
            });

            let room = receiver.await.unwrap();
            self.disconnect(handler_id);
            room
        }
    }

    pub fn contains_key(&self, room_id: &RoomId) -> bool {
        self.imp().list.borrow().contains_key(room_id)
    }

    pub fn remove(&self, room_id: &RoomId) {
        let imp = self.imp();

        let removed = {
            let mut list = imp.list.borrow_mut();

            list.shift_remove_full(room_id)
        };

        imp.tombstoned_rooms.borrow_mut().remove(room_id);

        if let Some((position, ..)) = removed {
            self.items_changed(position as u32, 1, 0);
        }
    }

    fn items_added(&self, added: usize) {
        let position = {
            let imp = self.imp();
            let list = imp.list.borrow();

            let position = list.len().saturating_sub(added);

            for (_room_id, room) in list.iter().skip(position) {
                room.connect_room_forgotten(clone!(@weak self as obj => move |room| {
                    obj.remove(room.room_id());
                }));
            }

            let mut to_remove = Vec::new();
            for room_id in imp.tombstoned_rooms.borrow().iter() {
                if let Some(room) = list.get(room_id) {
                    if room.update_outdated() {
                        to_remove.push(room_id.clone());
                    }
                } else {
                    to_remove.push(room_id.clone());
                }
            }

            if !to_remove.is_empty() {
                let mut tombstoned_rooms = imp.tombstoned_rooms.borrow_mut();
                for room_id in to_remove {
                    tombstoned_rooms.remove(&room_id);
                }
            }

            position
        };

        self.items_changed(position as u32, 0, added as u32);
    }

    /// Loads the state from the `Store`.
    ///
    /// Note that the `Store` currently doesn't store all events, therefore, we
    /// aren't really loading much via this function.
    pub async fn load(&self) {
        let imp = self.imp();

        let rooms = imp.metainfo.load_rooms().await;
        let added = rooms.len();
        imp.list.borrow_mut().extend(rooms);

        self.items_added(added);
    }

    pub fn handle_response_rooms(&self, rooms: ResponseRooms) {
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();
        let client = session.client();

        let mut new_rooms = HashMap::new();

        for (room_id, left_room) in rooms.leave {
            let room = match self.get(&room_id) {
                Some(room) => room,
                None => match client.get_room(&room_id) {
                    Some(matrix_room) => new_rooms
                        .entry(room_id.clone())
                        .or_insert_with(|| Room::new(&session, matrix_room, None))
                        .clone(),
                    None => {
                        warn!("Could not find left room {room_id}");
                        continue;
                    }
                },
            };

            self.pending_rooms_remove((*room_id).into());
            room.update_room();
            room.handle_left_response(left_room);
        }

        for (room_id, joined_room) in rooms.join {
            let room = match self.get(&room_id) {
                Some(room) => room,
                None => match client.get_room(&room_id) {
                    Some(matrix_room) => new_rooms
                        .entry(room_id.clone())
                        .or_insert_with(|| Room::new(&session, matrix_room, None))
                        .clone(),
                    None => {
                        warn!("Could not find joined room {room_id}");
                        continue;
                    }
                },
            };

            self.pending_rooms_remove((*room_id).into());
            imp.metainfo.watch_room(&room);
            room.update_room();
            room.handle_joined_response(joined_room);
        }

        for (room_id, _invited_room) in rooms.invite {
            let room = match self.get(&room_id) {
                Some(room) => room,
                None => match client.get_room(&room_id) {
                    Some(matrix_room) => new_rooms
                        .entry(room_id.clone())
                        .or_insert_with(|| Room::new(&session, matrix_room, None))
                        .clone(),
                    None => {
                        warn!("Could not find invited room {room_id}");
                        continue;
                    }
                },
            };

            self.pending_rooms_remove((*room_id).into());
            imp.metainfo.watch_room(&room);
            room.update_room();
        }

        if !new_rooms.is_empty() {
            let added = new_rooms.len();
            imp.list.borrow_mut().extend(new_rooms);
            self.items_added(added);
        }
    }

    /// Join the room with the given identifier.
    pub async fn join_by_id_or_alias(
        &self,
        identifier: OwnedRoomOrAliasId,
        via: Vec<OwnedServerName>,
    ) -> Result<OwnedRoomId, String> {
        let Some(session) = self.session() else {
            return Err("Failed to upgrade Session".to_owned());
        };
        let client = session.client();
        let identifier_clone = identifier.clone();

        self.pending_rooms_insert(identifier.clone());

        let handle = spawn_tokio!(async move {
            client
                .join_room_by_id_or_alias(&identifier_clone, &via)
                .await
        });

        match handle.await.unwrap() {
            Ok(matrix_room) => {
                self.pending_rooms_replace_or_remove(&identifier, matrix_room.room_id());
                Ok(matrix_room.room_id().to_owned())
            }
            Err(error) => {
                self.pending_rooms_remove(&identifier);
                error!("Joining room {identifier} failed: {error}");

                let error = gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', this is a
                    // variable name.
                    "Failed to join room {room_name}. Try again later.",
                    &[("room_name", identifier.as_str())],
                );

                Err(error)
            }
        }
    }

    pub fn connect_pending_rooms_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("pending-rooms-changed", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();

            f(&obj);

            None
        })
    }

    /// Get the room with the given identifier, if it is joined.
    pub fn joined_room(&self, identifier: &MatrixRoomId) -> Option<Room> {
        self.get_by_identifier(identifier)
            .filter(|room| room.is_joined())
    }

    /// Add a room that was tombstoned but for which we haven't joined the
    /// successor yet.
    pub fn add_tombstoned_room(&self, room_id: OwnedRoomId) {
        self.imp().tombstoned_rooms.borrow_mut().insert(room_id);
    }

    /// Get the joined room that is a direct chat with the user with the given
    /// ID.
    ///
    /// If several rooms are found, returns the room with the latest activity.
    pub async fn direct_chat(&self, user_id: &UserId) -> Option<Room> {
        let Some(session) = self.session() else {
            return None;
        };
        let direct_rooms = self
            .imp()
            .list
            .borrow()
            .values()
            .filter(|r| r.is_joined() && r.direct_user_id().as_deref() == Some(user_id))
            .cloned()
            .collect::<Vec<_>>();

        if direct_rooms.is_empty() {
            return None;
        }

        let own_user_id = session.user_id();

        let mut final_rooms = vec![];
        for room in direct_rooms {
            let matrix_room = room.matrix_room().clone();
            let handle =
                spawn_tokio!(async move { matrix_room.members(RoomMemberships::ACTIVE).await });

            let members = match handle.await.unwrap() {
                Ok(members) => members,
                Err(error) => {
                    error!("Failed to load members: {error}");
                    continue;
                }
            };

            if members.is_empty() {
                continue;
            }

            if members
                .iter()
                .any(|m| m.user_id() != own_user_id && m.user_id() != user_id)
            {
                // We found other members in this room, let's ignore
                // the room.
                continue;
            }

            final_rooms.push(room);
        }

        final_rooms
            .into_iter()
            .max_by(|x, y| x.latest_activity().cmp(&y.latest_activity()))
    }
}

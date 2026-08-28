//! The list of all rooms known by the user, headless.
//!
//! The application's `room_list/` as an [`eyeball_im::ObservableVector`]
//! plus a lookup map, with the metainfo persistence (the latest-activity
//! and read state that restore the sidebar before the first sync) watching
//! the rooms' observables instead of `GObject` notify signals.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use eyeball::{SharedObservable, Subscriber};
use eyeball_im::{ObservableVector, Vector, VectorDiff};
use futures_util::{Stream, StreamExt};
use indexmap::IndexMap;
use matrix_sdk::sync::RoomUpdates;
use ruma::{OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName, RoomId, RoomOrAliasId, UserId};
use serde::{Deserialize, Serialize};
use tracing::{error, warn};

use super::{Room, WeakSession};
use crate::{RUNTIME, spawn_tokio};

/// The state-store key the rooms metainfo is persisted under.
const ROOMS_METAINFO_KEY: &str = "rooms_metainfo";

/// List of all rooms known by the user.
///
/// This is the parent list of the sidebar from which all other models are
/// derived.
///
/// The `RoomList` also takes care of so called *pending rooms*, i.e. rooms
/// the user requested to join, but received no response from the server
/// yet.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct RoomList {
    inner: Arc<RoomListInner>,
}

#[derive(Debug)]
struct RoomListInner {
    /// The current session.
    session: WeakSession,
    /// The rooms, in insertion order.
    entries: Mutex<ObservableVector<Room>>,
    /// The rooms by ID, for lookups that must not walk the list.
    by_id: Mutex<HashMap<OwnedRoomId, Room>>,
    /// The list of rooms we are currently joining.
    joining_rooms: SharedObservable<HashSet<OwnedRoomOrAliasId>>,
    /// The list of rooms that were upgraded and for which we have not
    /// joined the successor yet.
    tombstoned_rooms: Mutex<HashSet<OwnedRoomId>>,
    /// The rooms metainfo that allow to restore this `RoomList` from its
    /// previous state.
    metainfo: RoomListMetainfo,
}

impl RoomList {
    /// Create a new empty `RoomList` for the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        let inner = Arc::new_cyclic(|weak: &Weak<RoomListInner>| RoomListInner {
            session,
            entries: Mutex::new(ObservableVector::new()),
            by_id: Mutex::new(HashMap::new()),
            joining_rooms: SharedObservable::new(HashSet::new()),
            tombstoned_rooms: Mutex::new(HashSet::new()),
            metainfo: RoomListMetainfo::new(weak.clone()),
        });

        Self { inner }
    }

    /// Load the list of rooms from the store.
    pub async fn load(&self) {
        let rooms = self.inner.metainfo.load_rooms(&self.inner.session).await;

        {
            let mut by_id = self.inner.by_id.lock().expect("mutex is not poisoned");
            for (room_id, room) in &rooms {
                by_id.insert(room_id.clone(), room.clone());
            }
        }

        for room in rooms.values() {
            self.watch_room(room);
        }

        self.inner
            .entries
            .lock()
            .expect("mutex is not poisoned")
            .append(rooms.into_values().collect());
    }

    /// Get a snapshot of the rooms list.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Room> {
        self.inner
            .entries
            .lock()
            .expect("mutex is not poisoned")
            .iter()
            .cloned()
            .collect()
    }

    /// The current rooms, and a stream of the changes that follow them.
    pub fn subscribe_entries(
        &self,
    ) -> (Vector<Room>, impl Stream<Item = VectorDiff<Room>> + use<>) {
        let entries = self.inner.entries.lock().expect("mutex is not poisoned");
        let subscriber = entries.subscribe();
        (entries.clone(), subscriber.into_stream())
    }

    /// Whether we are currently joining the room with the given identifier.
    #[must_use]
    pub fn is_joining_room(&self, identifier: &RoomOrAliasId) -> bool {
        self.inner.joining_rooms.get().contains(identifier)
    }

    /// Subscribe to the set of rooms we are currently joining.
    pub fn subscribe_joining_rooms(&self) -> Subscriber<HashSet<OwnedRoomOrAliasId>> {
        self.inner.joining_rooms.subscribe()
    }

    /// Get the room with the given room ID, if any.
    #[must_use]
    pub fn get(&self, room_id: &RoomId) -> Option<Room> {
        self.inner
            .by_id
            .lock()
            .expect("mutex is not poisoned")
            .get(room_id)
            .cloned()
    }

    /// Get the room with the given identifier, if any.
    #[must_use]
    pub fn get_by_identifier(&self, identifier: &RoomOrAliasId) -> Option<Room> {
        let room_alias = match <&RoomId>::try_from(identifier) {
            Ok(room_id) => return self.get(room_id),
            Err(room_alias) => room_alias,
        };

        let mut matches = self
            .snapshot()
            .into_iter()
            .filter(|room| {
                // We don't want a room that is not joined, it might not be the proper room for
                // the given alias anymore.
                if !room.is_joined() {
                    return false;
                }

                let matrix_room = room.matrix_room();
                matrix_room.canonical_alias().as_deref() == Some(room_alias)
                    || matrix_room.alt_aliases().iter().any(|a| a == room_alias)
            })
            .map(|room| (room.room_id().to_owned(), room))
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
    pub async fn get_wait(&self, room_id: &RoomId, timeout: Option<Duration>) -> Option<Room> {
        if let Some(room) = self.get(room_id) {
            return Some(room);
        }

        let (_, mut stream) = self.subscribe_entries();

        // Between the failed lookup and the subscription the room may have
        // arrived.
        if let Some(room) = self.get(room_id) {
            return Some(room);
        }

        let wait = async {
            while stream.next().await.is_some() {
                if let Some(room) = self.get(room_id) {
                    return Some(room);
                }
            }
            None
        };

        if let Some(timeout) = timeout {
            tokio::time::timeout(timeout, wait).await.ok().flatten()
        } else {
            wait.await
        }
    }

    /// Get the joined room that is a direct chat with the user with the
    /// given ID.
    ///
    /// If several rooms are found, returns the room with the latest
    /// activity.
    #[must_use]
    pub fn direct_chat(&self, user_id: &UserId) -> Option<Room> {
        self.snapshot()
            .into_iter()
            .filter(|room| {
                // A joined room where the direct member is the given user.
                room.is_joined() && room.direct_member_user_id().as_deref() == Some(user_id)
            })
            // Take the room with the latest activity.
            .max_by(|x, y| x.latest_activity().cmp(&y.latest_activity()))
    }

    /// Add a room that was tombstoned but for which we haven't joined the
    /// successor yet.
    pub(crate) fn add_tombstoned_room(&self, room_id: OwnedRoomId) {
        self.inner
            .tombstoned_rooms
            .lock()
            .expect("mutex is not poisoned")
            .insert(room_id);
    }

    /// Handle room updates received via sync.
    pub(crate) fn handle_room_updates(&self, rooms: RoomUpdates) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        let client = session.client();

        let mut new_rooms = HashMap::new();

        for (room_id, left_room) in rooms.left {
            let room = if let Some(room) = self.get(&room_id) {
                room
            } else if let Some(matrix_room) = client.get_room(&room_id) {
                new_rooms
                    .entry(room_id.clone())
                    .or_insert_with(|| Room::new(&session, matrix_room, None))
                    .clone()
            } else {
                warn!("Could not find left room {room_id}");
                continue;
            };

            self.remove_joining_room((*room_id).into());
            // Ambiguity changes wait for the member model.
            let _ = (room, left_room);
        }

        for (room_id, joined_room) in rooms.joined {
            let room = if let Some(room) = self.get(&room_id) {
                room
            } else if let Some(matrix_room) = client.get_room(&room_id) {
                new_rooms
                    .entry(room_id.clone())
                    .or_insert_with(|| Room::new(&session, matrix_room, None))
                    .clone()
            } else {
                warn!("Could not find joined room {room_id}");
                continue;
            };

            self.remove_joining_room((*room_id).into());
            self.inner.metainfo.watch_room(&room);
            room.handle_sync_timeline_events(
                joined_room
                    .timeline
                    .events
                    .iter()
                    .map(matrix_sdk::deserialized_responses::TimelineEvent::raw),
            );
        }

        for (room_id, _invited_room) in rooms.invited {
            let room = if let Some(room) = self.get(&room_id) {
                room
            } else if let Some(matrix_room) = client.get_room(&room_id) {
                new_rooms
                    .entry(room_id.clone())
                    .or_insert_with(|| Room::new(&session, matrix_room, None))
                    .clone()
            } else {
                warn!("Could not find invited room {room_id}");
                continue;
            };

            self.remove_joining_room((*room_id).into());
            self.inner.metainfo.watch_room(&room);
        }

        for (room_id, _knocked_room) in rooms.knocked {
            let room = if let Some(room) = self.get(&room_id) {
                room
            } else if let Some(matrix_room) = client.get_room(&room_id) {
                new_rooms
                    .entry(room_id.clone())
                    .or_insert_with(|| Room::new(&session, matrix_room, None))
                    .clone()
            } else {
                warn!("Could not find knocked room {room_id}");
                continue;
            };

            self.remove_joining_room((*room_id).into());
            self.inner.metainfo.watch_room(&room);
        }

        if !new_rooms.is_empty() {
            self.insert_rooms(new_rooms);
        }
    }

    /// Insert the given new rooms into the list.
    fn insert_rooms(&self, new_rooms: HashMap<OwnedRoomId, Room>) {
        {
            let mut by_id = self.inner.by_id.lock().expect("mutex is not poisoned");
            for (room_id, room) in &new_rooms {
                by_id.insert(room_id.clone(), room.clone());
            }
        }

        let mut tombstoned_rooms_to_remove = Vec::new();

        for room in new_rooms.values() {
            self.watch_room(room);

            // Check if the new room is the successor to a tombstoned room.
            if let Some(predecessor_id) = room.predecessor_id()
                && self
                    .inner
                    .tombstoned_rooms
                    .lock()
                    .expect("mutex is not poisoned")
                    .contains(predecessor_id)
                && let Some(predecessor) = self.get(predecessor_id)
            {
                predecessor.update_successor();
                tombstoned_rooms_to_remove.push(predecessor_id.clone());
            }
        }

        if !tombstoned_rooms_to_remove.is_empty() {
            let mut tombstoned_rooms = self
                .inner
                .tombstoned_rooms
                .lock()
                .expect("mutex is not poisoned");
            for room_id in tombstoned_rooms_to_remove {
                tombstoned_rooms.remove(&room_id);
            }
        }

        self.inner
            .entries
            .lock()
            .expect("mutex is not poisoned")
            .append(new_rooms.into_values().collect());
    }

    /// Remove the room with the given ID once it reports being forgotten.
    fn watch_room(&self, room: &Room) {
        let mut subscriber = room.subscribe_forgotten();
        let weak = Arc::downgrade(&self.inner);
        let room_id = room.room_id().to_owned();

        RUNTIME.spawn(async move {
            while let Some(forgotten) = subscriber.next().await {
                if forgotten {
                    if let Some(inner) = weak.upgrade() {
                        RoomList { inner }.remove(&room_id);
                    }
                    break;
                }
            }
        });
    }

    /// Remove the room with the given ID.
    fn remove(&self, room_id: &RoomId) {
        self.inner
            .by_id
            .lock()
            .expect("mutex is not poisoned")
            .remove(room_id);
        self.inner
            .tombstoned_rooms
            .lock()
            .expect("mutex is not poisoned")
            .remove(room_id);

        let mut entries = self.inner.entries.lock().expect("mutex is not poisoned");
        if let Some(index) = entries.iter().position(|room| room.room_id() == room_id) {
            entries.remove(index);
        }
    }

    /// Remove the given room identifier from the rooms we are currently
    /// joining.
    fn remove_joining_room(&self, identifier: &RoomOrAliasId) {
        let mut joining_rooms = self.inner.joining_rooms.get();

        if joining_rooms.remove(identifier) {
            self.inner.joining_rooms.set(joining_rooms);
        }
    }

    /// Add the given room identifier to the rooms we are currently joining.
    fn add_joining_room(&self, identifier: OwnedRoomOrAliasId) {
        let mut joining_rooms = self.inner.joining_rooms.get();

        if joining_rooms.insert(identifier) {
            self.inner.joining_rooms.set(joining_rooms);
        }
    }

    /// Remove the given room identifier from the rooms we are currently
    /// joining and replace it with the given room ID if the room is not in
    /// the list yet.
    fn remove_or_replace_joining_room(&self, identifier: &RoomOrAliasId, room_id: &RoomId) {
        let mut joining_rooms = self.inner.joining_rooms.get();
        joining_rooms.remove(identifier);

        if self.get(room_id).is_none() {
            joining_rooms.insert(room_id.to_owned().into());
        }

        self.inner.joining_rooms.set(joining_rooms);
    }

    /// Join the room with the given identifier.
    pub async fn join_by_id_or_alias(
        &self,
        identifier: OwnedRoomOrAliasId,
        via: Vec<OwnedServerName>,
    ) -> Result<OwnedRoomId, String> {
        let Some(session) = self.inner.session.upgrade() else {
            return Err("Could not upgrade Session".to_owned());
        };
        let client = session.client();
        let identifier_clone = identifier.clone();

        self.add_joining_room(identifier.clone());

        let handle = spawn_tokio!(async move {
            client
                .join_room_by_id_or_alias(&identifier_clone, &via)
                .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(matrix_room) => {
                self.remove_or_replace_joining_room(&identifier, matrix_room.room_id());
                Ok(matrix_room.room_id().to_owned())
            }
            Err(join_error) => {
                self.remove_joining_room(&identifier);
                error!("Joining room {identifier} failed: {join_error}");

                Err(format!("Could not join room {identifier}"))
            }
        }
    }

    /// Request an invite.
    pub async fn knock(
        &self,
        identifier: OwnedRoomOrAliasId,
        via: Vec<OwnedServerName>,
    ) -> Result<OwnedRoomId, String> {
        let Some(session) = self.inner.session.upgrade() else {
            return Err("Could not upgrade Session".to_owned());
        };
        let client = session.client();

        let identifier_clone = identifier.clone();
        let handle = spawn_tokio!(async move { client.knock(identifier_clone, None, via).await });

        match handle.await.expect("task was not aborted") {
            Ok(matrix_room) => Ok(matrix_room.room_id().to_owned()),
            Err(knock_error) => {
                error!("Invite request for room {identifier} failed: {knock_error}");

                Err(format!("Could not request an invite to room {identifier}"))
            }
        }
    }
}

type RoomsMetainfoMap = BTreeMap<OwnedRoomId, RoomMetainfo>;

/// The rooms metainfo that allow to restore the [`RoomList`] in its
/// previous state.
#[derive(Debug)]
struct RoomListMetainfo {
    /// The rooms metainfos.
    ///
    /// In a Mutex because persisting the data in the store is async and we
    /// only want one operation at a time.
    rooms_metainfo: tokio::sync::Mutex<RoomsMetainfoMap>,
    /// Set of room IDs for which the metainfo should be updated.
    ///
    /// This list is kept to avoid queuing the same room several times in a
    /// row while we wait for the async operation to finish.
    pending_updates: Mutex<HashSet<OwnedRoomId>>,
    /// The parent `RoomList`.
    room_list: Weak<RoomListInner>,
}

impl RoomListMetainfo {
    /// Create a new `RoomListMetainfo` for the given room list.
    fn new(room_list: Weak<RoomListInner>) -> Self {
        Self {
            rooms_metainfo: tokio::sync::Mutex::new(RoomsMetainfoMap::new()),
            pending_updates: Mutex::new(HashSet::new()),
            room_list,
        }
    }

    /// Load the rooms and their metainfo from the store.
    async fn load_rooms(&self, session: &WeakSession) -> IndexMap<OwnedRoomId, Room> {
        let Some(session) = session.upgrade() else {
            return IndexMap::new();
        };
        let client = session.client();

        // Load the serialized map from the store.
        let mut rooms_metainfo: RoomsMetainfoMap = match client
            .state_store()
            .get_custom_value(ROOMS_METAINFO_KEY.as_bytes())
            .await
        {
            Ok(Some(value)) => match serde_json::from_slice(&value) {
                Ok(metainfo) => metainfo,
                Err(deserialize_error) => {
                    error!("Could not deserialize rooms metainfo: {deserialize_error}");
                    RoomsMetainfoMap::default()
                }
            },
            Ok(None) => RoomsMetainfoMap::default(),
            Err(load_error) => {
                error!("Could not load rooms metainfo: {load_error}");
                RoomsMetainfoMap::default()
            }
        };

        // We need to acquire the lock now to make sure we have the full map before any
        // change happens and the map tries to be persisted.
        let mut rooms_metainfo_guard = self.rooms_metainfo.lock().await;

        // Restore rooms and listen to changes.
        let matrix_rooms = client.rooms();
        let mut rooms = IndexMap::with_capacity(matrix_rooms.len());

        for matrix_room in matrix_rooms {
            let room_id = matrix_room.room_id().to_owned();
            let metainfo = rooms_metainfo.remove(&room_id);

            let room = Room::new(&session, matrix_room, metainfo);

            self.watch_room(&room);

            if let Some(metainfo) = metainfo {
                rooms_metainfo_guard.insert(room_id.clone(), metainfo);
            }

            rooms.insert(room_id, room);
        }

        rooms
    }

    /// Watch the given room for metainfo changes.
    fn watch_room(&self, room: &Room) {
        let weak = Weak::clone(&self.room_list);
        let room_id = room.room_id().to_owned();
        let mut latest_activity = room.subscribe_latest_activity();
        let mut is_read = room.subscribe_is_read();

        RUNTIME.spawn(async move {
            loop {
                tokio::select! {
                    changed = latest_activity.next() => if changed.is_none() { break },
                    changed = is_read.next() => if changed.is_none() { break },
                }

                let Some(inner) = weak.upgrade() else {
                    break;
                };

                inner
                    .metainfo
                    .update_rooms_metainfo_for_room(room_id.clone())
                    .await;
            }
        });
    }

    /// Update the room metainfo for the room with the given ID.
    async fn update_rooms_metainfo_for_room(&self, room_id: OwnedRoomId) {
        self.pending_updates
            .lock()
            .expect("mutex is not poisoned")
            .insert(room_id);

        while !self
            .pending_updates
            .lock()
            .expect("mutex is not poisoned")
            .is_empty()
        {
            if !self.try_update_rooms_metainfo().await {
                return;
            }
        }
    }

    /// Update the rooms metainfo if a lock can be acquired.
    ///
    /// Returns `true` if the lock could be acquired.
    async fn try_update_rooms_metainfo(&self) -> bool {
        let Ok(mut rooms_metainfo_guard) = self.rooms_metainfo.try_lock() else {
            return false;
        };

        let room_ids =
            std::mem::take(&mut *self.pending_updates.lock().expect("mutex is not poisoned"));

        if room_ids.is_empty() {
            return true;
        }

        let Some(room_list) = self.room_list.upgrade().map(|inner| RoomList { inner }) else {
            return true;
        };
        let mut has_changed = false;

        for (room, room_id) in room_ids
            .into_iter()
            .filter_map(|room_id| room_list.get(&room_id).map(|room| (room, room_id)))
        {
            let metainfo = rooms_metainfo_guard.entry(room_id).or_default();
            has_changed |= metainfo.update(&room);
        }

        if has_changed {
            self.persist(&room_list, &rooms_metainfo_guard).await;
        }

        true
    }

    /// Persist the metainfo in the store.
    async fn persist(&self, room_list: &RoomList, rooms_metainfo: &RoomsMetainfoMap) {
        let Some(session) = room_list.inner.session.upgrade() else {
            return;
        };
        let value = match serde_json::to_vec(rooms_metainfo) {
            Ok(value) => value,
            Err(serialize_error) => {
                error!("Could not serialize rooms metainfo: {serialize_error}");
                return;
            }
        };

        let client = session.client();
        if let Err(store_error) = client
            .state_store()
            .set_custom_value(ROOMS_METAINFO_KEY.as_bytes(), value)
            .await
        {
            error!("Could not store rooms metainfo: {store_error}");
        }
    }
}

/// The room metainfo that needs to be persisted in the state store.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct RoomMetainfo {
    /// The timestamp of the room's latest activity.
    pub latest_activity: u64,
    /// Whether all messages of the room are read.
    pub is_read: bool,
}

impl RoomMetainfo {
    /// Update this `RoomMetainfo` for the given `Room`.
    ///
    /// Returns `true` if the data was updated.
    fn update(&mut self, room: &Room) -> bool {
        let mut has_changed = false;

        let latest_activity = room.latest_activity();
        if self.latest_activity != latest_activity {
            self.latest_activity = latest_activity;
            has_changed = true;
        }

        let is_read = room.is_read();
        if self.is_read != is_read {
            self.is_read = is_read;
            has_changed = true;
        }

        has_changed
    }
}

use std::cell::RefCell;

use commune_core::session::{RemoteRoom as CoreRemoteRoom, RemoteRoomEntry, RemoteRoomState};
use gtk::{glib, prelude::*, subclass::prelude::*};
use ruma::{OwnedRoomAliasId, OwnedRoomId, room::RoomSummary};
use tokio::task::AbortHandle;

use crate::{
    components::{AvatarImage, AvatarUriSource, PillSource},
    core_bridge::ObjectWatcher,
    prelude::*,
    session::{RoomListRoomInfo, Session},
    utils::{LoadingState, matrix::MatrixRoomIdUri, string::linkify},
};

mod imp {
    use std::cell::{Cell, OnceCell};

    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::RemoteRoom)]
    pub struct RemoteRoom {
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        session: glib::WeakRef<Session>,
        /// The Matrix URI of this room.
        uri: OnceCell<MatrixRoomIdUri>,
        /// The ID of this room.
        room_id: RefCell<Option<OwnedRoomId>>,
        /// The canonical alias of this room.
        canonical_alias: RefCell<Option<OwnedRoomAliasId>>,
        /// The name that is set for this room.
        ///
        /// This can be empty, the display name should be used instead in the
        /// interface.
        #[property(get)]
        name: RefCell<Option<String>>,
        /// The topic of this room.
        #[property(get)]
        topic: RefCell<Option<String>>,
        /// The linkified topic of this room.
        ///
        /// This is the string that should be used in the interface when markup
        /// is allowed.
        #[property(get)]
        topic_linkified: RefCell<Option<String>>,
        /// The number of joined members in the room.
        #[property(get)]
        joined_members_count: Cell<u32>,
        /// Whether we can knock on the room.
        #[property(get)]
        can_knock: Cell<bool>,
        /// Whether this room is a space.
        #[property(get)]
        is_space: Cell<bool>,
        /// Whether this room can be read without joining it.
        #[property(get)]
        is_world_readable: Cell<bool>,
        /// Whether this room is encrypted.
        #[property(get)]
        is_encrypted: Cell<bool>,
        /// Whether the space this room was listed from suggests it.
        ///
        /// This belongs to the `m.space.child` event rather than to the room,
        /// so it is only ever true for a room that came from a space's
        /// hierarchy.
        #[property(get, set)]
        is_suggested: Cell<bool>,
        /// The information about this room in the room list.
        #[property(get)]
        room_list_info: RoomListRoomInfo,
        /// The loading state.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// The task following the core's entry for this room, if it has one.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RemoteRoom {
        const NAME: &'static str = "RemoteRoom";
        type Type = super::RemoteRoom;
        type ParentType = PillSource;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RemoteRoom {
        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl PillSourceImpl for RemoteRoom {
        fn identifier(&self) -> String {
            self.uri().id.to_string()
        }
    }

    impl RemoteRoom {
        /// Set the current session.
        fn set_session(&self, session: &Session) {
            self.session.set(Some(session));

            self.obj().avatar_data().set_image(Some(AvatarImage::new(
                session,
                AvatarUriSource::Room,
                None,
                None,
            )));

            self.room_list_info.set_room_list(session.room_list());
        }

        /// Set the Matrix URI of this room.
        pub(super) fn set_uri(&self, uri: MatrixRoomIdUri) {
            if let Ok(room_id) = uri.id.clone().try_into() {
                self.set_room_id(room_id);
            }

            self.uri
                .set(uri)
                .expect("Matrix URI should be uninitialized");

            self.update_identifiers();
            self.update_display_name();
        }

        /// The Matrix URI of this room.
        pub(super) fn uri(&self) -> &MatrixRoomIdUri {
            self.uri.get().expect("Matrix URI should be initialized")
        }

        /// Follow the core's entry for this room.
        pub(super) fn watch(&self, entry: &RemoteRoomEntry) {
            let handle = ObjectWatcher::new(&*self.obj())
                .follow(entry.subscribe(), |obj: &super::RemoteRoom, state| {
                    obj.imp().update_state(&state);
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.update_state(&entry.state());
        }

        /// Mirror what the core knows about this room.
        fn update_state(&self, state: &RemoteRoomState) {
            if let Some(data) = &state.data {
                self.set_data(data);
            }

            self.set_loading_state(state.loading_state.into());
        }

        /// Set the ID of this room.
        fn set_room_id(&self, room_id: OwnedRoomId) {
            self.room_id.replace(Some(room_id));
        }

        /// The ID of this room.
        pub(super) fn room_id(&self) -> Option<OwnedRoomId> {
            self.room_id.borrow().clone()
        }

        /// Set the canonical alias of this room.
        fn set_canonical_alias(&self, alias: Option<OwnedRoomAliasId>) {
            if *self.canonical_alias.borrow() == alias {
                return;
            }

            self.canonical_alias.replace(alias);
            self.update_display_name();
        }

        /// The canonical alias of this room.
        pub(super) fn canonical_alias(&self) -> Option<OwnedRoomAliasId> {
            self.canonical_alias
                .borrow()
                .clone()
                .or_else(|| self.uri().id.clone().try_into().ok())
        }

        /// Update the identifiers to watch in the room list.
        fn update_identifiers(&self) {
            let id = self.uri().id.clone();
            let room_id = self
                .room_id()
                .filter(|room_id| room_id.as_str() != id.as_str())
                .map(Into::into);
            let canonical_alias = self
                .canonical_alias()
                .filter(|alias| alias.as_str() != id.as_str())
                .map(Into::into);

            let identifiers = room_id
                .into_iter()
                .chain(canonical_alias)
                .chain(Some(id))
                .collect();

            self.room_list_info.set_identifiers(identifiers);
        }

        /// Set the name of this room.
        fn set_name(&self, name: Option<String>) {
            if *self.name.borrow() == name {
                return;
            }

            self.name.replace(name);

            self.obj().notify_name();
            self.update_display_name();
        }

        /// The display name of this room.
        pub(super) fn update_display_name(&self) {
            let display_name = self
                .name
                .borrow()
                .clone()
                .or_else(|| {
                    self.canonical_alias
                        .borrow()
                        .as_ref()
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| self.identifier());

            self.obj().set_display_name(display_name);
        }

        /// Set the topic of this room.
        ///
        /// The core already left out a topic that is nothing but whitespace;
        /// the markup is this object's.
        fn set_topic(&self, topic: Option<String>) {
            if *self.topic.borrow() == topic {
                return;
            }

            let topic_linkified = topic.as_deref().map(|t| {
                // Detect links.
                let mut s = linkify(t);
                // Remove trailing spaces.
                s.truncate_end_whitespaces();
                s
            });

            self.topic.replace(topic);
            self.topic_linkified.replace(topic_linkified);

            let obj = self.obj();
            obj.notify_topic();
            obj.notify_topic_linkified();
        }

        /// Set the number of joined members in the room.
        fn set_joined_members_count(&self, count: u32) {
            if self.joined_members_count.get() == count {
                return;
            }

            self.joined_members_count.set(count);
            self.obj().notify_joined_members_count();
        }

        /// Set whether we can knock on the room.
        fn set_can_knock(&self, can_knock: bool) {
            if self.can_knock.get() == can_knock {
                return;
            }

            self.can_knock.set(can_knock);
            self.obj().notify_can_knock();
        }

        /// Set whether this room is a space.
        fn set_is_space(&self, is_space: bool) {
            if self.is_space.get() == is_space {
                return;
            }

            self.is_space.set(is_space);
            self.obj().notify_is_space();
        }

        /// Set whether this room can be read without joining it.
        fn set_is_world_readable(&self, is_world_readable: bool) {
            if self.is_world_readable.get() == is_world_readable {
                return;
            }

            self.is_world_readable.set(is_world_readable);
            self.obj().notify_is_world_readable();
        }

        /// Set whether this room is encrypted.
        fn set_is_encrypted(&self, is_encrypted: bool) {
            if self.is_encrypted.get() == is_encrypted {
                return;
            }

            self.is_encrypted.set(is_encrypted);
            self.obj().notify_is_encrypted();
        }

        /// Set the loading state.
        fn set_loading_state(&self, loading_state: LoadingState) {
            if self.loading_state.get() == loading_state {
                return;
            }

            self.loading_state.set(loading_state);
            self.obj().notify_loading_state();
        }

        /// Set the room data, as the core describes it.
        pub(super) fn set_data(&self, data: &CoreRemoteRoom) {
            self.set_room_id(data.room_id.clone());
            self.set_canonical_alias(data.canonical_alias.clone());
            self.set_name(data.name.clone());
            self.set_topic(data.topic.clone());
            self.set_joined_members_count(data.joined_members_count);
            self.set_can_knock(data.can_knock);
            self.set_is_space(data.is_space);
            self.set_is_world_readable(data.is_world_readable);
            self.set_is_encrypted(data.is_encrypted);

            if data.is_suggested {
                self.obj().set_is_suggested(true);
            }

            if let Some(image) = self.obj().avatar_data().image() {
                image.set_uri_and_info(data.avatar_url.clone(), None);
            }

            self.update_identifiers();
            self.set_loading_state(LoadingState::Ready);
        }
    }
}

glib::wrapper! {
    /// A Room that can only be updated by making remote calls, i.e. it won't be updated via sync.
    ///
    /// The data, and the asking for it, are the core's; this presents them.
    pub struct RemoteRoom(ObjectSubclass<imp::RemoteRoom>)
        @extends PillSource;
}

impl RemoteRoom {
    /// Construct a new `RemoteRoom` for the given URI, without any data.
    fn without_data(session: &Session, uri: MatrixRoomIdUri) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();
        obj.imp().set_uri(uri);
        obj
    }

    /// Construct a new `RemoteRoom` presenting the given entry of the core's
    /// cache, which asks for the data and asks again when it is stale.
    pub(super) fn new(session: &Session, entry: &RemoteRoomEntry) -> Self {
        let obj = Self::without_data(session, entry.uri().clone());
        obj.imp().watch(entry);
        obj
    }

    /// Construct a new `RemoteRoom` for the given URI and data.
    pub(crate) fn with_data(
        session: &Session,
        uri: MatrixRoomIdUri,
        data: impl Into<RoomSummary>,
    ) -> Self {
        Self::from_core(session, &CoreRemoteRoom::with_data(uri, data))
    }

    /// Construct a new `RemoteRoom` for the given room of the core.
    pub(crate) fn from_core(session: &Session, data: &CoreRemoteRoom) -> Self {
        let obj = Self::without_data(session, data.uri.clone());
        obj.imp().set_data(data);
        obj
    }

    /// The Matrix URI of this room.
    pub(crate) fn uri(&self) -> &MatrixRoomIdUri {
        self.imp().uri()
    }

    /// The ID of this room.
    pub(crate) fn room_id(&self) -> Option<OwnedRoomId> {
        self.imp().room_id()
    }

    /// The canonical alias of this room.
    pub(crate) fn canonical_alias(&self) -> Option<OwnedRoomAliasId> {
        self.imp().canonical_alias()
    }
}

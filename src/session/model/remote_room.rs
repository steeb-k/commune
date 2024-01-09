use std::cell::RefCell;

use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{
    api::client::space::{get_hierarchy, SpaceHierarchyRoomsChunk},
    assign, uint, OwnedRoomAliasId, OwnedRoomId,
};
use tracing::{debug, error};

use super::{AvatarData, AvatarImage, AvatarUriSource, Session};
use crate::{
    spawn, spawn_tokio,
    utils::{
        matrix::{MatrixRoomId, MatrixRoomIdUri},
        LoadingState,
    },
};

mod imp {
    use std::{
        cell::{Cell, OnceCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::RemoteRoom)]
    pub struct RemoteRoom {
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// The Matrix URI of this room.
        pub uri: OnceCell<MatrixRoomIdUri>,
        /// The identifier of this room, as a string.
        #[property(get = Self::identifier_string)]
        pub identifier_string: PhantomData<String>,
        /// The Matrix ID of this room.
        pub room_id: RefCell<Option<OwnedRoomId>>,
        /// The canonical alias of this room.
        pub alias: RefCell<Option<OwnedRoomAliasId>>,
        /// The name that is set for this room.
        ///
        /// This can be empty, the display name should be used instead in the
        /// interface.
        #[property(get)]
        pub name: RefCell<Option<String>>,
        /// The display name of this room.
        #[property(get = Self::display_name)]
        pub display_name: RefCell<String>,
        /// The topic of this room.
        #[property(get)]
        pub topic: RefCell<Option<String>>,
        /// The Avatar data of this room.
        #[property(get)]
        pub avatar_data: AvatarData,
        /// The number of joined members in the room.
        #[property(get)]
        pub joined_members_count: Cell<u32>,
        /// The loading state.
        #[property(get, builder(LoadingState::default()))]
        pub loading_state: Cell<LoadingState>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RemoteRoom {
        const NAME: &'static str = "RemoteRoom";
        type Type = super::RemoteRoom;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RemoteRoom {}

    impl RemoteRoom {
        /// Set the current session.
        fn set_session(&self, session: Session) {
            self.session.set(Some(&session));

            self.avatar_data.set_image(Some(AvatarImage::new(
                &session,
                None,
                AvatarUriSource::Room,
            )));
        }

        /// The identifier of this room, as a string.
        fn identifier_string(&self) -> String {
            self.uri.get().unwrap().id.to_string()
        }

        /// Set the Matrix ID of this room.
        fn set_room_id(&self, room_id: Option<OwnedRoomId>) {
            if *self.room_id.borrow() == room_id {
                return;
            }

            self.room_id.replace(room_id);
        }

        /// Set the alias of this room.
        fn set_alias(&self, alias: Option<OwnedRoomAliasId>) {
            if *self.alias.borrow() == alias {
                return;
            }

            self.alias.replace(alias);
            self.obj().notify_display_name();
        }

        /// Set the name of this room.
        fn set_name(&self, name: Option<String>) {
            if *self.name.borrow() == name {
                return;
            }

            self.name.replace(name);

            let obj = self.obj();
            obj.notify_name();
            obj.notify_display_name();
        }

        /// The display name of this room.
        fn display_name(&self) -> String {
            self.name
                .borrow()
                .clone()
                .or_else(|| self.alias.borrow().as_ref().map(ToString::to_string))
                .unwrap_or_else(|| self.identifier_string())
        }

        /// Set the topic of this room.
        fn set_topic(&self, topic: Option<String>) {
            let topic =
                topic.filter(|s| !s.is_empty() && s.find(|c: char| !c.is_whitespace()).is_some());

            if *self.topic.borrow() == topic {
                return;
            }

            self.topic.replace(topic);
            self.obj().notify_topic();
        }

        /// Set the loading state.
        fn set_joined_members_count(&self, count: u32) {
            if self.joined_members_count.get() == count {
                return;
            }

            self.joined_members_count.set(count);
            self.obj().notify_joined_members_count();
        }

        /// Set the loading state.
        pub(super) fn set_loading_state(&self, loading_state: LoadingState) {
            if self.loading_state.get() == loading_state {
                return;
            }

            self.loading_state.set(loading_state);
            self.obj().notify_loading_state();
        }

        /// Update the room data with the given response.
        pub(super) fn update_data(&self, data: SpaceHierarchyRoomsChunk) {
            self.set_room_id(Some(data.room_id));
            self.set_alias(data.canonical_alias);
            self.set_name(data.name);
            self.set_topic(data.topic);
            self.set_joined_members_count(data.num_joined_members.try_into().unwrap_or(u32::MAX));

            if let Some(image) = self.avatar_data.image() {
                image.set_uri(data.avatar_url.map(String::from));
            }

            self.set_loading_state(LoadingState::Ready);
        }
    }
}

glib::wrapper! {
    /// A Room that can only be updated by making remote calls, i.e. it won't be updated via sync.
    pub struct RemoteRoom(ObjectSubclass<imp::RemoteRoom>);
}

impl RemoteRoom {
    pub fn new(session: &Session, uri: MatrixRoomIdUri) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        let imp = obj.imp();
        imp.uri.set(uri).unwrap();
        obj.bind_property("display-name", &imp.avatar_data, "display-name")
            .sync_create()
            .build();

        spawn!(clone!(@weak obj => async move {
            obj.load().await;
        }));

        obj
    }

    /// The Matrix URI of this room.
    pub fn uri(&self) -> &MatrixRoomIdUri {
        self.imp().uri.get().unwrap()
    }

    /// The Matrix ID of this room.
    pub fn room_id(&self) -> Option<OwnedRoomId> {
        self.imp()
            .room_id
            .borrow()
            .clone()
            .or_else(|| self.uri().id.as_id().cloned())
    }

    /// The canonical alias of this room.
    pub fn alias(&self) -> Option<OwnedRoomAliasId> {
        self.imp()
            .alias
            .borrow()
            .clone()
            .or_else(|| self.uri().id.as_alias().cloned())
    }

    /// Load the data of this room.
    async fn load(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();

        imp.set_loading_state(LoadingState::Loading);

        let uri = self.uri();
        let client = session.client();

        let room_id = match uri.id.clone() {
            MatrixRoomId::Id(room_id) => room_id,
            MatrixRoomId::Alias(alias) => {
                let client_clone = client.clone();
                let handle =
                    spawn_tokio!(async move { client_clone.resolve_room_alias(&alias).await });

                match handle.await.unwrap() {
                    Ok(response) => response.room_id,
                    Err(error) => {
                        error!("Failed to resolve room alias `{}`: {error}", uri.id);
                        imp.set_loading_state(LoadingState::Error);
                        return;
                    }
                }
            }
        };

        // FIXME: The space hierarchy endpoint gives us the room details we want, but it
        // doesn't work if the room is not known by the homeserver. We need MSC3266 for
        // a proper endpoint.
        let request = assign!(get_hierarchy::v1::Request::new(room_id.clone()), {
            // We are only interested in the single room.
            limit: Some(uint!(1))
        });
        let handle = spawn_tokio!(async move { client.send(request, None).await });

        match handle.await.unwrap() {
            Ok(response) => {
                if let Some(chunk) = response
                    .rooms
                    .into_iter()
                    .next()
                    .filter(|c| c.room_id == room_id)
                {
                    imp.update_data(chunk);
                } else {
                    debug!("Endpoint did not return requested room");
                    imp.set_loading_state(LoadingState::Error);
                }
            }
            Err(error) => {
                error!("Failed to get room details for room `{}`: {error}", uri.id);
                imp.set_loading_state(LoadingState::Error);
            }
        }
    }
}

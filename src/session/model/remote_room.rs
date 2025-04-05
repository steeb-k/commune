use std::cell::RefCell;

use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{
    api::client::space::{get_hierarchy, SpaceHierarchyRoomsChunk},
    assign, uint, OwnedRoomAliasId, OwnedRoomId,
};
use tracing::{debug, warn};

use super::Session;
use crate::{
    components::{AvatarImage, AvatarUriSource, PillSource},
    prelude::*,
    spawn, spawn_tokio,
    utils::{matrix::MatrixRoomIdUri, string::linkify, LoadingState},
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
        /// The canonical alias of this room.
        alias: RefCell<Option<OwnedRoomAliasId>>,
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
        /// The loading state.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RemoteRoom {
        const NAME: &'static str = "RemoteRoom";
        type Type = super::RemoteRoom;
        type ParentType = PillSource;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RemoteRoom {}

    impl PillSourceImpl for RemoteRoom {
        fn identifier(&self) -> String {
            self.uri.get().unwrap().id.to_string()
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
        }

        /// Set the Matrix URI of this room.
        pub(super) fn set_uri(&self, uri: MatrixRoomIdUri) {
            self.uri
                .set(uri)
                .expect("Matrix URI should be uninitialized");

            self.update_display_name();

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }

        /// The Matrix URI of this room.
        pub(super) fn uri(&self) -> &MatrixRoomIdUri {
            self.uri.get().expect("Matrix URI should be initialized")
        }

        /// Set the alias of this room.
        fn set_alias(&self, alias: Option<OwnedRoomAliasId>) {
            if *self.alias.borrow() == alias {
                return;
            }

            self.alias.replace(alias);
            self.update_display_name();
        }

        /// The canonical alias of this room.
        pub(super) fn alias(&self) -> Option<OwnedRoomAliasId> {
            self.alias
                .borrow()
                .clone()
                .or_else(|| self.uri().id.clone().try_into().ok())
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
                .or_else(|| self.alias.borrow().as_ref().map(ToString::to_string))
                .unwrap_or_else(|| self.identifier());

            self.obj().set_display_name(display_name);
        }

        /// Set the topic of this room.
        fn set_topic(&self, topic: Option<String>) {
            let topic =
                topic.filter(|s| !s.is_empty() && s.find(|c: char| !c.is_whitespace()).is_some());

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
            self.set_alias(data.canonical_alias);
            self.set_name(data.name);
            self.set_topic(data.topic);
            self.set_joined_members_count(data.num_joined_members.try_into().unwrap_or(u32::MAX));

            if let Some(image) = self.obj().avatar_data().image() {
                image.set_uri_and_info(data.avatar_url, None);
            }

            self.set_loading_state(LoadingState::Ready);
        }

        /// Load the data of this room.
        async fn load(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            self.set_loading_state(LoadingState::Loading);

            let uri = self.uri();
            let client = session.client();

            let room_id = match OwnedRoomId::try_from(uri.id.clone()) {
                Ok(room_id) => room_id,
                Err(alias) => {
                    let client_clone = client.clone();
                    let handle =
                        spawn_tokio!(async move { client_clone.resolve_room_alias(&alias).await });

                    match handle.await.unwrap() {
                        Ok(response) => response.room_id,
                        Err(error) => {
                            warn!("Could not resolve room alias `{}`: {error}", uri.id);
                            self.set_loading_state(LoadingState::Error);
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
            let handle = spawn_tokio!(async move { client.send(request).await });

            match handle.await.unwrap() {
                Ok(response) => {
                    if let Some(chunk) = response
                        .rooms
                        .into_iter()
                        .next()
                        .filter(|c| c.room_id == room_id)
                    {
                        self.update_data(chunk);
                    } else {
                        debug!("Endpoint did not return requested room");
                        self.set_loading_state(LoadingState::Error);
                    }
                }
                Err(error) => {
                    warn!("Could not get room details for room `{}`: {error}", uri.id);
                    self.set_loading_state(LoadingState::Error);
                }
            }
        }
    }
}

glib::wrapper! {
    /// A Room that can only be updated by making remote calls, i.e. it won't be updated via sync.
    pub struct RemoteRoom(ObjectSubclass<imp::RemoteRoom>)
        @extends PillSource;
}

impl RemoteRoom {
    pub(crate) fn new(session: &Session, uri: MatrixRoomIdUri) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();
        obj.imp().set_uri(uri);
        obj
    }

    /// The Matrix URI of this room.
    pub(crate) fn uri(&self) -> &MatrixRoomIdUri {
        self.imp().uri()
    }

    /// The canonical alias of this room.
    pub(crate) fn alias(&self) -> Option<OwnedRoomAliasId> {
        self.imp().alias()
    }
}

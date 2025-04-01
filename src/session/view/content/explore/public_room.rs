use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{directory::PublicRoomsChunk, OwnedServerName};

use crate::{
    components::{AvatarData, AvatarImage, AvatarUriSource},
    session::model::{Room, RoomList},
    utils::BoundConstructOnlyObject,
};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PublicRoom)]
    pub struct PublicRoom {
        /// The list of rooms in the current session.
        #[property(get, set = Self::set_room_list, construct_only)]
        room_list: BoundConstructOnlyObject<RoomList>,
        /// The server that returned the room.
        server: OnceCell<OwnedServerName>,
        /// The data for this room.
        data: OnceCell<PublicRoomsChunk>,
        /// The avatar data for this room.
        #[property(get)]
        avatar_data: OnceCell<AvatarData>,
        /// The `Room` object for this room, if the user is already a member of
        /// this room.
        #[property(get)]
        room: RefCell<Option<Room>>,
        /// Whether the room is pending.
        ///
        /// A room is pending when the user clicked to join it.
        #[property(get)]
        is_pending: Cell<bool>,
        room_added_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PublicRoom {
        const NAME: &'static str = "PublicRoom";
        type Type = super::PublicRoom;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PublicRoom {
        fn dispose(&self) {
            if let Some(handler) = self.room_added_handler.take() {
                self.room_list.obj().disconnect(handler);
            }
        }
    }

    impl PublicRoom {
        /// Set the list of rooms in the current session.
        fn set_room_list(&self, room_list: RoomList) {
            let pending_rooms_changed_handler = room_list.connect_pending_rooms_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_is_pending();
                }
            ));

            self.room_list
                .set(room_list, vec![pending_rooms_changed_handler]);
        }

        /// Set the data for this room.
        pub(super) fn set_server_and_data(
            &self,
            server: Option<OwnedServerName>,
            data: PublicRoomsChunk,
        ) {
            let room_list = self.room_list.obj();
            let Some(session) = room_list.session() else {
                return;
            };

            if let Some(server) = server {
                self.server
                    .set(server)
                    .expect("server should not be initialized");
            }

            let data = self.data.get_or_init(|| data);

            let avatar_data = AvatarData::with_image(AvatarImage::new(
                &session,
                AvatarUriSource::Room,
                data.avatar_url.clone(),
                None,
            ));

            if let Some(display_name) = data.name.clone() {
                avatar_data.set_display_name(display_name);
            }

            self.avatar_data
                .set(avatar_data)
                .expect("avatar data was not initialized");

            if let Some(room) = room_list.get(&data.room_id) {
                self.set_room(Some(room));
            } else {
                let room_id = data.room_id.clone();
                let room_added_handler = room_list.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |room_list, _, _, _| {
                        if let Some(room) = room_list.get(&room_id) {
                            if let Some(handler) = imp.room_added_handler.take() {
                                imp.set_room(Some(room));
                                room_list.disconnect(handler);
                            }
                        }
                    }
                ));

                self.room_added_handler.replace(Some(room_added_handler));
            }

            self.update_is_pending();
        }

        /// The server that returned this room.
        pub(super) fn server(&self) -> Option<&OwnedServerName> {
            self.server.get()
        }

        /// The data for this room.
        pub(super) fn data(&self) -> &PublicRoomsChunk {
            self.data.get().expect("data should be initialized")
        }

        /// Set the [`Room`] for this room.
        fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }

            self.room.replace(room);
            self.obj().notify_room();
        }

        /// Update whether this room is pending.
        fn update_is_pending(&self) {
            let identifier = (*self.data().room_id).into();
            let is_pending = self.room_list.obj().is_pending_room(identifier);

            self.set_is_pending(is_pending);
        }

        /// Set whether this room is pending.
        fn set_is_pending(&self, is_pending: bool) {
            if self.is_pending.get() == is_pending {
                return;
            }

            self.is_pending.set(is_pending);
            self.obj().notify_is_pending();
        }
    }
}

glib::wrapper! {
    /// A room in a homeserver's public directory.
    pub struct PublicRoom(ObjectSubclass<imp::PublicRoom>);
}

impl PublicRoom {
    pub fn new(
        room_list: &RoomList,
        server: Option<OwnedServerName>,
        data: PublicRoomsChunk,
    ) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room-list", room_list)
            .build();
        obj.imp().set_server_and_data(server, data);
        obj
    }

    /// The server that returned this room.
    pub(crate) fn server(&self) -> Option<&OwnedServerName> {
        self.imp().server()
    }

    /// The data for this room.
    pub(crate) fn data(&self) -> &PublicRoomsChunk {
        self.imp().data()
    }

    /// The display name for this room.
    pub(crate) fn display_name(&self) -> String {
        let data = self.imp().data();

        data.name
            .clone()
            .or_else(|| data.canonical_alias.as_ref().map(ToString::to_string))
            .unwrap_or_else(|| data.room_id.to_string())
    }
}

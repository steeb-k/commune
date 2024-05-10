use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::directory::PublicRoomsChunk;

use crate::{
    components::{AvatarData, AvatarImage, AvatarUriSource},
    session::model::{Room, RoomList},
};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use glib::signal::SignalHandlerId;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PublicRoom)]
    pub struct PublicRoom {
        /// The list of rooms in this session.
        #[property(get, construct_only)]
        pub room_list: OnceCell<RoomList>,
        /// The server that returned the room.
        #[property(get, construct_only)]
        pub server: OnceCell<String>,
        pub matrix_public_room: OnceCell<PublicRoomsChunk>,
        /// The [`AvatarData`] of this room.
        #[property(get)]
        pub avatar_data: OnceCell<AvatarData>,
        /// The `Room` object for this room, if the user is already a member of
        /// this room.
        #[property(get)]
        pub room: RefCell<Option<Room>>,
        /// Whether the room is pending.
        ///
        /// A room is pending when the user clicked to join it.
        #[property(get)]
        pub pending: Cell<bool>,
        pub room_handler: RefCell<Option<SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PublicRoom {
        const NAME: &'static str = "PublicRoom";
        type Type = super::PublicRoom;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PublicRoom {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let avatar_data = if let Some(session) = obj.room_list().session() {
                AvatarData::with_image(AvatarImage::new(&session, None, AvatarUriSource::Room))
            } else {
                AvatarData::new()
            };

            self.avatar_data.set(avatar_data).unwrap();

            obj.room_list()
                .connect_pending_rooms_changed(clone!(@weak obj => move |_| {
                    let Some(matrix_public_room) = obj.matrix_public_room() else {
                        return;
                    };

                        obj.set_pending(obj.room_list()
                            .is_pending_room((*matrix_public_room.room_id).into()));
                }));
        }

        fn dispose(&self) {
            if let Some(handler_id) = self.room_handler.take() {
                self.obj().room_list().disconnect(handler_id);
            }
        }
    }
}

glib::wrapper! {
    /// A room in a homeserver's public directory.
    pub struct PublicRoom(ObjectSubclass<imp::PublicRoom>);
}

impl PublicRoom {
    pub fn new(room_list: &RoomList, server: &str) -> Self {
        glib::Object::builder()
            .property("room-list", room_list)
            .property("server", server)
            .build()
    }

    /// Set the `Room` object for this room.
    fn set_room(&self, room: Room) {
        self.imp().room.replace(Some(room));
        self.notify_room();
    }

    /// Set whether this room is pending.
    fn set_pending(&self, pending: bool) {
        if self.pending() == pending {
            return;
        }

        self.imp().pending.set(pending);
        self.notify_pending();
    }

    pub fn set_matrix_public_room(&self, room: PublicRoomsChunk) {
        let imp = self.imp();

        if let Some(display_name) = room.name.clone() {
            self.avatar_data().set_display_name(display_name);
        }
        self.avatar_data()
            .image()
            .unwrap()
            .set_uri(room.avatar_url.clone().map(String::from));

        if let Some(room) = self.room_list().get(&room.room_id) {
            self.set_room(room);
        } else {
            let room_id = room.room_id.clone();
            let handler_id = self.room_list().connect_items_changed(
                clone!(@weak self as obj => move |room_list, _, _, _| {
                    if let Some(room) = room_list.get(&room_id) {
                        if let Some(handler_id) = obj.imp().room_handler.take() {
                            obj.set_room(room);
                            room_list.disconnect(handler_id);
                        }
                    }
                }),
            );

            imp.room_handler.replace(Some(handler_id));
        }

        self.set_pending(self.room_list().is_pending_room((*room.room_id).into()));

        imp.matrix_public_room.set(room).unwrap();
    }

    pub fn matrix_public_room(&self) -> Option<&PublicRoomsChunk> {
        self.imp().matrix_public_room.get()
    }

    /// The display name for this room.
    ///
    /// Returns an empty string if there is no matrix public room.
    pub fn display_name(&self) -> String {
        let Some(matrix_public_room) = self.matrix_public_room() else {
            return String::new();
        };

        matrix_public_room
            .name
            .as_deref()
            .or(matrix_public_room
                .canonical_alias
                .as_ref()
                .map(|a| a.as_str()))
            .unwrap_or_else(|| matrix_public_room.room_id.as_str())
            .to_owned()
    }
}

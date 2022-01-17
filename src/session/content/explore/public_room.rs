use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::{
    directory::PublicRoomsChunk,
    identifiers::{RoomId, RoomOrAliasId},
};

use crate::session::{room::Room, Avatar, RoomList};

mod imp {
    use super::*;
    use glib::signal::SignalHandlerId;
    use once_cell::{sync::Lazy, unsync::OnceCell};
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default)]
    pub struct PublicRoom {
        pub room_list: OnceCell<RoomList>,
        pub matrix_public_room: OnceCell<PublicRoomsChunk>,
        pub avatar: OnceCell<Avatar>,
        pub room: OnceCell<Room>,
        pub is_pending: Cell<bool>,
        pub room_handler: RefCell<Option<SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PublicRoom {
        const NAME: &'static str = "PublicRoom";
        type Type = super::PublicRoom;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for PublicRoom {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "room-list",
                        "Room List",
                        "The list of rooms in this session",
                        RoomList::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_object(
                        "room",
                        "Room",
                        "The room, this is only set if the user is already a member",
                        Room::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_boolean(
                        "pending",
                        "Pending",
                        "A room is pending when the user already clicked to join a room",
                        false,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "avatar",
                        "Avatar",
                        "The Avatar of this room",
                        Avatar::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "room-list" => self
                    .room_list
                    .set(value.get::<RoomList>().unwrap())
                    .unwrap(),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "room-list" => obj.room_list().to_value(),
                "avatar" => obj.avatar().to_value(),
                "room" => obj.room().to_value(),
                "pending" => obj.is_pending().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            self.avatar
                .set(Avatar::new(&obj.room_list().session(), None))
                .unwrap();

            obj.room_list()
                .connect_pending_rooms_changed(clone!(@weak obj => move |_| {
                    if let Some(matrix_public_room) = obj.matrix_public_room() {
                        obj.set_pending(obj.room_list().session()
                        .room_list()
                        .is_pending_room((&*matrix_public_room.room_id).into()));
                    }
                }));
        }

        fn dispose(&self, obj: &Self::Type) {
            if let Some(handler_id) = self.room_handler.take() {
                obj.room_list().disconnect(handler_id);
            }
        }
    }
}

glib::wrapper! {
    pub struct PublicRoom(ObjectSubclass<imp::PublicRoom>);
}

impl PublicRoom {
    pub fn new(room_list: &RoomList) -> Self {
        glib::Object::new(&[("room-list", room_list)]).expect("Failed to create Room")
    }

    pub fn room_list(&self) -> &RoomList {
        let priv_ = imp::PublicRoom::from_instance(self);
        priv_.room_list.get().unwrap()
    }

    pub fn avatar(&self) -> &Avatar {
        let priv_ = imp::PublicRoom::from_instance(self);
        priv_.avatar.get().unwrap()
    }

    /// The room if the user is already a member of this room.
    pub fn room(&self) -> Option<&Room> {
        let priv_ = imp::PublicRoom::from_instance(self);
        priv_.room.get()
    }

    fn set_room(&self, room: Room) {
        let priv_ = imp::PublicRoom::from_instance(self);
        priv_.room.set(room).unwrap();
        self.notify("room");
    }

    fn set_pending(&self, is_pending: bool) {
        let priv_ = imp::PublicRoom::from_instance(self);

        if self.is_pending() == is_pending {
            return;
        }

        priv_.is_pending.set(is_pending);
        self.notify("pending");
    }

    pub fn is_pending(&self) -> bool {
        let priv_ = imp::PublicRoom::from_instance(self);
        priv_.is_pending.get()
    }

    pub fn set_matrix_public_room(&self, room: PublicRoomsChunk) {
        let priv_ = imp::PublicRoom::from_instance(self);

        let display_name = room.name.clone().map(Into::into);
        self.avatar().set_display_name(display_name);
        self.avatar().set_url(room.avatar_url.clone());

        if let Some(room) = self.room_list().get(&room.room_id) {
            self.set_room(room);
        } else {
            let room_id = room.room_id.clone();
            let handler_id = self.room_list().connect_items_changed(
                clone!(@weak self as obj => move |room_list, _, _, _| {
                    if let Some(room) = room_list.get(&room_id) {
                        let priv_ = imp::PublicRoom::from_instance(&obj);
                        if let Some(handler_id) = priv_.room_handler.take() {
                            obj.set_room(room);
                            room_list.disconnect(handler_id);
                        }
                    }
                }),
            );

            priv_.room_handler.replace(Some(handler_id));
        }

        self.set_pending(self.room_list().is_pending_room((&*room.room_id).into()));

        priv_.matrix_public_room.set(room).unwrap();
    }

    pub fn matrix_public_room(&self) -> Option<&PublicRoomsChunk> {
        let priv_ = imp::PublicRoom::from_instance(self);
        priv_.matrix_public_room.get()
    }

    pub fn join_or_view(&self) {
        if let Some(room) = self.room() {
            self.room_list().session().select_room(Some(room.clone()));
        } else if let Some(matrix_public_room) = self.matrix_public_room() {
            let room_id: &RoomId = matrix_public_room.room_id.as_ref();
            self.room_list()
                .join_by_id_or_alias(<&RoomOrAliasId>::from(room_id).to_owned());
        }
    }
}

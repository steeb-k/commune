use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use indexmap::map::IndexMap;
use matrix_sdk::{deserialized_responses::Rooms as ResponseRooms, identifiers::RoomId, Client};

use crate::session::{room::Room, user::User};

mod imp {
    use once_cell::unsync::OnceCell;
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct RoomList {
        pub list: RefCell<IndexMap<RoomId, Room>>,
        pub client: OnceCell<Client>,
        pub user: OnceCell<User>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomList {
        const NAME: &'static str = "RoomList";
        type Type = super::RoomList;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for RoomList {}

    impl ListModelImpl for RoomList {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            Room::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.borrow().len() as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .values()
                .nth(position as usize)
                .map(glib::object::Cast::upcast_ref::<glib::Object>)
                .cloned()
        }
    }
}

glib::wrapper! {
    pub struct RoomList(ObjectSubclass<imp::RoomList>)
        @implements gio::ListModel;
}

impl RoomList {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create RoomList")
    }

    pub fn set_client(&self, client: Client) -> Result<(), Client> {
        let priv_ = imp::RoomList::from_instance(&self);
        priv_.client.set(client)
    }

    pub fn set_user(&self, user: User) -> Result<(), User> {
        let priv_ = imp::RoomList::from_instance(&self);
        priv_.user.set(user)
    }

    pub fn get(&self, room_id: &RoomId) -> Option<Room> {
        let priv_ = imp::RoomList::from_instance(&self);
        priv_.list.borrow().get(room_id).cloned()
    }

    fn get_full(&self, room_id: &RoomId) -> Option<(usize, RoomId, Room)> {
        let priv_ = imp::RoomList::from_instance(&self);
        priv_
            .list
            .borrow()
            .get_full(room_id)
            .map(|(pos, room_id, room)| (pos, room_id.clone(), room.clone()))
    }

    pub fn contains_key(&self, room_id: &RoomId) -> bool {
        let priv_ = imp::RoomList::from_instance(&self);
        priv_.list.borrow().contains_key(room_id)
    }

    pub fn remove(&self, room_id: &RoomId) {
        let priv_ = imp::RoomList::from_instance(&self);

        let removed = {
            let mut list = priv_.list.borrow_mut();

            list.shift_remove_full(room_id)
        };

        if let Some((position, _, _)) = removed {
            self.items_changed(position as u32, 1, 0);
        }
    }

    fn items_added(&self, added: usize) {
        let priv_ = imp::RoomList::from_instance(&self);

        let list = priv_.list.borrow();

        let position = list.len() - added;

        for (_room_id, room) in list.iter().skip(position) {
            room.connect_notify_local(
                Some("category"),
                clone!(@weak self as obj => move |r, _| {
                    if let Some((position, _, _)) = obj.get_full(&r.matrix_room_id()) {
                        obj.items_changed(position as u32, 1, 1);
                    }
                }),
            );
        }

        self.items_changed(position as u32, 0, added as u32);
    }

    pub fn load(&self) {
        let priv_ = imp::RoomList::from_instance(&self);

        let matrix_rooms = priv_.client.get().unwrap().rooms();
        let added = matrix_rooms.len();

        if added > 0 {
            {
                let mut list = priv_.list.borrow_mut();
                for matrix_room in matrix_rooms {
                    let room = Room::new(matrix_room, priv_.user.get().unwrap());

                    list.insert(room.matrix_room_id(), room);
                }
            }

            self.items_added(added);
        }
    }

    pub fn handle_response_rooms(&self, rooms: ResponseRooms) {
        let priv_ = imp::RoomList::from_instance(&self);

        let mut added = 0;

        for (room_id, left_room) in rooms.leave {
            let matrix_room = priv_.client.get().unwrap().get_room(&room_id).unwrap();

            let room = priv_
                .list
                .borrow_mut()
                .entry(room_id)
                .or_insert_with(|| {
                    added += 1;
                    Room::new(matrix_room.clone(), priv_.user.get().unwrap())
                })
                .clone();

            room.handle_left_response(left_room, matrix_room);
        }

        for (room_id, joined_room) in rooms.join {
            let matrix_room = priv_.client.get().unwrap().get_room(&room_id).unwrap();

            let room = priv_
                .list
                .borrow_mut()
                .entry(room_id)
                .or_insert_with(|| {
                    added += 1;
                    Room::new(matrix_room.clone(), priv_.user.get().unwrap())
                })
                .clone();

            room.handle_joined_response(joined_room, matrix_room);
        }

        for (room_id, invited_room) in rooms.invite {
            let matrix_room = priv_.client.get().unwrap().get_room(&room_id).unwrap();

            let room = priv_
                .list
                .borrow_mut()
                .entry(room_id)
                .or_insert_with(|| {
                    added += 1;
                    Room::new(matrix_room.clone(), priv_.user.get().unwrap())
                })
                .clone();

            room.handle_invited_response(invited_room, matrix_room);
        }

        if added > 0 {
            self.items_added(added);
        }
    }
}

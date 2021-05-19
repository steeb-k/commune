use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use indexmap::map::IndexMap;
use matrix_sdk::identifiers::RoomId;

use crate::session::room::Room;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Debug)]
    pub struct RoomList {
        pub list: RefCell<IndexMap<RoomId, Room>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomList {
        const NAME: &'static str = "RoomList";
        type Type = super::RoomList;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);

        fn new() -> Self {
            Self {
                list: Default::default(),
            }
        }
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

impl Default for RoomList {
    fn default() -> Self {
        Self::new()
    }
}

impl RoomList {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create RoomList")
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

    pub fn insert(&self, rooms: Vec<(RoomId, Room)>) {
        let priv_ = imp::RoomList::from_instance(&self);

        let rooms: Vec<(RoomId, Room)> = {
            rooms
                .into_iter()
                .filter(|(room_id, _)| !priv_.list.borrow().contains_key(room_id))
                .collect()
        };

        let added = rooms.len();

        if added > 0 {
            let position = priv_.list.borrow().len();

            {
                let mut list = priv_.list.borrow_mut();
                for (room_id, room) in rooms {
                    room.connect_notify_local(
                        Some("category"),
                        clone!(@weak self as obj => move |r, _| {
                            if let Some((position, _, _)) = obj.get_full(r.matrix_room().room_id()) {
                                obj.items_changed(position as u32, 1, 1);
                            }
                        }),
                    );
                    list.insert(room_id, room);
                }
            }

            self.items_changed(position as u32, 0, added as u32);
        }
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
}

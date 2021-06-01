use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use indexmap::map::IndexMap;
use matrix_sdk::{deserialized_responses::Rooms as ResponseRooms, identifiers::RoomId};

use crate::session::{room::Room, Session};

mod imp {
    use once_cell::sync::{Lazy, OnceCell};
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct RoomList {
        pub list: RefCell<IndexMap<RoomId, Room>>,
        pub session: OnceCell<Session>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomList {
        const NAME: &'static str = "RoomList";
        type Type = super::RoomList;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for RoomList {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "session",
                    "Session",
                    "The session",
                    Session::static_type(),
                    glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                )]
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
                "session" => self.session.set(value.get().unwrap()).unwrap(),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "session" => obj.session().to_value(),
                _ => unimplemented!(),
            }
        }
    }

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
    pub fn new(session: &Session) -> Self {
        glib::Object::new(&[("session", session)]).expect("Failed to create RoomList")
    }

    pub fn session(&self) -> &Session {
        let priv_ = imp::RoomList::from_instance(&self);
        priv_.session.get().unwrap()
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
                    if let Some((position, _, _)) = obj.get_full(&r.room_id()) {
                        obj.items_changed(position as u32, 1, 1);
                    }
                }),
            );
        }

        self.items_changed(position as u32, 0, added as u32);
    }

    /// Loads the state from the `Store`.
    ///
    /// Note that the `Store` currently doesn't store all events, therefore, we aren't really
    /// loading much via this function.
    pub fn load(&self) {
        let priv_ = imp::RoomList::from_instance(&self);
        let session = self.session();
        let client = session.client();
        let matrix_rooms = client.rooms();
        let added = matrix_rooms.len();

        if added > 0 {
            {
                let mut list = priv_.list.borrow_mut();
                for matrix_room in matrix_rooms {
                    let room_id = matrix_room.room_id().to_owned();
                    let room = Room::new(session, &room_id);
                    list.insert(room_id, room);
                }
            }

            self.items_added(added);
        }
    }

    pub fn handle_response_rooms(&self, rooms: ResponseRooms) {
        let priv_ = imp::RoomList::from_instance(&self);
        let session = self.session();

        let mut added = 0;

        for (room_id, left_room) in rooms.leave {
            let room = priv_
                .list
                .borrow_mut()
                .entry(room_id.clone())
                .or_insert_with(|| {
                    added += 1;
                    Room::new(session, &room_id)
                })
                .clone();

            room.handle_left_response(left_room);
        }

        for (room_id, joined_room) in rooms.join {
            let room = priv_
                .list
                .borrow_mut()
                .entry(room_id.clone())
                .or_insert_with(|| {
                    added += 1;
                    Room::new(session, &room_id)
                })
                .clone();

            room.handle_joined_response(joined_room);
        }

        for (room_id, invited_room) in rooms.invite {
            let room = priv_
                .list
                .borrow_mut()
                .entry(room_id.clone())
                .or_insert_with(|| {
                    added += 1;
                    Room::new(session, &room_id)
                })
                .clone();

            room.handle_invited_response(invited_room);
        }

        if added > 0 {
            self.items_added(added);
        }
    }
}

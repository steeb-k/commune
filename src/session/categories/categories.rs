use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use std::collections::{hash_map::Entry, HashMap};

use crate::session::{
    categories::{Category, CategoryType},
    room::Room,
};

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Debug)]
    pub struct Categories {
        pub list: [Category; 5],
        pub room_map: RefCell<HashMap<Room, CategoryType>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Categories {
        const NAME: &'static str = "Categories";
        type Type = super::Categories;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);

        fn new() -> Self {
            Self {
                list: [
                    Category::new(CategoryType::Invited),
                    Category::new(CategoryType::Favorite),
                    Category::new(CategoryType::Normal),
                    Category::new(CategoryType::LowPriority),
                    Category::new(CategoryType::Left),
                ],
                room_map: Default::default(),
            }
        }
    }

    impl ObjectImpl for Categories {}

    impl ListModelImpl for Categories {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            Category::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.len() as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .get(position as usize)
                .map(glib::object::Cast::upcast_ref::<glib::Object>)
                .cloned()
        }
    }
}

glib::wrapper! {
    pub struct Categories(ObjectSubclass<imp::Categories>)
        @implements gio::ListModel;
}

impl Default for Categories {
    fn default() -> Self {
        Self::new()
    }
}

impl Categories {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create Categories")
    }

    pub fn append(&self, rooms: Vec<Room>) {
        let priv_ = imp::Categories::from_instance(&self);

        for room in rooms {
            if priv_.room_map.borrow().contains_key(&room) {
                return;
            }

            room.connect_notify_local(
                Some("category"),
                clone!(@weak self as obj => move |room, _| {
                    obj.move_room(room);
                }),
            );
            // TODO: Add all rooms at once
            self.move_room(&room);
        }
    }

    fn move_room(&self, room: &Room) {
        let priv_ = imp::Categories::from_instance(&self);
        let mut room_map = priv_.room_map.borrow_mut();

        let entry = room_map.entry(room.clone());

        match entry {
            Entry::Occupied(mut entry) => {
                if entry.get() != &room.category() {
                    entry.insert(room.category());
                    self.remove_from_category(entry.get(), room);
                    self.add_to_category(entry.get(), room);
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(room.category());
                self.add_to_category(&room.category(), room);
            }
        }
    }

    fn add_to_category(&self, type_: &CategoryType, room: &Room) {
        let priv_ = imp::Categories::from_instance(&self);

        let position = priv_.list.iter().position(|item| item.type_() == *type_);
        if let Some(position) = position {
            priv_.list.get(position).unwrap().append(room);
        }
    }

    fn remove_from_category(&self, type_: &CategoryType, room: &Room) {
        let priv_ = imp::Categories::from_instance(&self);

        let position = priv_.list.iter().position(|item| item.type_() == *type_);
        if let Some(position) = position {
            priv_.list.get(position).unwrap().remove(room);
        }
    }
}

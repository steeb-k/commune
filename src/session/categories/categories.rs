use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use std::collections::HashMap;

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

        let rooms: Vec<Room> = {
            let room_map = priv_.room_map.borrow();
            rooms
                .into_iter()
                .filter(|room| !room_map.contains_key(&room))
                .collect()
        };

        let rooms_by_category = rooms.into_iter().fold(HashMap::new(), |mut acc, room| {
            acc.entry(room.category()).or_insert(vec![]).push(room);
            acc
        });
        let mut room_map = priv_.room_map.borrow_mut();
        for (category_type, rooms) in rooms_by_category {
            for room in &rooms {
                room_map.insert(room.clone(), category_type);
                room.connect_notify_local(
                    Some("category"),
                    clone!(@weak self as obj => move |room, _| {
                        obj.move_room(room);
                    }),
                );
            }

            self.find_category_by_type(category_type)
                .append_batch(rooms);
        }
    }

    fn find_category_by_type(&self, type_: CategoryType) -> &Category {
        let priv_ = imp::Categories::from_instance(&self);
        let position = priv_.list.iter().position(|item| item.type_() == type_);
        priv_.list.get(position.unwrap()).unwrap()
    }

    fn move_room(&self, room: &Room) {
        let priv_ = imp::Categories::from_instance(&self);
        let mut room_map = priv_.room_map.borrow_mut();

        if let Some(old_category_type) = room_map.remove(&room) {
            self.find_category_by_type(old_category_type).remove(room);
        }

        room_map.insert(room.clone(), room.category());
        self.find_category_by_type(room.category()).append(room);
    }
}

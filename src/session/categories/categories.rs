use gtk::{gio, glib, prelude::*, subclass::prelude::*};

use crate::session::categories::{Category, CategoryType, RoomList};

mod imp {
    use super::*;

    #[derive(Debug)]
    pub struct Categories {
        pub list: [Category; 5],
        pub room_list: RoomList,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Categories {
        const NAME: &'static str = "Categories";
        type Type = super::Categories;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);

        fn new() -> Self {
            let room_list = RoomList::new();

            Self {
                list: [
                    Category::new(CategoryType::Invited, &room_list),
                    Category::new(CategoryType::Favorite, &room_list),
                    Category::new(CategoryType::Normal, &room_list),
                    Category::new(CategoryType::LowPriority, &room_list),
                    Category::new(CategoryType::Left, &room_list),
                ],
                room_list,
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

    pub fn room_list(&self) -> &RoomList {
        let priv_ = imp::Categories::from_instance(&self);
        &priv_.room_list
    }
}

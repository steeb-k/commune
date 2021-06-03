use gtk::{gio, glib, prelude::*, subclass::prelude::*};

use crate::session::{room::RoomType, room_list::RoomList, sidebar::Category};

mod imp {
    use once_cell::unsync::OnceCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct Categories {
        pub list: OnceCell<[Category; 5]>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Categories {
        const NAME: &'static str = "Categories";
        type Type = super::Categories;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for Categories {}

    impl ListModelImpl for Categories {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            Category::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.get().map(|l| l.len()).unwrap_or(0) as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .get()
                .and_then(|l| l.get(position as usize))
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

    pub fn set_room_list(&self, room_list: &RoomList) {
        let priv_ = imp::Categories::from_instance(&self);

        priv_
            .list
            .set([
                Category::new(RoomType::Invited, room_list),
                Category::new(RoomType::Favorite, room_list),
                Category::new(RoomType::Normal, room_list),
                Category::new(RoomType::LowPriority, room_list),
                Category::new(RoomType::Left, room_list),
            ])
            .unwrap();

        self.items_changed(0, 0, 5);
    }
}

use crate::session::sidebar::Category;
use gtk::subclass::prelude::*;
use gtk::{self, gio, glib, glib::clone, prelude::*};
use matrix_sdk::identifiers::RoomId;

mod imp {
    use super::*;
    use gio::subclass::prelude::*;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct CategoryList {
        pub list: RefCell<Vec<Category>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CategoryList {
        const NAME: &'static str = "CategoryList";
        type Type = super::CategoryList;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for CategoryList {}

    impl ListModelImpl for CategoryList {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            Category::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.borrow().len() as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get(position as usize)
                .map(glib::object::Cast::upcast_ref::<glib::Object>)
                .cloned()
        }
    }
}

glib::wrapper! {
    pub struct CategoryList(ObjectSubclass<imp::CategoryList>)
        @implements gio::ListModel;
}
// TODO allow moving between categories
// TODO allow selection only in one category

impl CategoryList {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create CategoryList")
    }

    pub fn update(&self, room_id: &RoomId) {
        let priv_ = imp::CategoryList::from_instance(self);
        let list = priv_.list.borrow();
        for category in list.iter() {
            category.update(room_id);
        }
    }

    pub fn append(&self, category: Category) {
        let priv_ = imp::CategoryList::from_instance(self);
        let index = {
            let mut list = priv_.list.borrow_mut();
            category.connect_selection_changed(
                clone!(@weak self as obj => move |category, position, _| {
                    if category.is_selected(position) {
                        obj.unselect_other_lists(&category);
                    }
                }),
            );
            list.push(category);
            list.len() - 1
        };
        self.items_changed(index as u32, 0, 1);
    }

    fn unselect_other_lists(&self, category: &Category) {
        let priv_ = imp::CategoryList::from_instance(self);
        let list = priv_.list.borrow();

        for item in list.iter() {
            if item != category {
                item.unselect();
            }
        }
    }

    pub fn append_batch(&self, batch: &[Category]) {
        let priv_ = imp::CategoryList::from_instance(self);
        let index = {
            let mut list = priv_.list.borrow_mut();
            let index = list.len();
            for category in batch.iter() {
                category.connect_selection_changed(
                    clone!(@weak self as obj => move |category, position, _| {
                        if category.is_selected(position) {
                            obj.unselect_other_lists(&category);
                        }
                    }),
                );
                list.push(category.clone());
            }
            index
        };
        self.items_changed(index as u32, 0, batch.len() as u32);
    }
}

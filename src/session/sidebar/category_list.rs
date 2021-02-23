use crate::session::sidebar::FrctlCategory;
use gtk::subclass::prelude::*;
use gtk::{self, gio, glib, glib::clone, prelude::*};
use matrix_sdk::identifiers::RoomId;

mod imp {
    use super::*;
    use gio::subclass::prelude::*;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct FrctlCategoryList {
        pub list: RefCell<Vec<FrctlCategory>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FrctlCategoryList {
        const NAME: &'static str = "FrctlCategoryList";
        type Type = super::FrctlCategoryList;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for FrctlCategoryList {}

    impl ListModelImpl for FrctlCategoryList {
        fn get_item_type(&self, _list_model: &Self::Type) -> glib::Type {
            FrctlCategory::static_type()
        }
        fn get_n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.borrow().len() as u32
        }
        fn get_item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get(position as usize)
                .map(glib::object::Cast::upcast_ref::<glib::Object>)
                .cloned()
        }
    }
}

glib::wrapper! {
    pub struct FrctlCategoryList(ObjectSubclass<imp::FrctlCategoryList>)
        @implements gio::ListModel;
}
// TODO allow moving between categories
// TODO allow selection only in one category

impl FrctlCategoryList {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create FrctlCategoryList")
    }

    pub fn update(&self, room_id: &RoomId) {
        let priv_ = imp::FrctlCategoryList::from_instance(self);
        let list = priv_.list.borrow();
        for category in list.iter() {
            category.update(room_id);
        }
    }

    pub fn append(&self, category: FrctlCategory) {
        let priv_ = imp::FrctlCategoryList::from_instance(self);
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

    fn unselect_other_lists(&self, category: &FrctlCategory) {
        let priv_ = imp::FrctlCategoryList::from_instance(self);
        let list = priv_.list.borrow();

        for item in list.iter() {
            if item != category {
                item.unselect();
            }
        }
    }

    pub fn append_batch(&self, batch: &[FrctlCategory]) {
        let priv_ = imp::FrctlCategoryList::from_instance(self);
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

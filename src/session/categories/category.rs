use gtk::{gio, glib, prelude::*, subclass::prelude::*};

use crate::session::{categories::CategoryType, room::Room};

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default)]
    pub struct Category {
        pub list: RefCell<Vec<Room>>,
        pub type_: Cell<CategoryType>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Category {
        const NAME: &'static str = "Category";
        type Type = super::Category;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for Category {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_enum(
                        "type",
                        "Type",
                        "The type of this category",
                        CategoryType::static_type(),
                        CategoryType::default() as i32,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name of this category",
                        None,
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
                "type" => {
                    let type_ = value.get().unwrap();
                    self.type_.set(type_);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "type" => obj.type_().to_value(),
                "display-name" => obj.type_().to_string().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl ListModelImpl for Category {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            Room::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.borrow().len() as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get(position as usize)
                .map(|o| o.clone().upcast::<glib::Object>())
        }
    }
}

glib::wrapper! {
    pub struct Category(ObjectSubclass<imp::Category>)
        @implements gio::ListModel;
}

impl Category {
    pub fn new(type_: CategoryType) -> Self {
        glib::Object::new(&[("type", &type_)]).expect("Failed to create Category")
    }

    pub fn type_(&self) -> CategoryType {
        let priv_ = imp::Category::from_instance(self);
        priv_.type_.get()
    }

    pub fn append(&self, room: &Room) {
        let priv_ = imp::Category::from_instance(self);
        let index = {
            let mut list = priv_.list.borrow_mut();
            let index = list.len();
            list.push(room.clone());
            index
        };
        self.items_changed(index as u32, 0, 1);
    }

    pub fn append_batch(&self, rooms: Vec<Room>) {
        let priv_ = imp::Category::from_instance(self);
        let added = rooms.len();
        let index = {
            let mut list = priv_.list.borrow_mut();
            let index = list.len();
            list.reserve(added);
            for room in rooms {
                list.push(room);
            }
            index
        };
        self.items_changed(index as u32, 0, added as u32);
    }

    pub fn remove(&self, room: &Room) {
        let priv_ = imp::Category::from_instance(self);

        let index = {
            let mut list = priv_.list.borrow_mut();

            let index = list.iter().position(|item| item == room);
            if let Some(index) = index {
                list.remove(index);
            }
            index
        };

        if let Some(index) = index {
            self.items_changed(index as u32, 1, 0);
        }
    }
}

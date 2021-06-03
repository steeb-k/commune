use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use crate::session::{
    room::{Room, RoomType},
    room_list::RoomList,
};

mod imp {
    use once_cell::unsync::OnceCell;
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct Category {
        pub model: OnceCell<gtk::FilterListModel>,
        pub type_: Cell<RoomType>,
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
                        RoomType::static_type(),
                        RoomType::default() as i32,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name of this category",
                        None,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "model",
                        "Model",
                        "The filter list model in that category",
                        gio::ListModel::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "type" => {
                    let type_ = value.get().unwrap();
                    self.type_.set(type_);
                }
                "model" => {
                    let model = value.get().unwrap();
                    obj.set_model(model);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "type" => obj.type_().to_value(),
                "display-name" => obj.type_().to_string().to_value(),
                "model" => self.model.get().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl ListModelImpl for Category {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            Room::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.model.get().map(|l| l.n_items()).unwrap_or(0)
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.model.get().and_then(|l| l.item(position))
        }
    }
}

glib::wrapper! {
    pub struct Category(ObjectSubclass<imp::Category>)
        @implements gio::ListModel;
}

impl Category {
    pub fn new(type_: RoomType, model: &RoomList) -> Self {
        glib::Object::new(&[("type", &type_), ("model", model)]).expect("Failed to create Category")
    }

    pub fn type_(&self) -> RoomType {
        let priv_ = imp::Category::from_instance(self);
        priv_.type_.get()
    }

    fn set_model(&self, model: gio::ListModel) {
        let priv_ = imp::Category::from_instance(self);
        let type_ = self.type_();

        let filter = gtk::CustomFilter::new(move |o| {
            o.downcast_ref::<Room>()
                .filter(|r| r.category() == type_)
                .is_some()
        });
        let filter_model = gtk::FilterListModel::new(Some(&model), Some(&filter));

        filter_model.connect_items_changed(
            clone!(@weak self as obj => move |_, pos, added, removed| {
                obj.items_changed(pos, added, removed);
            }),
        );

        let _ = priv_.model.set(filter_model);
    }
}

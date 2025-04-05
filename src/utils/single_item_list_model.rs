use gtk::{gio, glib, prelude::*, subclass::prelude::*};

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SingleItemListModel)]
    pub struct SingleItemListModel {
        /// The item contained by this model.
        #[property(get, construct_only)]
        inner_item: OnceCell<glib::Object>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SingleItemListModel {
        const NAME: &'static str = "SingleItemListModel";
        type Type = super::SingleItemListModel;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for SingleItemListModel {}

    impl ListModelImpl for SingleItemListModel {
        fn item_type(&self) -> glib::Type {
            self.inner_item().type_()
        }

        fn n_items(&self) -> u32 {
            1
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            (position == 0).then(|| self.inner_item().clone().upcast())
        }
    }

    impl SingleItemListModel {
        /// The item contained by this model.
        fn inner_item(&self) -> &glib::Object {
            self.inner_item.get().expect("inner item was initialized")
        }
    }
}

glib::wrapper! {
    /// A list model always containing a single item.
    pub struct SingleItemListModel(ObjectSubclass<imp::SingleItemListModel>)
        @implements gio::ListModel;
}

impl SingleItemListModel {
    /// Construct a new `SingleItemListModel` for the given item.
    pub fn new(item: &impl IsA<glib::Object>) -> Self {
        glib::Object::builder().property("inner-item", item).build()
    }
}

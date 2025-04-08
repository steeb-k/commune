use gtk::{gio, glib, prelude::*, subclass::prelude::*};

mod imp {
    use std::cell::{Cell, OnceCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SingleItemListModel)]
    pub struct SingleItemListModel {
        /// The item contained by this model.
        #[property(get, construct_only)]
        item: OnceCell<glib::Object>,
        /// Whether the item is hidden.
        #[property(get, set = Self::set_is_hidden, explicit_notify)]
        is_hidden: Cell<bool>,
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
            self.item().type_()
        }

        fn n_items(&self) -> u32 {
            1 - u32::from(self.is_hidden.get())
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            (!self.is_hidden.get() && position == 0).then(|| self.item().clone().upcast())
        }
    }

    impl SingleItemListModel {
        /// The item contained by this model.
        fn item(&self) -> &glib::Object {
            self.item.get().expect("item should be initialized")
        }

        /// Set whether the item is hidden.
        fn set_is_hidden(&self, hidden: bool) {
            if self.is_hidden.get() == hidden {
                return;
            }

            self.is_hidden.set(hidden);

            let obj = self.obj();
            obj.notify_is_hidden();

            let removed = (hidden).into();
            let added = (!hidden).into();
            obj.items_changed(0, removed, added);
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
        glib::Object::builder().property("item", item).build()
    }
}

use gtk::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::DummyObject)]
    pub struct DummyObject {
        /// The identifier of this item.
        #[property(get, set)]
        id: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DummyObject {
        const NAME: &'static str = "DummyObject";
        type Type = super::DummyObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for DummyObject {}
}

glib::wrapper! {
    /// A dummy GObject.
    ///
    /// It can be used for example to add extra widgets in a list model and can be identified with its ID.
    pub struct DummyObject(ObjectSubclass<imp::DummyObject>);
}

impl DummyObject {
    pub fn new(id: &str) -> Self {
        glib::Object::builder().property("id", id).build()
    }
}

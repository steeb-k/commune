use gtk::{glib, prelude::*, subclass::prelude::*};

use crate::session::content::ContentType;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default)]
    pub struct Entry {
        pub type_: Cell<ContentType>,
        pub display_name: RefCell<Option<String>>,
        pub icon_name: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Entry {
        const NAME: &'static str = "Entry";
        type Type = super::Entry;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for Entry {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_enum(
                        "type",
                        "Type",
                        "The type of this category",
                        ContentType::static_type(),
                        ContentType::default() as i32,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name of this Entry",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_string(
                        "icon-name",
                        "Icon Name",
                        "The icon name used for this Entry",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
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
                    self.type_.set(value.get().unwrap());
                }
                "display-name" => {
                    let _ = self.display_name.replace(value.get().unwrap());
                }
                "icon-name" => {
                    let _ = self.icon_name.replace(value.get().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "type" => obj.type_().to_value(),
                "display-name" => obj.type_().to_string().to_value(),
                "icon-name" => obj.icon_name().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct Entry(ObjectSubclass<imp::Entry>);
}

impl Entry {
    pub fn new(type_: ContentType) -> Self {
        glib::Object::new(&[("type", &type_)]).expect("Failed to create Entry")
    }

    pub fn type_(&self) -> ContentType {
        let priv_ = imp::Entry::from_instance(self);
        priv_.type_.get()
    }

    pub fn icon_name(&self) -> Option<&str> {
        match self.type_() {
            ContentType::Explore => Some("explore-symbolic"),
            _ => None,
        }
    }
}

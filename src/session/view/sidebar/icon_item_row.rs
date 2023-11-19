use adw::subclass::prelude::BinImpl;
use gtk::{self, glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::model::{IconItem, ItemType};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/sidebar/icon_item_row.ui")]
    pub struct IconItemRow {
        pub icon_item: RefCell<Option<IconItem>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IconItemRow {
        const NAME: &'static str = "SidebarIconItemRow";
        type Type = super::IconItemRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_css_name("icon-item");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for IconItemRow {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecObject::builder::<IconItem>("icon-item")
                    .explicit_notify()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "icon-item" => self.obj().set_icon_item(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "icon-item" => self.obj().icon_item().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for IconItemRow {}
    impl BinImpl for IconItemRow {}
}

glib::wrapper! {
    pub struct IconItemRow(ObjectSubclass<imp::IconItemRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl IconItemRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The [`IconItem`] of this row.
    pub fn icon_item(&self) -> Option<IconItem> {
        self.imp().icon_item.borrow().clone()
    }

    /// Set the [`IconItem`] of this row.
    pub fn set_icon_item(&self, icon_item: Option<IconItem>) {
        if self.icon_item() == icon_item {
            return;
        }

        if icon_item
            .as_ref()
            .is_some_and(|i| i.type_() == ItemType::Forget)
        {
            self.add_css_class("forget");
        } else {
            self.remove_css_class("forget");
        }

        self.imp().icon_item.replace(icon_item);
        self.notify("icon-item");
    }
}

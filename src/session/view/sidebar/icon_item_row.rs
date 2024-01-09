use adw::subclass::prelude::BinImpl;
use gtk::{self, glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::model::{SidebarIconItem, SidebarIconItemType};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/sidebar/icon_item_row.ui")]
    #[properties(wrapper_type = super::IconItemRow)]
    pub struct IconItemRow {
        /// The [`IconItem`] of this row.
        #[property(get, set = Self::set_icon_item, explicit_notify, nullable)]
        pub icon_item: RefCell<Option<SidebarIconItem>>,
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

    #[glib::derived_properties]
    impl ObjectImpl for IconItemRow {}

    impl WidgetImpl for IconItemRow {}
    impl BinImpl for IconItemRow {}

    impl IconItemRow {
        /// Set the [`IconItem`] of this row.
        fn set_icon_item(&self, icon_item: Option<SidebarIconItem>) {
            if *self.icon_item.borrow() == icon_item {
                return;
            }
            let obj = self.obj();

            if icon_item
                .as_ref()
                .is_some_and(|i| i.item_type() == SidebarIconItemType::Forget)
            {
                obj.add_css_class("forget");
            } else {
                obj.remove_css_class("forget");
            }

            self.icon_item.replace(icon_item);
            obj.notify_icon_item();
        }
    }
}

glib::wrapper! {
    pub struct IconItemRow(ObjectSubclass<imp::IconItemRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl IconItemRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

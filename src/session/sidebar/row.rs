use adw::{subclass::prelude::BinImpl, BinExt};
use gtk::{glib, prelude::*, subclass::prelude::*};

use crate::session::sidebar::RoomRow;
use crate::session::{categories::Category, room::Room};

mod imp {
    use super::*;
    use once_cell::sync::Lazy;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct Row {
        pub item: RefCell<Option<glib::Object>>,
        pub binding: RefCell<Option<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Row {
        const NAME: &'static str = "SidebarRow";
        type Type = super::Row;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for Row {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "item",
                    "Item",
                    "The sidebar item of this row",
                    glib::Object::static_type(),
                    glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                )]
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
                "item" => {
                    let item = value.get().unwrap();
                    obj.set_item(item);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "item" => obj.item().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for Row {}
    impl BinImpl for Row {}
}

glib::wrapper! {
    pub struct Row(ObjectSubclass<imp::Row>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Row {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create Row")
    }

    pub fn item(&self) -> Option<glib::Object> {
        let priv_ = imp::Row::from_instance(&self);
        priv_.item.borrow().clone()
    }

    pub fn set_item(&self, item: Option<glib::Object>) {
        let priv_ = imp::Row::from_instance(&self);

        if self.item() == item {
            return;
        }

        if let Some(binding) = priv_.binding.take() {
            binding.unbind();
        }

        if let Some(item) = item {
            if let Some(category) = item.downcast_ref::<Category>() {
                let child =
                    if let Some(Ok(child)) = self.child().map(|w| w.downcast::<gtk::Label>()) {
                        child
                    } else {
                        let child = gtk::Label::new(None);
                        self.set_child(Some(&child));
                        self.set_halign(gtk::Align::Start);
                        child.add_css_class("dim-label");
                        child
                    };

                let binding = category
                    .bind_property("display-name", &child, "label")
                    .flags(glib::BindingFlags::SYNC_CREATE)
                    .build()
                    .unwrap();

                priv_.binding.replace(Some(binding));
            } else if let Some(room) = item.downcast_ref::<Room>() {
                let child = if let Some(Ok(child)) = self.child().map(|w| w.downcast::<RoomRow>()) {
                    child
                } else {
                    let child = RoomRow::new();
                    self.set_child(Some(&child));
                    child
                };

                child.set_room(Some(room.clone()));
            } else {
                panic!("Wrong row item: {:?}", item);
            }
        }
        self.notify("item");
    }
}

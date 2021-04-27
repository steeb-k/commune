use adw::{prelude::*, subclass::prelude::*};
use chrono::{offset::Local, Datelike};
use gettextrs::gettext;
use gtk::{glib, prelude::*, subclass::prelude::*};

use crate::components::{ContextMenuBin, ContextMenuBinImpl};
use crate::session::content::{DividerRow, MessageRow, StateRow};
use crate::session::room::{Item, ItemType};
use matrix_sdk::events::AnyRoomEvent;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct ItemRow {
        pub item: RefCell<Option<Item>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ItemRow {
        const NAME: &'static str = "ContentItemRow";
        type Type = super::ItemRow;
        type ParentType = ContextMenuBin;
    }

    impl ObjectImpl for ItemRow {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "item",
                    "item",
                    "The item represented by this row",
                    Item::static_type(),
                    glib::ParamFlags::READWRITE,
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
                    let item = value.get::<Option<Item>>().unwrap();
                    obj.set_item(item);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "item" => self.item.borrow().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);
        }
    }

    impl WidgetImpl for ItemRow {}
    impl BinImpl for ItemRow {}
    impl ContextMenuBinImpl for ItemRow {}
}

glib::wrapper! {
    pub struct ItemRow(ObjectSubclass<imp::ItemRow>)
        @extends gtk::Widget, ContextMenuBin, adw::Bin, @implements gtk::Accessible;
}

// TODO:
// - [ ] Add context menu for operations
// - [ ] Don't show rows for items that don't have a visible UI
impl ItemRow {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create ItemRow")
    }

    /// This method sets this row to a new `Item`.
    ///
    /// It tries to reuse the widget and only update the content whenever possible, but it will
    /// create a new widget and drop the old one if it has to.
    fn set_item(&self, item: Option<Item>) {
        let priv_ = imp::ItemRow::from_instance(&self);

        if let Some(ref item) = item {
            match item.type_() {
                ItemType::Event(event) => match event.matrix_event() {
                    AnyRoomEvent::Message(_message) => {
                        let child = if let Some(Ok(child)) =
                            self.child().map(|w| w.downcast::<MessageRow>())
                        {
                            child
                        } else {
                            let child = MessageRow::new();
                            self.set_child(Some(&child));
                            child
                        };
                        child.set_event(event.clone());
                    }
                    AnyRoomEvent::State(state) => {
                        let child = if let Some(Ok(child)) =
                            self.child().map(|w| w.downcast::<StateRow>())
                        {
                            child
                        } else {
                            let child = StateRow::new();
                            self.set_child(Some(&child));
                            child
                        };

                        child.update(&state);
                    }
                    AnyRoomEvent::RedactedMessage(_) => {
                        let child = if let Some(Ok(child)) =
                            self.child().map(|w| w.downcast::<MessageRow>())
                        {
                            child
                        } else {
                            let child = MessageRow::new();
                            self.set_child(Some(&child));
                            child
                        };
                        child.set_event(event.clone());
                    }
                    AnyRoomEvent::RedactedState(_) => {
                        let child = if let Some(Ok(child)) =
                            self.child().map(|w| w.downcast::<MessageRow>())
                        {
                            child
                        } else {
                            let child = MessageRow::new();
                            self.set_child(Some(&child));
                            child
                        };
                        child.set_event(event.clone());
                    }
                },
                ItemType::DayDivider(date) => {
                    let fmt = if date.year() == Local::today().year() {
                        // Translators: This is a date format in the day divider without the year
                        gettext("%A, %B %e")
                    } else {
                        // Translators: This is a date format in the day divider with the year
                        gettext("%A, %B %e, %Y")
                    };
                    let date = date.format(&fmt).to_string();

                    if let Some(Ok(child)) = self.child().map(|w| w.downcast::<DividerRow>()) {
                        child.set_label(&date);
                    } else {
                        let child = DividerRow::new(date);
                        self.set_child(Some(&child));
                    };
                }
                ItemType::NewMessageDivider => {
                    let label = gettext("New Messages");

                    if let Some(Ok(child)) = self.child().map(|w| w.downcast::<DividerRow>()) {
                        child.set_label(&label);
                    } else {
                        let child = DividerRow::new(label);
                        self.set_child(Some(&child));
                    };
                }
            }
        }
        priv_.item.replace(item);
    }
}

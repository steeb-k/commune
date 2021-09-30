use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, subclass::prelude::*};

use crate::components::{ContextMenuBin, ContextMenuBinExt, ContextMenuBinImpl};
use crate::session::content::{DividerRow, MessageRow, StateRow};
use crate::session::event_source_dialog::EventSourceDialog;
use crate::session::room::{Event, Item, ItemType};
use matrix_sdk::ruma::events::AnySyncRoomEvent;

mod imp {
    use super::*;
    use glib::signal::SignalHandlerId;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct ItemRow {
        pub item: RefCell<Option<Item>>,
        pub menu_model: RefCell<Option<gio::MenuModel>>,
        pub event_notify_handler: RefCell<Option<SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ItemRow {
        const NAME: &'static str = "ContentItemRow";
        type Type = super::ItemRow;
        type ParentType = ContextMenuBin;

        fn class_init(klass: &mut Self::Class) {
            // View Event Source
            klass.install_action("item-row.view-source", None, move |widget, _, _| {
                let window = widget.root().unwrap().downcast().unwrap();
                let dialog =
                    EventSourceDialog::new(&window, widget.item().unwrap().event().unwrap());
                dialog.show();
            });
        }
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

        fn dispose(&self, _obj: &Self::Type) {
            if let Some(ItemType::Event(event)) =
                self.item.borrow().as_ref().map(|item| item.type_())
            {
                if let Some(handler) = self.event_notify_handler.borrow_mut().take() {
                    event.disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for ItemRow {}
    impl BinImpl for ItemRow {}
    impl ContextMenuBinImpl for ItemRow {}
}

glib::wrapper! {
    pub struct ItemRow(ObjectSubclass<imp::ItemRow>)
        @extends gtk::Widget, adw::Bin, ContextMenuBin, @implements gtk::Accessible;
}

// TODO:
// - [ ] Don't show rows for items that don't have a visible UI
impl ItemRow {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create ItemRow")
    }

    /// Get the row's `Item`.
    pub fn item(&self) -> Option<Item> {
        let priv_ = imp::ItemRow::from_instance(self);
        priv_.item.borrow().clone()
    }

    fn enable_gactions(&self) {
        self.action_set_enabled("item-row.view-source", true);
    }

    fn disable_gactions(&self) {
        self.action_set_enabled("item-row.view-source", false);
    }

    /// This method sets this row to a new `Item`.
    ///
    /// It tries to reuse the widget and only update the content whenever possible, but it will
    /// create a new widget and drop the old one if it has to.
    fn set_item(&self, item: Option<Item>) {
        let priv_ = imp::ItemRow::from_instance(self);

        if let Some(ItemType::Event(event)) = priv_.item.borrow().as_ref().map(|item| item.type_())
        {
            if let Some(handler) = priv_.event_notify_handler.borrow_mut().take() {
                event.disconnect(handler);
            }
        }

        if let Some(ref item) = item {
            match item.type_() {
                ItemType::Event(event) => {
                    if self.context_menu().is_none() {
                        let menu_model = gtk::Builder::from_resource(
                            "/org/gnome/FractalNext/content-item-row-menu.ui",
                        )
                        .object("menu_model");
                        self.set_context_menu(menu_model);

                        self.enable_gactions();
                    }

                    let event_notify_handler = event.connect_notify_local(
                        Some("event"),
                        clone!(@weak self as obj => move |event, _| {
                            obj.set_event_widget(event);
                        }),
                    );

                    priv_
                        .event_notify_handler
                        .borrow_mut()
                        .replace(event_notify_handler);

                    self.set_event_widget(event);
                }
                ItemType::DayDivider(date) => {
                    if self.context_menu().is_some() {
                        self.set_context_menu(None);
                        self.disable_gactions();
                    }

                    let fmt = if date.year() == glib::DateTime::new_now_local().unwrap().year() {
                        // Translators: This is a date format in the day divider without the year
                        gettext("%A, %B %e")
                    } else {
                        // Translators: This is a date format in the day divider with the year
                        gettext("%A, %B %e, %Y")
                    };
                    let date = date.format(&fmt).unwrap().to_string();

                    if let Some(Ok(child)) = self.child().map(|w| w.downcast::<DividerRow>()) {
                        child.set_label(&date);
                    } else {
                        let child = DividerRow::new(date);
                        self.set_child(Some(&child));
                    };
                }
                ItemType::NewMessageDivider => {
                    if self.context_menu().is_some() {
                        self.set_context_menu(None);
                        self.disable_gactions();
                    }

                    let label = gettext("New Messages");

                    if let Some(Ok(child)) = self.child().map(|w| w.downcast::<DividerRow>()) {
                        child.set_label(&label);
                    } else {
                        let child = DividerRow::new(label);
                        self.set_child(Some(&child));
                    };
                }
                ItemType::LoadingSpinner => {
                    if !self
                        .child()
                        .map_or(false, |widget| widget.is::<gtk::Spinner>())
                    {
                        let spinner = gtk::SpinnerBuilder::new()
                            .spinning(true)
                            .margin_top(12)
                            .margin_bottom(12)
                            .build();
                        self.set_child(Some(&spinner));
                    }
                }
            }
        }
        priv_.item.replace(item);
    }

    fn set_event_widget(&self, event: &Event) {
        match event.matrix_event() {
            Some(AnySyncRoomEvent::State(state)) => {
                let child = if let Some(Ok(child)) = self.child().map(|w| w.downcast::<StateRow>())
                {
                    child
                } else {
                    let child = StateRow::new();
                    self.set_child(Some(&child));
                    child
                };
                child.update(&state);
            }
            _ => {
                let child =
                    if let Some(Ok(child)) = self.child().map(|w| w.downcast::<MessageRow>()) {
                        child
                    } else {
                        let child = MessageRow::new();
                        self.set_child(Some(&child));
                        child
                    };
                child.set_event(event.clone());
            }
        }
    }
}

impl Default for ItemRow {
    fn default() -> Self {
        Self::new()
    }
}

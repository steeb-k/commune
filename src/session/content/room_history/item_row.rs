use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, subclass::prelude::*};
use matrix_sdk::ruma::events::AnySyncRoomEvent;

use crate::components::{ContextMenuBin, ContextMenuBinExt, ContextMenuBinImpl, ReactionChooser};
use crate::session::content::room_history::{message_row::MessageRow, DividerRow, StateRow};
use crate::session::room::{Event, EventActions, Item, ItemType, ReactionList};

mod imp {
    use super::*;
    use glib::signal::SignalHandlerId;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct ItemRow {
        pub item: RefCell<Option<Item>>,
        pub menu_model: RefCell<Option<gio::MenuModel>>,
        pub event_notify_handler: RefCell<Option<SignalHandlerId>>,
        pub reaction_chooser: RefCell<Option<ReactionChooser>>,
        pub emoji_chooser: RefCell<Option<gtk::EmojiChooser>>,
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
                vec![glib::ParamSpecObject::new(
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

        fn dispose(&self, obj: &Self::Type) {
            if let Some(ItemType::Event(event)) =
                self.item.borrow().as_ref().map(|item| item.type_())
            {
                if let Some(handler) = self.event_notify_handler.borrow_mut().take() {
                    event.disconnect(handler);
                }
            }

            obj.remove_reaction_chooser();
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
                    let action_group = self.set_event_actions(Some(event));

                    if event.message_content().is_some() {
                        self.set_context_menu(Some(Self::event_message_menu_model()));
                        self.set_reaction_chooser(event.reactions());

                        // Open emoji chooser
                        let more_reactions = gio::SimpleAction::new("more-reactions", None);
                        more_reactions.connect_activate(clone!(@weak self as obj => move |_, _| {
                            obj.show_emoji_chooser();
                        }));
                        action_group.unwrap().add_action(&more_reactions);
                    } else {
                        self.set_context_menu(Some(Self::event_state_menu_model()));
                        self.remove_reaction_chooser();
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
                        self.set_event_actions(None);
                        self.remove_reaction_chooser();
                    }

                    let fmt = if date.year() == glib::DateTime::now_local().unwrap().year() {
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
                        self.set_event_actions(None);
                        self.remove_reaction_chooser();
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
                        let spinner = gtk::Spinner::builder()
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

    /// Set the reaction chooser for the given `reactions`.
    ///
    /// If it doesn't exist, it is created
    fn set_reaction_chooser(&self, reactions: &ReactionList) {
        let priv_ = imp::ItemRow::from_instance(self);

        if priv_.reaction_chooser.borrow().is_none() {
            let reaction_chooser = ReactionChooser::new();
            self.popover()
                .add_child(&reaction_chooser, "reaction-chooser");
            priv_.reaction_chooser.replace(Some(reaction_chooser));
        }

        priv_
            .reaction_chooser
            .borrow()
            .as_ref()
            .unwrap()
            .set_reactions(Some(reactions.to_owned()));
    }

    /// Remove the reaction chooser and the emoji chooser, if they exist.
    fn remove_reaction_chooser(&self) {
        let priv_ = imp::ItemRow::from_instance(self);

        if let Some(reaction_chooser) = priv_.reaction_chooser.take() {
            reaction_chooser.unparent();
        }

        if let Some(emoji_chooser) = priv_.emoji_chooser.take() {
            emoji_chooser.unparent();
        }
    }

    fn show_emoji_chooser(&self) {
        let priv_ = imp::ItemRow::from_instance(self);

        if priv_.emoji_chooser.borrow().is_none() {
            let emoji_chooser = gtk::EmojiChooser::builder().has_arrow(false).build();
            emoji_chooser.connect_emoji_picked(|emoji_chooser, emoji| {
                emoji_chooser
                    .activate_action("event.toggle-reaction", Some(&emoji.to_variant()))
                    .unwrap();
            });
            emoji_chooser.set_parent(self);
            priv_.emoji_chooser.replace(Some(emoji_chooser));
        }

        let emoji_chooser = priv_.emoji_chooser.borrow().clone().unwrap();
        let (_, rectangle) = self.popover().pointing_to();
        emoji_chooser.set_pointing_to(Some(&rectangle));

        self.popover().popdown();
        emoji_chooser.popup();
    }
}

impl Default for ItemRow {
    fn default() -> Self {
        Self::new()
    }
}

impl EventActions for ItemRow {}

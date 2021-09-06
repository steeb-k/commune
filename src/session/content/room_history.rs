use adw::subclass::prelude::*;
use gtk::{
    gdk, glib, glib::clone, glib::signal::Inhibit, prelude::*, subclass::prelude::*,
    CompositeTemplate,
};
use sourceview::prelude::*;

use crate::components::{CustomEntry, RoomTitle};
use crate::session::content::{ItemRow, MarkdownPopover, RoomDetails};
use crate::session::room::{Room, RoomType};

mod imp {
    use super::*;
    use crate::Application;
    use glib::{signal::SignalHandlerId, subclass::InitializingObject};
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content-room-history.ui")]
    pub struct RoomHistory {
        pub compact: Cell<bool>,
        pub room: RefCell<Option<Room>>,
        pub category_handler: RefCell<Option<SignalHandlerId>>,
        pub empty_timeline_handler: RefCell<Option<SignalHandlerId>>,
        pub md_enabled: Cell<bool>,
        pub is_auto_scrolling: Cell<bool>,
        pub sticky: Cell<bool>,
        #[template_child]
        pub headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub room_title: TemplateChild<RoomTitle>,
        #[template_child]
        pub room_menu: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub listview: TemplateChild<gtk::ListView>,
        #[template_child]
        pub content: TemplateChild<gtk::Widget>,
        #[template_child]
        pub scrolled_window: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub scroll_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub scroll_btn_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub message_entry: TemplateChild<sourceview::View>,
        #[template_child]
        pub markdown_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub loading: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistory {
        const NAME: &'static str = "ContentRoomHistory";
        type Type = super::RoomHistory;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            CustomEntry::static_type();
            ItemRow::static_type();
            MarkdownPopover::static_type();
            Self::bind_template(klass);
            klass.set_accessible_role(gtk::AccessibleRole::Group);
            klass.install_action(
                "room-history.send-text-message",
                None,
                move |widget, _, _| {
                    widget.send_text_message();
                },
            );
            klass.install_action("room-history.leave", None, move |widget, _, _| {
                widget.leave();
            });

            klass.install_action("room-history.details", None, move |widget, _, _| {
                widget.open_room_details();
            });

            klass.install_action("room-history.scroll-down", None, move |widget, _, _| {
                widget.scroll_down();
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for RoomHistory {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_boolean(
                        "compact",
                        "Compact",
                        "Whether a compact view is used",
                        false,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_object(
                        "room",
                        "Room",
                        "The room currently shown",
                        Room::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_boolean(
                        "empty",
                        "Empty",
                        "Wheter there is currently a room shown",
                        false,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_boolean(
                        "markdown-enabled",
                        "Markdown enabled",
                        "Whether outgoing messages should be interpreted as markdown",
                        false,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_boolean(
                        "sticky",
                        "Sticky",
                        "Whether the room history should stick to the newest message in the timeline",
                        true,
                        glib::ParamFlags::READWRITE,
                    ),
                ]
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
                "compact" => {
                    let compact = value.get().unwrap();
                    self.compact.set(compact);
                }
                "room" => {
                    let room = value.get().unwrap();
                    obj.set_room(room);
                }
                "markdown-enabled" => {
                    let md_enabled = value.get().unwrap();
                    self.md_enabled.set(md_enabled);
                    self.markdown_button.set_icon_name(if md_enabled {
                        "format-indent-more-symbolic"
                    } else {
                        "format-justify-left-symbolic"
                    });
                }
                "sticky" => obj.set_sticky(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "compact" => self.compact.get().to_value(),
                "room" => obj.room().to_value(),
                "empty" => obj.room().is_none().to_value(),
                "markdown-enabled" => self.md_enabled.get().to_value(),
                "sticky" => obj.sticky().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            obj.set_sticky(true);
            let adj = self.listview.vadjustment().unwrap();

            adj.connect_value_changed(clone!(@weak obj => move |adj| {
                let priv_ = imp::RoomHistory::from_instance(&obj);

                if priv_.is_auto_scrolling.get() {
                    if adj.value() + adj.page_size() == adj.upper() {
                        priv_.is_auto_scrolling.set(false);
                        obj.set_sticky(true);
                    }
                } else {
                    obj.set_sticky(adj.value() + adj.page_size() == adj.upper());
                    obj.load_more_messages(adj);
                }
            }));
            adj.connect_upper_notify(clone!(@weak obj => move |_| {
                if obj.sticky() {
                    obj.scroll_down();
                }
            }));

            let key_events = gtk::EventControllerKey::new();
            self.message_entry.add_controller(&key_events);

            key_events
                .connect_key_pressed(clone!(@weak obj => @default-return Inhibit(false), move |_, key, _, modifier| {
                if !modifier.contains(gdk::ModifierType::SHIFT_MASK) && (key == gdk::keys::constants::Return || key == gdk::keys::constants::KP_Enter) {
                    obj.activate_action("room-history.send-text-message", None);
                    Inhibit(true)
                } else {
                    Inhibit(false)
                }
            }));

            let buffer = self
                .message_entry
                .buffer()
                .downcast::<sourceview::Buffer>()
                .unwrap();

            buffer.connect_text_notify(clone!(@weak obj => move |buffer| {
               let (start_iter, end_iter) = buffer.bounds();
               obj.action_set_enabled("room-history.send-text-message", start_iter != end_iter);
            }));

            let (start_iter, end_iter) = buffer.bounds();
            obj.action_set_enabled("room-history.send-text-message", start_iter != end_iter);

            let md_lang =
                sourceview::LanguageManager::default().and_then(|lm| lm.language("markdown"));
            buffer.set_language(md_lang.as_ref());
            obj.bind_property("markdown-enabled", &buffer, "highlight-syntax")
                .flags(glib::BindingFlags::SYNC_CREATE)
                .build();

            let settings = Application::default().settings();
            settings
                .bind("markdown-enabled", obj, "markdown-enabled")
                .build();

            self.parent_constructed(obj);
        }
    }

    impl WidgetImpl for RoomHistory {}
    impl BinImpl for RoomHistory {}
}

glib::wrapper! {
    pub struct RoomHistory(ObjectSubclass<imp::RoomHistory>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl RoomHistory {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create RoomHistory")
    }

    pub fn set_room(&self, room: Option<Room>) {
        let priv_ = imp::RoomHistory::from_instance(self);

        if self.room() == room {
            return;
        }

        if let Some(category_handler) = priv_.category_handler.take() {
            if let Some(room) = self.room() {
                room.disconnect(category_handler);
            }
        }

        if let Some(empty_timeline_handler) = priv_.empty_timeline_handler.take() {
            if let Some(room) = self.room() {
                room.timeline().disconnect(empty_timeline_handler);
            }
        }

        if let Some(ref room) = room {
            let handler_id = room.connect_notify_local(
                Some("category"),
                clone!(@weak self as obj => move |_, _| {
                        obj.update_room_state();
                }),
            );

            priv_.category_handler.replace(Some(handler_id));

            let handler_id = room.timeline().connect_notify_local(
                Some("empty"),
                clone!(@weak self as obj => move |_, _| {
                        obj.set_empty_timeline();
                }),
            );

            priv_.empty_timeline_handler.replace(Some(handler_id));
            room.load_members();
        }

        // TODO: use gtk::MultiSelection to allow selection
        let model = room
            .as_ref()
            .map(|room| gtk::NoSelection::new(Some(room.timeline())));

        priv_.listview.set_model(model.as_ref());
        priv_.room.replace(room);
        let adj = priv_.listview.vadjustment().unwrap();
        self.load_more_messages(&adj);
        self.update_room_state();
        self.set_empty_timeline();
        self.notify("room");
        self.notify("empty");
    }

    pub fn room(&self) -> Option<Room> {
        let priv_ = imp::RoomHistory::from_instance(self);
        priv_.room.borrow().clone()
    }

    pub fn send_text_message(&self) {
        let priv_ = imp::RoomHistory::from_instance(self);
        let buffer = priv_.message_entry.buffer();
        let (start_iter, end_iter) = buffer.bounds();
        let body = buffer.text(&start_iter, &end_iter, true);

        if let Some(room) = &*priv_.room.borrow() {
            room.send_text_message(body.as_str(), priv_.md_enabled.get());
        }

        buffer.set_text("");
    }

    pub fn leave(&self) {
        let priv_ = imp::RoomHistory::from_instance(self);

        if let Some(room) = &*priv_.room.borrow() {
            room.set_category(RoomType::Left);
        }
    }

    pub fn open_room_details(&self) {
        if let Some(room) = self.room() {
            let window = RoomDetails::new(&self.parent_window(), &room);
            window.show();
        }
    }

    fn update_room_state(&self) {
        let priv_ = imp::RoomHistory::from_instance(self);

        if let Some(room) = &*priv_.room.borrow() {
            if room.category() == RoomType::Left {
                self.action_set_enabled("room-history.leave", false);
                priv_.room_menu.hide();
            } else {
                self.action_set_enabled("room-history.leave", true);
                priv_.room_menu.show();
            }
        }
    }

    fn set_empty_timeline(&self) {
        let priv_ = imp::RoomHistory::from_instance(self);

        if let Some(room) = &*priv_.room.borrow() {
            if room.timeline().empty() {
                priv_.stack.set_visible_child(&*priv_.loading);
            } else {
                priv_.stack.set_visible_child(&*priv_.content);
            }
        }
    }

    fn load_more_messages(&self, adj: &gtk::Adjustment) {
        // Load more messages when the user gets close to the end of the known room history
        // Use the page size twice to detect if the user gets close to the end
        if adj.value() < adj.page_size() * 2.0 || adj.upper() <= adj.page_size() * 2.0 {
            if let Some(room) = self.room() {
                room.load_previous_events();
            }
        }
    }

    /// Returns the parent GtkWindow containing this widget.
    fn parent_window(&self) -> Option<gtk::Window> {
        self.root()?.downcast().ok()
    }

    pub fn sticky(&self) -> bool {
        let priv_ = imp::RoomHistory::from_instance(self);

        priv_.sticky.get()
    }

    pub fn set_sticky(&self, sticky: bool) {
        let priv_ = imp::RoomHistory::from_instance(self);

        if self.sticky() == sticky {
            return;
        }

        priv_.scroll_btn_revealer.set_reveal_child(!sticky);

        priv_.sticky.set(sticky);
        self.notify("sticky");
    }

    /// Scroll to the newest message in the timeline
    pub fn scroll_down(&self) {
        let priv_ = imp::RoomHistory::from_instance(self);

        priv_.is_auto_scrolling.set(true);

        priv_
            .scrolled_window
            .emit_by_name("scroll-child", &[&gtk::ScrollType::End, &false])
            .unwrap();
    }
}

impl Default for RoomHistory {
    fn default() -> Self {
        Self::new()
    }
}

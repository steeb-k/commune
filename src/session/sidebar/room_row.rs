use crate::components::Avatar;
use adw::subclass::prelude::BinImpl;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::room::{HighlightFlags, Room};

mod imp {
    use super::*;
    use glib::{subclass::InitializingObject, SignalHandlerId};
    use once_cell::sync::Lazy;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar-room-row.ui")]
    pub struct RoomRow {
        pub room: RefCell<Option<Room>>,
        pub bindings: RefCell<Vec<glib::Binding>>,
        pub signal_handler: RefCell<Option<SignalHandlerId>>,
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub notification_count: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomRow {
        const NAME: &'static str = "SidebarRoomRow";
        type Type = super::RoomRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Avatar::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for RoomRow {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "room",
                    "Room",
                    "The room of this row",
                    Room::static_type(),
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
                "room" => {
                    let room = value.get().unwrap();
                    obj.set_room(room);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "room" => obj.room().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for RoomRow {}
    impl BinImpl for RoomRow {}
}

glib::wrapper! {
    pub struct RoomRow(ObjectSubclass<imp::RoomRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl RoomRow {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create RoomRow")
    }

    pub fn room(&self) -> Option<Room> {
        let priv_ = imp::RoomRow::from_instance(&self);
        priv_.room.borrow().clone()
    }

    pub fn set_room(&self, room: Option<Room>) {
        let priv_ = imp::RoomRow::from_instance(&self);

        if self.room() == room {
            return;
        }

        if let Some(room) = priv_.room.take() {
            if let Some(id) = priv_.signal_handler.take() {
                room.disconnect(id);
            }
        }

        let mut bindings = priv_.bindings.borrow_mut();
        while let Some(binding) = bindings.pop() {
            binding.unbind();
        }

        if let Some(ref room) = room {
            let display_name_binding = room
                .bind_property("display-name", &priv_.display_name.get(), "label")
                .flags(glib::BindingFlags::SYNC_CREATE)
                .build()
                .unwrap();

            let notification_count_binding = room
                .bind_property(
                    "notification-count",
                    &priv_.notification_count.get(),
                    "label",
                )
                .flags(glib::BindingFlags::SYNC_CREATE)
                .build()
                .unwrap();
            let notification_count_vislbe_binding = room
                .bind_property(
                    "notification-count",
                    &priv_.notification_count.get(),
                    "visible",
                )
                .flags(glib::BindingFlags::SYNC_CREATE)
                .transform_from(|_, value| Some((value.get::<u64>().unwrap() > 0).to_value()))
                .build()
                .unwrap();

            priv_.signal_handler.replace(Some(room.connect_notify_local(
                Some("highlight"),
                clone!(@weak self as obj => move |_, _| {
                        obj.set_highlight();
                }),
            )));

            self.set_highlight();

            bindings.append(&mut vec![
                display_name_binding,
                notification_count_binding,
                notification_count_vislbe_binding,
            ]);
        }
        priv_
            .avatar
            .set_item(room.clone().map(|room| room.avatar().clone()));
        priv_.room.replace(room);
        self.notify("room");
    }

    fn set_highlight(&self) {
        let priv_ = imp::RoomRow::from_instance(&self);
        if let Some(room) = &*priv_.room.borrow() {
            match room.highlight() {
                HighlightFlags::NONE => {
                    priv_.notification_count.remove_css_class("highlight");
                    priv_.display_name.remove_css_class("bold");
                }
                HighlightFlags::HIGHLIGHT => {
                    priv_.notification_count.add_css_class("highlight");
                    priv_.display_name.remove_css_class("bold");
                }
                HighlightFlags::BOLD => {
                    priv_.display_name.add_css_class("bold");
                    priv_.notification_count.remove_css_class("highlight");
                }
                HighlightFlags::HIGHLIGHT_BOLD => {
                    priv_.notification_count.add_css_class("highlight");
                    priv_.display_name.add_css_class("bold");
                }
                _ => {}
            };
        }
    }
}

impl Default for RoomRow {
    fn default() -> Self {
        Self::new()
    }
}

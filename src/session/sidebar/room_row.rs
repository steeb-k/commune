use adw::subclass::prelude::BinImpl;
use gtk::{gdk, glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::room::{HighlightFlags, Room, RoomType};

mod imp {
    use super::*;
    use glib::{subclass::InitializingObject, SignalHandlerId};
    use once_cell::sync::Lazy;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar-room-row.ui")]
    pub struct RoomRow {
        pub room: RefCell<Option<Room>>,
        pub binding: RefCell<Option<glib::Binding>>,
        pub signal_handler: RefCell<Option<SignalHandlerId>>,
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

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            // Allow to drag rooms
            let drag = gtk::DragSource::builder()
                .actions(gdk::DragAction::MOVE)
                .build();
            drag.connect_prepare(
                clone!(@weak obj => @default-return None, move |drag, x, y| {
                    obj.drag_prepare(drag, x, y)
                }),
            );
            drag.connect_drag_begin(clone!(@weak obj => move |_, _| {
                obj.drag_begin();
            }));
            drag.connect_drag_end(clone!(@weak obj => move |_, _, _| {
                obj.drag_end();
            }));
            obj.add_controller(&drag);
        }

        fn dispose(&self, _obj: &Self::Type) {
            if let Some(room) = self.room.take() {
                if let Some(id) = self.signal_handler.take() {
                    room.disconnect(id);
                }
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
        let priv_ = imp::RoomRow::from_instance(self);
        priv_.room.borrow().clone()
    }

    pub fn set_room(&self, room: Option<Room>) {
        let priv_ = imp::RoomRow::from_instance(self);

        if self.room() == room {
            return;
        }

        if let Some(room) = priv_.room.take() {
            if let Some(id) = priv_.signal_handler.take() {
                room.disconnect(id);
            }
            if let Some(binding) = priv_.binding.take() {
                binding.unbind();
            }
            priv_.display_name.remove_css_class("dim-label");
        }

        if let Some(ref room) = room {
            priv_.binding.replace(Some(
                room.bind_property(
                    "notification-count",
                    &priv_.notification_count.get(),
                    "visible",
                )
                .flags(glib::BindingFlags::SYNC_CREATE)
                .transform_from(|_, value| Some((value.get::<u64>().unwrap() > 0).to_value()))
                .build()
                .unwrap(),
            ));

            priv_.signal_handler.replace(Some(room.connect_notify_local(
                Some("highlight"),
                clone!(@weak self as obj => move |_, _| {
                        obj.set_highlight();
                }),
            )));

            if room.category() == RoomType::Left {
                priv_.display_name.add_css_class("dim-label");
            }

            self.set_highlight();
        }
        priv_.room.replace(room);
        self.notify("room");
    }

    fn set_highlight(&self) {
        let priv_ = imp::RoomRow::from_instance(self);
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

    fn drag_prepare(&self, drag: &gtk::DragSource, x: f64, y: f64) -> Option<gdk::ContentProvider> {
        let paintable = gtk::WidgetPaintable::new(Some(&self.parent().unwrap()));
        // FIXME: The hotspot coordinates don't work.
        // See https://gitlab.gnome.org/GNOME/gtk/-/issues/2341
        drag.set_icon(Some(&paintable), x as i32, y as i32);
        self.room()
            .map(|room| gdk::ContentProvider::for_value(&room.to_value()))
    }

    fn drag_begin(&self) {
        self.parent().unwrap().add_css_class("drag");
        let category = Some(u32::from(self.room().unwrap().category()));
        self.activate_action("sidebar.set-drop-source-type", Some(&category.to_variant()));
    }

    fn drag_end(&self) {
        self.activate_action("sidebar.set-drop-source-type", None);
        self.parent().unwrap().remove_css_class("drag");
    }
}

impl Default for RoomRow {
    fn default() -> Self {
        Self::new()
    }
}

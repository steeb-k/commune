use adw::{prelude::*, subclass::prelude::*};
use gtk::{gdk, glib, glib::clone, CompositeTemplate};

use super::Row;
use crate::{
    i18n::{gettext_f, ngettext_f},
    prelude::*,
    session::model::{HighlightFlags, Room, RoomCategory},
    utils::BoundObject,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/sidebar/room_row.ui")]
    #[properties(wrapper_type = super::RoomRow)]
    pub struct RoomRow {
        /// The room represented by this row.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        pub room: BoundObject<Room>,
        #[template_child]
        pub display_name_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub notification_count: TemplateChild<gtk::Label>,
        pub direct_icon: RefCell<Option<gtk::Image>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomRow {
        const NAME: &'static str = "SidebarRoomRow";
        type Type = super::RoomRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_css_name("room");

            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomRow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // Allow to drag rooms
            let drag = gtk::DragSource::builder()
                .actions(gdk::DragAction::MOVE)
                .build();
            drag.connect_prepare(clone!(
                #[weak]
                obj,
                #[upgrade_or]
                None,
                move |drag, x, y| obj.drag_prepare(drag, x, y)
            ));
            drag.connect_drag_begin(clone!(
                #[weak]
                obj,
                move |_, _| {
                    obj.drag_begin();
                }
            ));
            drag.connect_drag_end(clone!(
                #[weak]
                obj,
                move |_, _, _| {
                    obj.drag_end();
                }
            ));
            obj.add_controller(drag);
        }
    }

    impl WidgetImpl for RoomRow {}
    impl BinImpl for RoomRow {}

    impl RoomRow {
        /// Set the room represented by this row.
        pub fn set_room(&self, room: Option<Room>) {
            if self.room.obj() == room {
                return;
            }
            let obj = self.obj();

            self.room.disconnect_signals();
            self.display_name.remove_css_class("dim-label");

            if let Some(room) = room {
                let highlight_handler = room.connect_highlight_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_highlight();
                    }
                ));
                let direct_handler = room.connect_is_direct_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_direct_icon();
                    }
                ));
                let name_handler = room.connect_display_name_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_accessibility_label();
                    }
                ));
                let notifications_count_handler = room.connect_notification_count_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_accessibility_label();
                    }
                ));

                if room.category() == RoomCategory::Left {
                    self.display_name.add_css_class("dim-label");
                }

                self.room.set(
                    room,
                    vec![
                        highlight_handler,
                        direct_handler,
                        name_handler,
                        notifications_count_handler,
                    ],
                );

                obj.update_accessibility_label();
            }

            obj.update_highlight();
            obj.update_direct_icon();
            obj.notify_room();
        }
    }
}

glib::wrapper! {
    /// A sidebar row representing a room.
    pub struct RoomRow(ObjectSubclass<imp::RoomRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl RoomRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Update how this row is highlighted according to the current state.
    fn update_highlight(&self) {
        let imp = self.imp();
        if let Some(room) = self.room() {
            let flags = room.highlight();

            if flags.contains(HighlightFlags::HIGHLIGHT) {
                imp.notification_count.add_css_class("highlight");
            } else {
                imp.notification_count.remove_css_class("highlight");
            }

            if flags.contains(HighlightFlags::BOLD) {
                imp.display_name.add_css_class("bold");
            } else {
                imp.display_name.remove_css_class("bold");
            }
        } else {
            imp.notification_count.remove_css_class("highlight");
            imp.display_name.remove_css_class("bold");
        }
    }

    fn drag_prepare(&self, drag: &gtk::DragSource, x: f64, y: f64) -> Option<gdk::ContentProvider> {
        let room = self.room()?;

        if let Some(parent) = self.parent() {
            let paintable = gtk::WidgetPaintable::new(Some(&parent));
            // FIXME: The hotspot coordinates don't work.
            // See https://gitlab.gnome.org/GNOME/gtk/-/issues/2341
            drag.set_icon(Some(&paintable), x as i32, y as i32);
        }

        Some(gdk::ContentProvider::for_value(&room.to_value()))
    }

    fn drag_begin(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let Some(row) = self.parent().and_downcast::<Row>() else {
            return;
        };
        let Some(sidebar) = row.sidebar() else {
            return;
        };
        row.add_css_class("drag");

        sidebar.set_drop_source_category(Some(room.category()));
    }

    fn drag_end(&self) {
        let Some(row) = self.parent().and_downcast::<Row>() else {
            return;
        };
        let Some(sidebar) = row.sidebar() else {
            return;
        };
        sidebar.set_drop_source_category(None);
        row.remove_css_class("drag");
    }

    fn update_direct_icon(&self) {
        let imp = self.imp();
        let is_direct = self.room().is_some_and(|room| room.is_direct());

        if is_direct {
            if imp.direct_icon.borrow().is_none() {
                let icon = gtk::Image::builder()
                    .icon_name("avatar-default-symbolic")
                    .icon_size(gtk::IconSize::Normal)
                    .css_classes(["dim-label"])
                    .build();

                imp.display_name_box.prepend(&icon);
                imp.direct_icon.replace(Some(icon));
            }
        } else if let Some(icon) = imp.direct_icon.take() {
            imp.display_name_box.remove(&icon);
        }
    }

    fn update_accessibility_label(&self) {
        let Some(parent) = self.parent() else {
            return;
        };
        parent.update_property(&[gtk::accessible::Property::Label(&self.accessible_label())]);
    }

    fn accessible_label(&self) -> String {
        let Some(room) = self.room() else {
            return String::new();
        };

        let name = if room.is_direct() {
            gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name. Presented to screen readers when a
                // room is a direct chat with another user.
                "Direct chat with {name}",
                &[("name", &room.display_name())],
            )
        } else {
            room.display_name()
        };

        if room.notification_count() > 0 {
            let count = ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name. Presented to screen readers when a room has notifications
                // for unread messages.
                "1 notification",
                "{count} notifications",
                room.notification_count() as u32,
                &[("count", &room.notification_count().to_string())],
            );
            format!("{name} {count}")
        } else {
            name
        }
    }
}

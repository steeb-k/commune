use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::glib;

use crate::session::PeekedMessage;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/dialogs/peek_row.ui")]
    #[properties(wrapper_type = super::RoomPeekRow)]
    pub struct RoomPeekRow {
        #[template_child]
        sender_label: TemplateChild<gtk::Label>,
        #[template_child]
        timestamp_label: TemplateChild<gtk::Label>,
        #[template_child]
        body_label: TemplateChild<gtk::Label>,
        /// The message presented by this row.
        #[property(get, set = Self::set_message, explicit_notify, nullable)]
        message: RefCell<Option<PeekedMessage>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomPeekRow {
        const NAME: &'static str = "RoomPeekRow";
        type Type = super::RoomPeekRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_accessible_role(gtk::AccessibleRole::ListItem);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomPeekRow {}

    impl WidgetImpl for RoomPeekRow {}
    impl BinImpl for RoomPeekRow {}

    impl RoomPeekRow {
        /// Set the message presented by this row.
        fn set_message(&self, message: Option<PeekedMessage>) {
            if *self.message.borrow() == message {
                return;
            }

            if let Some(message) = &message {
                self.sender_label.set_label(&message.sender_name());
                self.body_label.set_label(&message.body());

                // Translators: This is the date and time at which a message in
                // a room preview was sent. See `man strftime` or the
                // documentation of g_date_time_format for the available
                // specifiers:
                // <https://docs.gtk.org/glib/method.DateTime.format.html>
                let format = gettext("%x %R");
                let timestamp = message.timestamp().format(&format).unwrap_or_default();
                self.timestamp_label.set_label(&timestamp);
            } else {
                self.sender_label.set_label("");
                self.body_label.set_label("");
                self.timestamp_label.set_label("");
            }

            self.message.replace(message);
            self.obj().notify_message();
        }
    }
}

glib::wrapper! {
    /// A row presenting one message of a room that was not joined.
    ///
    /// There is no avatar here, and the body is plain text. Both are
    /// deliberate: nothing on this row fetches media from a room the person
    /// looking at it has not joined.
    pub struct RoomPeekRow(ObjectSubclass<imp::RoomPeekRow>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RoomPeekRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for RoomPeekRow {
    fn default() -> Self {
        Self::new()
    }
}

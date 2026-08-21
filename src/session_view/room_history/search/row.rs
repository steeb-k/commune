use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone};

use crate::{
    components::{Avatar, AvatarData},
    prelude::*,
    session::RoomSearchResult,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_history/search/row.ui")]
    #[properties(wrapper_type = super::RoomHistorySearchRow)]
    pub struct RoomHistorySearchRow {
        #[template_child]
        avatar: TemplateChild<Avatar>,
        #[template_child]
        sender_label: TemplateChild<gtk::Label>,
        #[template_child]
        timestamp_label: TemplateChild<gtk::Label>,
        #[template_child]
        body_label: TemplateChild<gtk::Label>,
        /// The result presented by this row.
        #[property(get, set = Self::set_result, explicit_notify, nullable)]
        result: RefCell<Option<RoomSearchResult>>,
        sender_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistorySearchRow {
        const NAME: &'static str = "RoomHistorySearchRow";
        type Type = super::RoomHistorySearchRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Avatar::ensure_type();

            Self::bind_template(klass);
            klass.set_accessible_role(gtk::AccessibleRole::ListItem);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomHistorySearchRow {
        fn dispose(&self) {
            self.disconnect_sender();
        }
    }

    impl WidgetImpl for RoomHistorySearchRow {}
    impl BinImpl for RoomHistorySearchRow {}

    impl RoomHistorySearchRow {
        /// Disconnect the signals of the current sender.
        fn disconnect_sender(&self) {
            if let Some(handler) = self.sender_handler.take()
                && let Some(sender) = self
                    .result
                    .borrow()
                    .as_ref()
                    .and_then(RoomSearchResult::sender)
            {
                sender.disconnect(handler);
            }
        }

        /// Set the result presented by this row.
        fn set_result(&self, result: Option<RoomSearchResult>) {
            if *self.result.borrow() == result {
                return;
            }

            self.disconnect_sender();
            self.result.replace(result.clone());

            if let Some(result) = result {
                if let Some(sender) = result.sender() {
                    self.avatar.set_data(Some(sender.avatar_data().clone()));

                    let handler = sender.connect_display_name_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_sender_label();
                        }
                    ));
                    self.sender_handler.replace(Some(handler));
                } else {
                    self.avatar.set_data(None::<AvatarData>);
                }

                self.update_sender_label();
                self.body_label.set_label(&result.body());

                // Translators: This is the date and time at which a message matching a
                // search was sent. See `man strftime` or the documentation of
                // g_date_time_format for the available specifiers:
                // <https://docs.gtk.org/glib/method.DateTime.format.html>
                let format = gettext("%x %R");
                let timestamp = result.timestamp().format(&format).unwrap_or_default();
                self.timestamp_label.set_label(&timestamp);
            } else {
                self.avatar.set_data(None::<AvatarData>);
                self.sender_label.set_label("");
                self.body_label.set_label("");
                self.timestamp_label.set_label("");
            }

            self.obj().notify_result();
        }

        /// Update the label presenting the sender of the message.
        fn update_sender_label(&self) {
            let name = self
                .result
                .borrow()
                .as_ref()
                .and_then(RoomSearchResult::sender)
                .map(|sender| sender.disambiguated_name())
                .unwrap_or_default();

            self.sender_label.set_label(&name);
        }
    }
}

glib::wrapper! {
    /// A row presenting a message matching a search in a room.
    pub struct RoomHistorySearchRow(ObjectSubclass<imp::RoomHistorySearchRow>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RoomHistorySearchRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for RoomHistorySearchRow {
    fn default() -> Self {
        Self::new()
    }
}

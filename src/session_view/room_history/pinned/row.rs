use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone};
use tracing::error;

use crate::{
    components::{Avatar, AvatarData},
    prelude::*,
    session::Event,
    spawn, toast,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_history/pinned/row.ui")]
    #[properties(wrapper_type = super::RoomHistoryPinnedRow)]
    pub struct RoomHistoryPinnedRow {
        #[template_child]
        avatar: TemplateChild<Avatar>,
        #[template_child]
        sender_label: TemplateChild<gtk::Label>,
        #[template_child]
        timestamp_label: TemplateChild<gtk::Label>,
        #[template_child]
        body_label: TemplateChild<gtk::Label>,
        #[template_child]
        unpin_button: TemplateChild<gtk::Button>,
        /// The pinned event presented by this row.
        #[property(get, set = Self::set_event, explicit_notify, nullable)]
        event: RefCell<Option<Event>>,
        sender_handler: RefCell<Option<glib::SignalHandlerId>>,
        permissions_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistoryPinnedRow {
        const NAME: &'static str = "RoomHistoryPinnedRow";
        type Type = super::RoomHistoryPinnedRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Avatar::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_accessible_role(gtk::AccessibleRole::ListItem);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomHistoryPinnedRow {
        fn dispose(&self) {
            self.disconnect_signals();
        }
    }

    impl WidgetImpl for RoomHistoryPinnedRow {}
    impl BinImpl for RoomHistoryPinnedRow {}

    #[gtk::template_callbacks]
    impl RoomHistoryPinnedRow {
        /// Disconnect the signals of the current event.
        fn disconnect_signals(&self) {
            let event = self.event.borrow().clone();
            let Some(event) = event else {
                return;
            };

            if let Some(handler) = self.sender_handler.take() {
                event.sender().disconnect(handler);
            }

            if let Some(handler) = self.permissions_handler.take() {
                event.room().permissions().disconnect(handler);
            }
        }

        /// Set the pinned event presented by this row.
        fn set_event(&self, event: Option<Event>) {
            if *self.event.borrow() == event {
                return;
            }

            self.disconnect_signals();
            self.event.replace(event.clone());

            if let Some(event) = event {
                let sender = event.sender();
                self.avatar.set_data(Some(sender.avatar_data().clone()));

                let sender_handler = sender.connect_display_name_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_sender_label();
                    }
                ));
                self.sender_handler.replace(Some(sender_handler));

                let permissions_handler = event.room().permissions().connect_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_unpin_button();
                    }
                ));
                self.permissions_handler.replace(Some(permissions_handler));

                self.update_sender_label();
                self.body_label.set_label(&event_body(&event));

                // Translators: This is the date and time at which a pinned message
                // was sent. See `man strftime` or the documentation of
                // g_date_time_format for the available specifiers:
                // <https://docs.gtk.org/glib/method.DateTime.format.html>
                let format = gettext("%x %R");
                let timestamp = event.timestamp().format(&format).unwrap_or_default();
                self.timestamp_label.set_label(&timestamp);
            } else {
                self.avatar.set_data(None::<AvatarData>);
                self.sender_label.set_label("");
                self.body_label.set_label("");
                self.timestamp_label.set_label("");
            }

            self.update_unpin_button();
            self.obj().notify_event();
        }

        /// Update the label presenting the sender of the message.
        fn update_sender_label(&self) {
            let name = self
                .event
                .borrow()
                .as_ref()
                .map(|event| event.sender().disambiguated_name())
                .unwrap_or_default();

            self.sender_label.set_label(&name);
        }

        /// Update whether the button to unpin the message is shown.
        fn update_unpin_button(&self) {
            let can_pin_events = self
                .event
                .borrow()
                .as_ref()
                .is_some_and(|event| event.room().permissions().can_pin_events());

            self.unpin_button.set_visible(can_pin_events);
        }

        /// Unpin the message presented by this row.
        #[template_callback]
        fn unpin(&self) {
            let event = self.event.borrow().clone();
            let Some(event) = event else {
                return;
            };
            let Some(event_id) = event.event_id() else {
                error!("Pinned event does not have an event ID");
                return;
            };

            let obj = self.obj().clone();
            spawn!(async move {
                if event.room().unpin_event(event_id).await.is_err() {
                    toast!(obj, gettext("Could not unpin message"));
                }
            });
        }
    }
}

glib::wrapper! {
    /// A row presenting a message pinned in a room.
    pub struct RoomHistoryPinnedRow(ObjectSubclass<imp::RoomHistoryPinnedRow>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RoomHistoryPinnedRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for RoomHistoryPinnedRow {
    fn default() -> Self {
        Self::new()
    }
}

/// The text to present for the given pinned event.
fn event_body(event: &Event) -> String {
    if let Some(message) = event.message() {
        return message.msgtype().body().to_owned();
    }

    // A room can pin anything, including a state event. Rather than teach this
    // row to render every kind, say plainly that it is not a message.
    gettext("This pinned event is not a message.")
}

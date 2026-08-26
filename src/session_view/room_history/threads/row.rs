use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone};

use crate::{
    components::{Avatar, AvatarData},
    gettext_f, ngettext_f,
    prelude::*,
    session::ThreadListEntry,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_history/threads/row.ui")]
    #[properties(wrapper_type = super::RoomHistoryThreadsRow)]
    pub struct RoomHistoryThreadsRow {
        #[template_child]
        avatar: TemplateChild<Avatar>,
        #[template_child]
        sender_label: TemplateChild<gtk::Label>,
        #[template_child]
        timestamp_label: TemplateChild<gtk::Label>,
        #[template_child]
        body_label: TemplateChild<gtk::Label>,
        #[template_child]
        replies_label: TemplateChild<gtk::Label>,
        #[template_child]
        latest_label: TemplateChild<gtk::Label>,
        /// The thread presented by this row.
        #[property(get, set = Self::set_entry, explicit_notify, nullable)]
        entry: RefCell<Option<ThreadListEntry>>,
        sender_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistoryThreadsRow {
        const NAME: &'static str = "RoomHistoryThreadsRow";
        type Type = super::RoomHistoryThreadsRow;
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
    impl ObjectImpl for RoomHistoryThreadsRow {
        fn dispose(&self) {
            self.disconnect_sender();
        }
    }

    impl WidgetImpl for RoomHistoryThreadsRow {}
    impl BinImpl for RoomHistoryThreadsRow {}

    impl RoomHistoryThreadsRow {
        /// Disconnect the signals of the current sender.
        fn disconnect_sender(&self) {
            if let Some(handler) = self.sender_handler.take()
                && let Some(sender) = self
                    .entry
                    .borrow()
                    .as_ref()
                    .and_then(ThreadListEntry::sender)
            {
                sender.disconnect(handler);
            }
        }

        /// Set the thread presented by this row.
        fn set_entry(&self, entry: Option<ThreadListEntry>) {
            if *self.entry.borrow() == entry {
                return;
            }

            self.disconnect_sender();
            self.entry.replace(entry.clone());

            if let Some(entry) = entry {
                if let Some(sender) = entry.sender() {
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
                self.body_label.set_label(&entry.body());

                // Translators: This is the date and time at which the first message
                // of a thread was sent. See `man strftime` or the documentation of
                // g_date_time_format for the available specifiers:
                // <https://docs.gtk.org/glib/method.DateTime.format.html>
                let format = gettext("%x %R");
                let timestamp = entry.timestamp().format(&format).unwrap_or_default();
                self.timestamp_label.set_label(&timestamp);

                let count = entry.num_replies();
                self.replies_label.set_label(&ngettext_f(
                    // Translators: Do NOT translate the content between '{' and
                    // '}', this is a variable name.
                    "1 reply",
                    "{n} replies",
                    count,
                    &[("n", &count.to_string())],
                ));

                let latest = entry.latest_body().map(|body| {
                    if let Some(sender) = entry.latest_sender() {
                        // Translators: Do NOT translate the content between '{' and
                        // '}', these are variable names. This is the preview of the
                        // latest reply in a thread, like "Alice: What a good idea".
                        gettext_f(
                            "{user}: {message}",
                            &[("user", &sender.disambiguated_name()), ("message", &body)],
                        )
                    } else {
                        body
                    }
                });
                self.latest_label
                    .set_label(latest.as_deref().unwrap_or_default());
                self.latest_label.set_visible(latest.is_some());
            } else {
                self.avatar.set_data(None::<AvatarData>);
                self.sender_label.set_label("");
                self.body_label.set_label("");
                self.timestamp_label.set_label("");
                self.replies_label.set_label("");
                self.latest_label.set_label("");
            }

            self.obj().notify_entry();
        }

        /// Update the label presenting the sender of the thread root.
        fn update_sender_label(&self) {
            let name = self
                .entry
                .borrow()
                .as_ref()
                .and_then(ThreadListEntry::sender)
                .map(|sender| sender.disambiguated_name())
                .unwrap_or_default();

            self.sender_label.set_label(&name);
        }
    }
}

glib::wrapper! {
    /// A row presenting a thread of a room.
    pub struct RoomHistoryThreadsRow(ObjectSubclass<imp::RoomHistoryThreadsRow>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RoomHistoryThreadsRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for RoomHistoryThreadsRow {
    fn default() -> Self {
        Self::new()
    }
}

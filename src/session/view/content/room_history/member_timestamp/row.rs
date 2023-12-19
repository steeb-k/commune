use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

use super::MemberTimestamp;
use crate::{system_settings::ClockFormat, Application};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/member_timestamp/row.ui"
    )]
    #[properties(wrapper_type = super::MemberTimestampRow)]
    pub struct MemberTimestampRow {
        #[template_child]
        pub timestamp: TemplateChild<gtk::Label>,
        /// The `MemberTimestamp` presented by this row.
        #[property(get, set = Self::set_data, explicit_notify, nullable)]
        pub data: glib::WeakRef<MemberTimestamp>,
        pub system_settings_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberTimestampRow {
        const NAME: &'static str = "ContentMemberTimestampRow";
        type Type = super::MemberTimestampRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MemberTimestampRow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let system_settings = Application::default().system_settings();
            let system_settings_handler = system_settings.connect_notify_local(
                Some("clock-format"),
                clone!(@weak obj => move |_,_| {
                    obj.update_timestamp();
                }),
            );
            self.system_settings_handler
                .replace(Some(system_settings_handler));
        }

        fn dispose(&self) {
            if let Some(handler) = self.system_settings_handler.take() {
                Application::default().system_settings().disconnect(handler);
            }
        }
    }

    impl WidgetImpl for MemberTimestampRow {}
    impl BinImpl for MemberTimestampRow {}

    impl MemberTimestampRow {
        /// Set the `MemberTimestamp` presented by this row.
        fn set_data(&self, data: Option<MemberTimestamp>) {
            if self.data.upgrade() == data {
                return;
            }
            let obj = self.obj();

            self.data.set(data.as_ref());
            obj.notify_data();

            obj.update_timestamp();
        }
    }
}

glib::wrapper! {
    /// A row displaying a room member and timestamp.
    pub struct MemberTimestampRow(ObjectSubclass<imp::MemberTimestampRow>)
        @extends gtk::Widget, adw::Bin;
}

impl MemberTimestampRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The formatted date and time of this receipt.
    fn update_timestamp(&self) {
        let imp = self.imp();

        let Some(timestamp) = self.data().map(|d| d.timestamp()).filter(|t| *t > 0) else {
            // No timestamp.
            imp.timestamp.set_visible(false);
            return;
        };

        let datetime = glib::DateTime::from_unix_utc(timestamp as i64)
            .and_then(|t| t.to_local())
            .unwrap();

        let clock_format = Application::default().system_settings().clock_format();

        let format = if clock_format == ClockFormat::TwelveHours {
            // Translators: this is a date and a time in 12h format.
            // For example, "May 5 at 01:20 PM".
            // Do not change the time format as it will follow the system settings.
            // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
            gettext("%B %-e at %I∶%M %p")
        } else {
            // Translators: this is a date and a time in 24h format.
            // For example, "May 5 at 13:20".
            // Do not change the time format as it will follow the system settings.
            // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
            gettext("%B %-e at %H∶%M")
        };
        let label = datetime.format(&format).unwrap();

        imp.timestamp.set_label(&label);
        imp.timestamp.set_visible(true);
    }
}

impl Default for MemberTimestampRow {
    fn default() -> Self {
        Self::new()
    }
}

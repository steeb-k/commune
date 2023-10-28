use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{glib, prelude::*};

pub mod row;

use crate::session::model::Member;

mod imp {
    use std::cell::Cell;

    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default)]
    pub struct MemberTimestamp {
        /// The room member.
        pub member: glib::WeakRef<Member>,
        /// The timestamp, in milliseconds since Unix Epoch.
        ///
        /// A value of 0 means no timestamp.
        pub timestamp: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberTimestamp {
        const NAME: &'static str = "ContentMemberTimestamp";
        type Type = super::MemberTimestamp;
    }

    impl ObjectImpl for MemberTimestamp {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<Member>("member")
                        .construct_only()
                        .build(),
                    glib::ParamSpecUInt64::builder("timestamp")
                        .construct_only()
                        .build(),
                    glib::ParamSpecString::builder("datetime")
                        .read_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "member" => obj.member().to_value(),
                "timestamp" => obj.timestamp().to_value(),
                "datetime" => obj.datetime().to_value(),
                _ => unimplemented!(),
            }
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "member" => obj.set_member(value.get::<Option<Member>>().unwrap().as_ref()),
                "timestamp" => obj.set_timestamp(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    /// A room member and a timestamp.
    pub struct MemberTimestamp(ObjectSubclass<imp::MemberTimestamp>);
}

impl MemberTimestamp {
    /// Constructs a new `MemberTimestamp` with the given member and
    /// timestamp.
    pub fn new(member: &Member, timestamp: Option<u64>) -> Self {
        glib::Object::builder()
            .property("member", member)
            .property("timestamp", timestamp.unwrap_or_default())
            .build()
    }

    /// The room member of this read receipt.
    pub fn member(&self) -> Option<Member> {
        self.imp().member.upgrade()
    }

    /// Set the room member of this read receipt.
    fn set_member(&self, member: Option<&Member>) {
        let Some(member) = member else {
            // Ignore if there is no member.
            return;
        };

        self.imp().member.set(Some(member));
        self.notify("member");
    }

    /// The timestamp of this read receipt, in milliseconds since Unix Epoch, if
    /// any.
    ///
    /// A value of 0 means no timestamp.
    pub fn timestamp(&self) -> u64 {
        self.imp().timestamp.get()
    }

    /// Set the timestamp of this read receipt.
    pub fn set_timestamp(&self, ts: u64) {
        if self.timestamp() == ts {
            return;
        }

        self.imp().timestamp.set(ts);
        self.notify("timestamp");
    }

    /// The formatted date and time of this receipt.
    pub fn datetime(&self) -> String {
        let timestamp = self.timestamp();

        if timestamp == 0 {
            // No timestamp.
            return String::new();
        }

        let datetime = glib::DateTime::from_unix_utc((timestamp / 1000) as i64)
            .and_then(|t| t.to_local())
            .unwrap();

        // FIXME: Use system setting.
        let local_time = datetime.format("%X").unwrap().as_str().to_ascii_lowercase();
        let is_12h_format = local_time.ends_with("am") || local_time.ends_with("pm");

        let format = if is_12h_format {
            // Translators: this is a date and a time in 12h format.
            // For example, "May 5 at 1:20 PM".
            // Do not change the time format as it will follow the system settings.
            // Please use `-` before specifiers that add spaces on single digits.
            // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
            gettext("%B %-e at %-l∶%M %p")
        } else {
            // Translators: this is a date and a time in 24h format.
            // For example, "May 5 at 13:20".
            // Do not change the time format as it will follow the system settings.
            // Please use `-` before specifiers that add spaces on single digits.
            // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
            gettext("%B %-e at %-k∶%M")
        };
        datetime.format(&format).unwrap().to_string()
    }
}

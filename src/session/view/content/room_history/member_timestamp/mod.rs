use adw::subclass::prelude::*;
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
        /// The timestamp, in seconds since Unix Epoch.
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
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "member" => obj.member().to_value(),
                "timestamp" => obj.timestamp().to_value(),
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

    /// The timestamp of this read receipt, in seconds since Unix Epoch, if
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
}

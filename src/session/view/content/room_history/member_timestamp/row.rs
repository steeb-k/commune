use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, CompositeTemplate};

use super::MemberTimestamp;

mod imp {
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/member_timestamp/row.ui"
    )]
    pub struct MemberTimestampRow {
        /// The `MemberTimestamp` presented by this row.
        pub data: glib::WeakRef<MemberTimestamp>,
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

    impl ObjectImpl for MemberTimestampRow {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecObject::builder::<MemberTimestamp>("data")
                    .explicit_notify()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "data" => obj.set_data(value.get::<Option<MemberTimestamp>>().unwrap().as_ref()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "data" => obj.data().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for MemberTimestampRow {}
    impl BinImpl for MemberTimestampRow {}
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

    /// The `MemberTimestamp` presented by this row.
    pub fn data(&self) -> Option<MemberTimestamp> {
        self.imp().data.upgrade()
    }

    /// Set the `MemberTimestamp` presented by this row.
    pub fn set_data(&self, data: Option<&MemberTimestamp>) {
        if self.data().as_ref() == data {
            return;
        }

        self.imp().data.set(data);
        self.notify("data");
    }
}

impl Default for MemberTimestampRow {
    fn default() -> Self {
        Self::new()
    }
}

use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::{
    components::{Avatar, Badge},
    session::model::Member,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/members_page/members_list_view/member_row.ui"
    )]
    pub struct MemberRow {
        pub member: RefCell<Option<Member>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberRow {
        const NAME: &'static str = "ContentMemberRow";
        type Type = super::MemberRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            Avatar::static_type();
            Badge::static_type();

            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MemberRow {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecObject::builder::<Member>("member")
                    .explicit_notify()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "member" => {
                    self.obj().set_member(value.get().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "member" => self.obj().member().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for MemberRow {}
    impl BoxImpl for MemberRow {}
}

glib::wrapper! {
    pub struct MemberRow(ObjectSubclass<imp::MemberRow>)
        @extends gtk::Widget, gtk::Box, @implements gtk::Accessible;
}

impl MemberRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The member displayed by this row.
    pub fn member(&self) -> Option<Member> {
        self.imp().member.borrow().clone()
    }

    /// Set the member displayed by this row.
    pub fn set_member(&self, member: Option<Member>) {
        let imp = self.imp();

        if self.member() == member {
            return;
        }

        imp.member.replace(member);
        self.notify("member");
    }
}

use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::{
    components::{Avatar, RoleBadge},
    session::model::Member,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/members_page/members_list_view/member_row.ui"
    )]
    #[properties(wrapper_type = super::MemberRow)]
    pub struct MemberRow {
        /// The room member presented by this row.
        #[property(get, set = Self::set_member, explicit_notify, nullable)]
        pub member: RefCell<Option<Member>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberRow {
        const NAME: &'static str = "ContentMemberRow";
        type Type = super::MemberRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            Avatar::ensure_type();
            RoleBadge::ensure_type();

            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MemberRow {}

    impl WidgetImpl for MemberRow {}
    impl BoxImpl for MemberRow {}

    impl MemberRow {
        /// Set the member displayed by this row.
        fn set_member(&self, member: Option<Member>) {
            if *self.member.borrow() == member {
                return;
            }

            self.member.replace(member);
            self.obj().notify_member();
        }
    }
}

glib::wrapper! {
    /// A row presenting a room member.
    pub struct MemberRow(ObjectSubclass<imp::MemberRow>)
        @extends gtk::Widget, gtk::Box, @implements gtk::Accessible;
}

impl MemberRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

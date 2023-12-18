use gtk::{
    gio, glib,
    glib::{prelude::*, subclass::prelude::*},
};

use crate::session::model::Membership;

mod imp {
    use std::cell::{Cell, OnceCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::MembershipSubpageItem)]
    pub struct MembershipSubpageItem {
        /// The membership state used to filter the subpage's list.
        #[property(get, construct_only, builder(Membership::default()))]
        pub state: Cell<Membership>,
        /// The model used for the subpage.
        #[property(get, construct_only)]
        pub model: OnceCell<gio::ListModel>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MembershipSubpageItem {
        const NAME: &'static str = "MembersPageMembershipSubpageItem";
        type Type = super::MembershipSubpageItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for MembershipSubpageItem {}
}

glib::wrapper! {
    /// An item representing a subpage for a subset of the list of room members filtered by membership.
    pub struct MembershipSubpageItem(ObjectSubclass<imp::MembershipSubpageItem>);
}

impl MembershipSubpageItem {
    pub fn new(state: Membership, model: &impl IsA<gio::ListModel>) -> Self {
        glib::Object::builder()
            .property("state", state)
            .property("model", model)
            .build()
    }
}

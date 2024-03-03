use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::{OwnedMxcUri, OwnedUserId};

use crate::{
    components::PillSource,
    prelude::*,
    session::model::{Session, User},
};

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Invitee)]
    pub struct Invitee {
        /// Whether this user is invited.
        #[property(get, set = Self::set_invited, explicit_notify)]
        pub invited: Cell<bool>,
        /// The reason the user can't be invited.
        #[property(get, set = Self::set_invite_exception, explicit_notify, nullable)]
        pub invite_exception: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Invitee {
        const NAME: &'static str = "Invitee";
        type Type = super::Invitee;
        type ParentType = User;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Invitee {}

    impl PillSourceImpl for Invitee {
        fn identifier(&self) -> String {
            self.obj().upcast_ref::<User>().user_id_string()
        }
    }

    impl Invitee {
        /// Set whether this user is invited.
        fn set_invited(&self, invited: bool) {
            if self.invited.get() == invited {
                return;
            }

            self.invited.set(invited);
            self.obj().notify_invited();
        }

        /// Set the reason the user can't be invited.
        fn set_invite_exception(&self, exception: Option<String>) {
            if exception == *self.invite_exception.borrow() {
                return;
            }

            self.invite_exception.replace(exception);
            self.obj().notify_invite_exception();
        }
    }
}

glib::wrapper! {
    /// A possible invitee.
    pub struct Invitee(ObjectSubclass<imp::Invitee>) @extends PillSource, User;
}

impl Invitee {
    pub fn new(
        session: &Session,
        user_id: OwnedUserId,
        display_name: Option<&str>,
        avatar_url: Option<OwnedMxcUri>,
    ) -> Self {
        let obj: Self = glib::Object::builder()
            .property("session", session)
            .property("display-name", display_name)
            .build();

        obj.set_avatar_url(avatar_url);
        obj.upcast_ref::<User>().imp().set_user_id(user_id);
        obj
    }
}

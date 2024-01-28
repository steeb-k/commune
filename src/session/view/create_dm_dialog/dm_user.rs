use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::{OwnedMxcUri, OwnedUserId};

use crate::{
    prelude::*,
    session::model::{Room, Session, User},
};

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::DmUser)]
    pub struct DmUser {
        /// The direct chat with this user, if any.
        #[property(get, set = Self::set_direct_chat, explicit_notify, nullable)]
        pub direct_chat: glib::WeakRef<Room>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DmUser {
        const NAME: &'static str = "CreateDmDialogUser";
        type Type = super::DmUser;
        type ParentType = User;
    }

    #[glib::derived_properties]
    impl ObjectImpl for DmUser {}

    impl DmUser {
        /// Set the direct chat with this user.
        fn set_direct_chat(&self, direct_chat: Option<Room>) {
            if self.direct_chat.upgrade() == direct_chat {
                return;
            }

            self.direct_chat.set(direct_chat.as_ref());
            self.obj().notify_direct_chat();
        }
    }
}

glib::wrapper! {
    /// A User in the context of creating a direct chat.
    pub struct DmUser(ObjectSubclass<imp::DmUser>) @extends User;
}

impl DmUser {
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

        let user = obj.upcast_ref::<User>();
        user.set_avatar_url(avatar_url);
        user.imp().set_user_id(user_id);
        obj.set_direct_chat(user.direct_chat());

        obj
    }
}

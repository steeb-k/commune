use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::{MxcUri, UserId};

use crate::{
    prelude::*,
    session::model::{Room, Session, User},
    spawn,
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
    impl ObjectImpl for DmUser {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            spawn!(clone!(@weak obj => async move {
                let direct_chat = obj.upcast_ref::<User>().direct_chat().await;
                obj.set_direct_chat(direct_chat);
            }));
        }
    }

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
        user_id: &UserId,
        display_name: Option<&str>,
        avatar_url: Option<&MxcUri>,
    ) -> Self {
        let obj: Self = glib::Object::builder()
            .property("session", session)
            .property("user-id", user_id.as_str())
            .property("display-name", display_name)
            .build();
        // FIXME: we should make the avatar_url settable as property
        obj.set_avatar_url(avatar_url.map(std::borrow::ToOwned::to_owned));
        obj
    }
}

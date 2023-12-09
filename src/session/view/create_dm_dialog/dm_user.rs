use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::{MxcUri, UserId};

use crate::{
    prelude::*,
    session::model::{Room, Session, User},
    spawn,
};

mod imp {
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default)]
    pub struct DmUser {
        pub direct_chat: glib::WeakRef<Room>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DmUser {
        const NAME: &'static str = "CreateDmDialogUser";
        type Type = super::DmUser;
        type ParentType = User;
    }

    impl ObjectImpl for DmUser {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecObject::builder::<Room>("direct-chat")
                    .read_only()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "direct-chat" => obj.direct_chat().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            spawn!(clone!(@weak obj => async move {
                let direct_chat = obj.upcast_ref::<User>().direct_chat().await;
                obj.set_direct_chat(direct_chat.as_ref());
            }));
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

    /// Get the direct chat with this user, if any.
    pub fn direct_chat(&self) -> Option<Room> {
        self.imp().direct_chat.upgrade()
    }

    /// Set the direct chat with this user.
    fn set_direct_chat(&self, direct_chat: Option<&Room>) {
        if self.direct_chat().as_ref() == direct_chat {
            return;
        }

        self.imp().direct_chat.set(direct_chat);
        self.notify("direct-chat");
    }
}

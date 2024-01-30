use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::OwnedUserId;
use tracing::error;

use super::{Session, User};
use crate::{components::PillSource, prelude::*, spawn_tokio};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct RemoteUser {}

    #[glib::object_subclass]
    impl ObjectSubclass for RemoteUser {
        const NAME: &'static str = "RemoteUser";
        type Type = super::RemoteUser;
        type ParentType = User;
    }

    impl ObjectImpl for RemoteUser {}

    impl PillSourceImpl for RemoteUser {
        fn identifier(&self) -> String {
            self.obj().upcast_ref::<User>().user_id_string()
        }
    }
}

glib::wrapper! {
    /// A User that can only be updated by making remote calls, i.e. it won't be updated via sync.
    pub struct RemoteUser(ObjectSubclass<imp::RemoteUser>) @extends PillSource, User;
}

impl RemoteUser {
    pub fn new(session: &Session, user_id: OwnedUserId) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        obj.upcast_ref::<User>().imp().set_user_id(user_id);
        obj
    }

    /// Request this user's profile from the homeserver.
    pub async fn load_profile(&self) {
        let client = self.session().client();
        let user_id = self.user_id();

        let user_id_clone = user_id.clone();
        let handle = spawn_tokio!(async move { client.get_profile(&user_id_clone).await });

        let profile = match handle.await.unwrap() {
            Ok(profile) => profile,
            Err(error) => {
                error!("Failed to load profile for user `{user_id}`: {error}");
                return;
            }
        };

        self.set_name(profile.displayname);
        self.set_avatar_url(profile.avatar_url);
    }
}

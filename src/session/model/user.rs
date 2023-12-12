use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    encryption::identities::UserIdentity,
    ruma::{OwnedMxcUri, OwnedUserId, UserId},
};
use tracing::error;

use crate::{
    components::Pill,
    prelude::*,
    session::model::{
        AvatarData, AvatarImage, AvatarUriSource, IdentityVerification, Session, VerificationState,
    },
    spawn, spawn_tokio,
};

#[glib::flags(name = "UserActions")]
pub enum UserActions {
    VERIFY = 0b00000001,
}

impl Default for UserActions {
    fn default() -> Self {
        Self::empty()
    }
}

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::User)]
    pub struct User {
        /// The ID of this user.
        #[property(get = Self::user_id, set = Self::set_user_id, construct_only, type = String)]
        pub user_id: OnceCell<OwnedUserId>,
        /// The display name of this user.
        #[property(get = Self::display_name, set = Self::set_display_name, explicit_notify, nullable)]
        pub display_name: RefCell<String>,
        /// The current session.
        #[property(get, construct_only)]
        pub session: OnceCell<Session>,
        /// The [`AvatarData`] of this user.
        #[property(get)]
        pub avatar_data: OnceCell<AvatarData>,
        /// Whether this user has been verified.
        #[property(get)]
        pub verified: Cell<bool>,
        /// The actions the currently logged-in user is allowed to perform on
        /// this user.
        #[property(get = Self::allowed_actions)]
        pub allowed_actions: PhantomData<UserActions>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for User {
        const NAME: &'static str = "User";
        type Type = super::User;
    }

    #[glib::derived_properties]
    impl ObjectImpl for User {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let avatar_data = AvatarData::with_image(AvatarImage::new(
                &obj.session(),
                None,
                AvatarUriSource::User,
            ));
            self.avatar_data.set(avatar_data).unwrap();

            obj.bind_property("display-name", &obj.avatar_data(), "display-name")
                .sync_create()
                .build();

            obj.init_is_verified();
        }
    }

    impl User {
        /// The ID of this user.
        fn user_id(&self) -> String {
            self.user_id.get().unwrap().to_string()
        }

        /// Set the ID of this user.
        fn set_user_id(&self, user_id: String) {
            self.user_id.set(UserId::parse(user_id).unwrap()).unwrap();
        }

        /// The display name of this user.
        fn display_name(&self) -> String {
            let display_name = self.display_name.borrow().clone();

            if !display_name.is_empty() {
                display_name
            } else {
                self.user_id.get().unwrap().localpart().to_owned()
            }
        }

        /// Set the display name of this user.
        fn set_display_name(&self, display_name: Option<String>) {
            if Some(&*self.display_name.borrow()) == display_name.as_ref() {
                return;
            }
            self.display_name.replace(display_name.unwrap_or_default());
            self.obj().notify_display_name();
        }

        /// The actions the currently logged-in user is allowed to perform on
        /// this user.
        fn allowed_actions(&self) -> UserActions {
            let is_other = self.session.get().unwrap().user_id() != self.user_id.get().unwrap();

            if !self.verified.get() && is_other {
                UserActions::VERIFY
            } else {
                UserActions::empty()
            }
        }
    }
}

glib::wrapper! {
    /// `glib::Object` representation of a Matrix user.
    pub struct User(ObjectSubclass<imp::User>);
}

impl User {
    /// Constructs a new user with the given user ID for the given session.
    pub fn new(session: &Session, user_id: &UserId) -> Self {
        glib::Object::builder()
            .property("session", session)
            .property("user-id", user_id.as_str())
            .build()
    }

    /// Get the cryptographic identity (aka cross-signing identity) of this
    /// user.
    pub async fn crypto_identity(&self) -> Option<UserIdentity> {
        let encryption = self.session().client().encryption();
        let user_id = UserExt::user_id(self);
        let handle = spawn_tokio!(async move { encryption.get_user_identity(&user_id).await });

        match handle.await.unwrap() {
            Ok(identity) => identity,
            Err(error) => {
                error!("Failed to find crypto identity: {error}");
                None
            }
        }
    }

    /// Start a verification of the identity of this user.
    pub async fn verify_identity(&self) -> IdentityVerification {
        let request = IdentityVerification::create(&self.session(), Some(self.clone())).await;
        self.session().verification_list().add(request.clone());
        // FIXME: actually listen to room events to get updates for verification state
        request.connect_notify_local(
            Some("state"),
            clone!(@weak self as obj => move |request,_| {
                if request.state() == VerificationState::Completed {
                    obj.init_is_verified();
                }
            }),
        );
        request
    }

    /// Load whether this user is verified.
    fn init_is_verified(&self) {
        spawn!(clone!(@weak self as obj => async move {
            let verified = obj.crypto_identity().await.map_or(false, |i| i.is_verified());

            if verified == obj.verified() {
                return;
            }

            obj.imp().verified.set(verified);
            obj.notify_verified();
            obj.notify_allowed_actions();
        }));
    }
}

pub trait UserExt: IsA<User> {
    /// The current session.
    fn session(&self) -> Session {
        self.upcast_ref().session()
    }

    /// The ID of this user.
    fn user_id(&self) -> OwnedUserId {
        self.upcast_ref().imp().user_id.get().unwrap().clone()
    }

    /// The display name of this user.
    fn display_name(&self) -> String {
        self.upcast_ref().display_name()
    }

    /// Set the display name of this user.
    fn set_display_name(&self, display_name: Option<String>) {
        self.upcast_ref().set_display_name(display_name);
    }

    /// The [`AvatarData`] of this user.
    fn avatar_data(&self) -> AvatarData {
        self.upcast_ref().avatar_data()
    }

    /// Set the avatar URL of this user.
    fn set_avatar_url(&self, uri: Option<OwnedMxcUri>) {
        self.avatar_data()
            .image()
            .unwrap()
            .set_uri(uri.map(String::from));
    }

    /// The actions the currently logged-in user is allowed to perform on this
    /// user.
    fn allowed_actions(&self) -> UserActions {
        self.upcast_ref().allowed_actions()
    }

    /// Get a `Pill` representing this `User`.
    fn to_pill(&self) -> Pill {
        let user = self.upcast_ref();
        Pill::for_user(user)
    }

    /// Get the HTML mention representation for this `User`.
    fn html_mention(&self) -> String {
        let uri = self.user_id().matrix_to_uri();
        format!("<a href=\"{uri}\">{}</a>", self.display_name())
    }

    /// Load the user profile from the homeserver.
    ///
    /// This overwrites the already loaded display name and avatar.
    fn load_profile(&self) {
        let client = self.session().client();
        let user_id = self.user_id();
        let user = self.upcast_ref::<User>();

        let handle = spawn_tokio!(async move { client.get_profile(&user_id).await });

        spawn!(clone!(@weak user => async move {
            match handle.await.unwrap() {
                Ok(response) => {
                    user.set_display_name(response.displayname);
                    user.set_avatar_url(response.avatar_url);
                },
                Err(error) => {
                    error!("Failed to load user profile for {}: {}", user.user_id(), error);
                }
            };
        }));
    }
}

impl<T: IsA<User>> UserExt for T {}

unsafe impl<T: ObjectImpl + 'static> IsSubclassable<T> for User {
    fn class_init(class: &mut glib::Class<Self>) {
        <glib::Object as IsSubclassable<T>>::class_init(class.upcast_ref_mut());
    }

    fn instance_init(instance: &mut glib::subclass::InitializingObject<T>) {
        <glib::Object as IsSubclassable<T>>::instance_init(instance);
    }
}

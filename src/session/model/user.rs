use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::encryption::identities::UserIdentity;
use ruma::{
    api::client::room::create_room,
    assign,
    events::{room::encryption::RoomEncryptionEventContent, InitialStateEvent},
    OwnedMxcUri, OwnedUserId,
};
use tracing::{debug, error};

use super::{AvatarData, AvatarImage, AvatarUriSource, IdentityVerification, Room, Session};
use crate::{components::Pill, prelude::*, spawn, spawn_tokio};

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
        pub user_id: OnceCell<OwnedUserId>,
        /// The ID of this user, as a string.
        #[property(get = Self::user_id_string)]
        pub user_id_string: PhantomData<String>,
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
        /// Whether this user is currently ignored..
        #[property(get)]
        pub is_ignored: Cell<bool>,
        ignored_handler: RefCell<Option<glib::SignalHandlerId>>,
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
        }

        fn dispose(&self) {
            if let Some(session) = self.session.get() {
                if let Some(handler) = self.ignored_handler.take() {
                    session.ignored_users().disconnect(handler);
                }
            }
        }
    }

    impl User {
        /// The ID of this user, as a string.
        fn user_id_string(&self) -> String {
            self.user_id.get().unwrap().to_string()
        }

        /// Set the ID of this user.
        pub fn set_user_id(&self, user_id: OwnedUserId) {
            self.user_id.set(user_id.clone()).unwrap();

            let obj = self.obj();
            obj.bind_property("display-name", &obj.avatar_data(), "display-name")
                .sync_create()
                .build();

            let ignored_users = self.session.get().unwrap().ignored_users();
            let ignored_handler = ignored_users.connect_items_changed(
                clone!(@weak self as imp => move |ignored_users, _, _, _| {
                    let user_id = imp.user_id.get().unwrap();
                    let is_ignored = ignored_users.contains(user_id);

                    if imp.is_ignored.get() != is_ignored {
                        imp.is_ignored.set(is_ignored);
                        imp.obj().notify_is_ignored();
                    }
                }),
            );
            self.is_ignored.set(ignored_users.contains(&user_id));
            self.ignored_handler.replace(Some(ignored_handler));

            obj.init_is_verified();
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
    pub fn new(session: &Session, user_id: OwnedUserId) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        obj.imp().set_user_id(user_id);
        obj
    }

    /// Get the cryptographic identity (aka cross-signing identity) of this
    /// user.
    pub async fn crypto_identity(&self) -> Option<UserIdentity> {
        let encryption = self.session().client().encryption();
        let user_id = self.user_id().clone();
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
    pub async fn verify_identity(&self) -> Result<IdentityVerification, ()> {
        self.session()
            .verification_list()
            .create(Some(self.clone()))
            .await
    }

    /// Load whether this user is verified.
    fn init_is_verified(&self) {
        spawn!(clone!(@weak self as obj => async move {
            let verified = obj.crypto_identity().await.is_some_and(|i| i.is_verified());

            if verified == obj.verified() {
                return;
            }

            obj.imp().verified.set(verified);
            obj.notify_verified();
            obj.notify_allowed_actions();
        }));
    }

    /// The existing direct chat with this user, if any.
    ///
    /// A direct chat is a joined room marked as direct, with only our own user
    /// and the other user in it.
    pub async fn direct_chat(&self) -> Option<Room> {
        self.session().room_list().direct_chat(self.user_id()).await
    }

    /// Create an encrypted direct chat with this user.
    async fn create_direct_chat(&self) -> Result<Room, matrix_sdk::Error> {
        let request = assign!(create_room::v3::Request::new(),
        {
            is_direct: true,
            invite: vec![self.user_id().clone()],
            preset: Some(create_room::v3::RoomPreset::TrustedPrivateChat),
            initial_state: vec![
               InitialStateEvent::new(RoomEncryptionEventContent::with_recommended_defaults()).to_raw_any(),
            ],
        });

        let client = self.session().client();
        let handle = spawn_tokio!(async move { client.create_room(request).await });

        match handle.await.unwrap() {
            Ok(matrix_room) => {
                let room = self
                    .session()
                    .room_list()
                    .get_wait(matrix_room.room_id())
                    .await
                    .expect("The newly created room was not found");
                Ok(room)
            }
            Err(error) => {
                error!("Failed to create direct chat: {error}");
                Err(error)
            }
        }
    }

    /// Get or create a direct chat with this user.
    ///
    /// If there is no existing direct chat, a new one is created. If a direct
    /// chat exists but the other user has left the room, we re-invite them.
    pub async fn get_or_create_direct_chat(&self) -> Result<Room, ()> {
        let user_id = self.user_id();

        if let Some(room) = self.direct_chat().await {
            debug!("Using existing direct chat with {user_id}…");
            return Ok(room);
        }

        debug!("Creating direct chat with {user_id}…");
        self.create_direct_chat().await.map_err(|_| ())
    }

    /// Ignore this user.
    pub async fn ignore(&self) -> Result<(), ()> {
        self.session().ignored_users().add(self.user_id()).await
    }

    /// Stop ignoring this user.
    pub async fn stop_ignoring(&self) -> Result<(), ()> {
        self.session().ignored_users().remove(self.user_id()).await
    }
}

pub trait UserExt: IsA<User> {
    /// The current session.
    fn session(&self) -> Session {
        self.upcast_ref().session()
    }

    /// The ID of this user.
    fn user_id(&self) -> &OwnedUserId {
        self.upcast_ref().imp().user_id.get().unwrap()
    }

    /// Whether this user is the same as the session's user.
    fn is_own_user(&self) -> bool {
        self.session().user_id() == self.user_id()
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
        let user_id = self.user_id().clone();
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

    /// Whether this user is currently ignored.
    fn is_ignored(&self) -> bool {
        self.upcast_ref().is_ignored()
    }

    /// Conntect to the signal emitted when the display name changes.
    fn connect_display_name_notify<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.upcast_ref()
            .connect_display_name_notify(move |user| f(user.downcast_ref().unwrap()))
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

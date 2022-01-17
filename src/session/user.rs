use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::identifiers::{MxcUri, UserId};

use crate::session::{Avatar, Session};
use crate::spawn_tokio;
use matrix_sdk::encryption::identities::UserIdentity;

use log::error;

mod imp {
    use super::*;
    use glib::object::WeakRef;
    use once_cell::{sync::Lazy, unsync::OnceCell};
    use std::{cell::RefCell, convert::TryInto};

    #[derive(Debug, Default)]
    pub struct User {
        pub user_id: OnceCell<Box<UserId>>,
        pub display_name: RefCell<Option<String>>,
        pub session: OnceCell<WeakRef<Session>>,
        pub avatar: OnceCell<Avatar>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for User {
        const NAME: &'static str = "User";
        type Type = super::User;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for User {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_string(
                        "user-id",
                        "User id",
                        "The user id of this user",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name of the user",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_object(
                        "avatar",
                        "Avatar",
                        "The avatar of this user",
                        Avatar::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "session",
                        "Session",
                        "The session",
                        Session::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "user-id" => {
                    let user_id = value.get::<&str>().unwrap().try_into().unwrap();
                    self.user_id.set(user_id).unwrap();
                }
                "display-name" => {
                    obj.set_display_name(value.get::<Option<String>>().unwrap());
                }
                "session" => self
                    .session
                    .set(value.get::<Session>().unwrap().downgrade())
                    .unwrap(),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "display-name" => obj.display_name().to_value(),
                "user-id" => obj.user_id().as_str().to_value(),
                "session" => obj.session().to_value(),
                "avatar" => obj.avatar().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            let avatar = Avatar::new(&obj.session(), None);
            self.avatar.set(avatar).unwrap();

            obj.bind_property("display-name", obj.avatar(), "display-name")
                .flags(glib::BindingFlags::SYNC_CREATE)
                .build()
                .unwrap();
        }
    }
}

glib::wrapper! {
    /// `glib::Object` representation of a Matrix user.
    pub struct User(ObjectSubclass<imp::User>);
}

impl User {
    pub fn new(session: &Session, user_id: &UserId) -> Self {
        glib::Object::new(&[("session", session), ("user-id", &user_id.as_str())])
            .expect("Failed to create User")
    }

    pub async fn crypto_identity(&self) -> Option<UserIdentity> {
        let client = self.session().client();
        let user_id = self.user_id().to_owned();
        let handle = spawn_tokio!(async move { client.get_user_identity(&user_id).await });

        match handle.await.unwrap() {
            Ok(identity) => identity,
            Err(error) => {
                error!("Failed to find crypto identity: {}", error);
                None
            }
        }
    }
}

pub trait UserExt: IsA<User> {
    fn session(&self) -> Session {
        let priv_ = imp::User::from_instance(self.upcast_ref());
        priv_.session.get().unwrap().upgrade().unwrap()
    }

    fn user_id(&self) -> &UserId {
        let priv_ = imp::User::from_instance(self.upcast_ref());
        priv_.user_id.get().unwrap()
    }

    fn display_name(&self) -> String {
        let priv_ = imp::User::from_instance(self.upcast_ref());

        if let Some(display_name) = priv_.display_name.borrow().to_owned() {
            display_name
        } else {
            priv_.user_id.get().unwrap().localpart().to_owned()
        }
    }

    fn set_display_name(&self, display_name: Option<String>) {
        if Some(self.display_name()) == display_name {
            return;
        }
        let priv_ = imp::User::from_instance(self.upcast_ref());
        priv_.display_name.replace(display_name);
        self.notify("display-name");
    }

    fn avatar(&self) -> &Avatar {
        let priv_ = imp::User::from_instance(self.upcast_ref());
        priv_.avatar.get().unwrap()
    }

    fn set_avatar_url(&self, url: Option<Box<MxcUri>>) {
        self.avatar().set_url(url);
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

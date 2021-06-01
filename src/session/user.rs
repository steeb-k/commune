use gtk::{gio, glib, prelude::*, subclass::prelude::*};

use crate::session::Session;
use matrix_sdk::{
    events::{room::member::MemberEventContent, StateEvent, StrippedStateEvent},
    identifiers::UserId,
    RoomMember,
};

mod imp {
    use super::*;
    use once_cell::sync::{Lazy, OnceCell};
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct User {
        pub user_id: OnceCell<String>,
        pub display_name: RefCell<Option<String>>,
        pub avatar: RefCell<Option<gio::LoadableIcon>>,
        pub session: OnceCell<Session>,
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
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_object(
                        "avatar",
                        "Avatar",
                        "The avatar of this user",
                        gio::LoadableIcon::static_type(),
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
            _obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "user-id" => {
                    let user_id = value.get().unwrap();
                    self.user_id.set(user_id).unwrap();
                }
                "session" => self.session.set(value.get().unwrap()).unwrap(),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "display-name" => obj.display_name().to_value(),
                "user-id" => self.user_id.get().to_value(),
                "avatar" => self.avatar.borrow().to_value(),
                "session" => obj.session().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct User(ObjectSubclass<imp::User>);
}

/// This is a `glib::Object` rapresentation of matrix users.
impl User {
    pub fn new(session: &Session, user_id: &UserId) -> Self {
        glib::Object::new(&[("session", session), ("user-id", &user_id.to_string())])
            .expect("Failed to create User")
    }

    pub fn session(&self) -> &Session {
        let priv_ = imp::User::from_instance(&self);
        priv_.session.get().unwrap()
    }

    pub fn user_id(&self) -> UserId {
        use std::convert::TryFrom;
        let priv_ = imp::User::from_instance(&self);
        UserId::try_from(priv_.user_id.get().unwrap().as_str()).unwrap()
    }

    pub fn display_name(&self) -> String {
        let priv_ = imp::User::from_instance(&self);

        if let Some(display_name) = priv_.display_name.borrow().to_owned() {
            display_name
        } else {
            priv_
                .user_id
                .get()
                .unwrap()
                .trim_start_matches("@")
                .to_owned()
        }
    }

    /// Update the user based on the the room member state event
    //TODO: create the GLoadableIcon and set `avatar`
    pub fn update_from_room_member(&self, member: &RoomMember) {
        let changed = {
            let priv_ = imp::User::from_instance(&self);
            let user_id = priv_.user_id.get().unwrap();
            if member.user_id().as_str() != user_id {
                return;
            };

            //let content = event.content;
            let display_name = member.display_name().map(|name| name.to_owned());

            let mut current_display_name = priv_.display_name.borrow_mut();
            if *current_display_name != display_name {
                *current_display_name = display_name;
                true
            } else {
                false
            }
        };

        if changed {
            self.notify("display-name");
        }
    }

    /// Update the user based on the the room member state event
    //TODO: create the GLoadableIcon and set `avatar`
    pub fn update_from_member_event(&self, event: &StateEvent<MemberEventContent>) {
        let changed = {
            let priv_ = imp::User::from_instance(&self);
            let user_id = priv_.user_id.get().unwrap();
            if event.sender.as_str() != user_id {
                return;
            };

            let display_name = if let Some(display_name) = &event.content.displayname {
                Some(display_name.to_owned())
            } else {
                event
                    .content
                    .third_party_invite
                    .as_ref()
                    .map(|i| i.display_name.to_owned())
            };

            let mut current_display_name = priv_.display_name.borrow_mut();
            if *current_display_name != display_name {
                *current_display_name = display_name;
                true
            } else {
                false
            }
        };

        if changed {
            self.notify("display-name");
        }
    }

    /// Update the user based on the the stripped room member state event
    //TODO: create the GLoadableIcon and set `avatar`
    pub fn update_from_stripped_member_event(
        &self,
        event: &StrippedStateEvent<MemberEventContent>,
    ) {
        let changed = {
            let priv_ = imp::User::from_instance(&self);
            let user_id = priv_.user_id.get().unwrap();
            if event.sender.as_str() != user_id {
                return;
            };

            let display_name = if let Some(display_name) = &event.content.displayname {
                Some(display_name.to_owned())
            } else {
                event
                    .content
                    .third_party_invite
                    .as_ref()
                    .map(|i| i.display_name.to_owned())
            };

            let mut current_display_name = priv_.display_name.borrow_mut();
            if *current_display_name != display_name {
                *current_display_name = display_name;
                true
            } else {
                false
            }
        };

        if changed {
            self.notify("display-name");
        }
    }
}

use gtk::glib;
use gtk::subclass::prelude::*;
use matrix_sdk::ruma::events::room::member::MemberEventContent;
use matrix_sdk::ruma::events::{StrippedStateEvent, SyncStateEvent};
use matrix_sdk::ruma::identifiers::UserId;
use matrix_sdk::RoomMember;

use crate::prelude::*;
use crate::session::{Room, User};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Member {}

    #[glib::object_subclass]
    impl ObjectSubclass for Member {
        const NAME: &'static str = "Member";
        type Type = super::Member;
        type ParentType = User;
    }

    impl ObjectImpl for Member {}
}

glib::wrapper! {
    /// A User in the context of a given room.
    pub struct Member(ObjectSubclass<imp::Member>) @extends User;
}

impl Member {
    pub fn new(room: &Room, user_id: &UserId) -> Self {
        let session = room.session();
        glib::Object::new(&[("session", &session), ("user-id", &user_id.as_str())])
            .expect("Failed to create Member")
    }
}

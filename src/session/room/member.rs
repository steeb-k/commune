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

    /// Update the user based on the the room member state event
    pub fn update_from_room_member(&self, member: &RoomMember) {
        if member.user_id() != self.user_id() {
            return;
        };

        let display_name = member.display_name().map(|name| name.to_owned());
        self.avatar().set_url(member.avatar_url().cloned());

        if Some(self.display_name()) != display_name {
            self.set_display_name(display_name);
        }
    }

    /// Update the user based on the the room member state event
    pub fn update_from_member_event(&self, event: &SyncStateEvent<MemberEventContent>) {
        if &event.sender != self.user_id() {
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

        self.avatar().set_url(event.content.avatar_url.to_owned());

        if Some(self.display_name()) != display_name {
            self.set_display_name(display_name);
        }
    }

    /// Update the user based on the the stripped room member state event
    pub fn update_from_stripped_member_event(
        &self,
        event: &StrippedStateEvent<MemberEventContent>,
    ) {
        if &event.sender != self.user_id() {
            return;
        };

        let display_name = match &event.content.displayname {
            Some(display_name) => Some(display_name.to_owned()),
            None => event
                .content
                .third_party_invite
                .as_ref()
                .map(|i| i.display_name.to_owned()),
        };
        self.avatar().set_url(event.content.avatar_url.to_owned());

        if Some(self.display_name()) != display_name {
            self.set_display_name(display_name)
        }
    }
}

use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    room::RoomMember,
    ruma::{
        events::{
            room::member::{MembershipState, RoomMemberEventContent},
            OriginalSyncStateEvent, StrippedStateEvent,
        },
        OwnedMxcUri, OwnedUserId,
    },
};
use tracing::error;

use super::{
    permissions::{PowerLevel, POWER_LEVEL_MAX, POWER_LEVEL_MIN},
    MemberRole, Room,
};
use crate::{components::PillSource, prelude::*, session::model::User};

#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum, glib::Variant)]
#[variant_enum(repr)]
#[repr(u32)]
#[enum_type(name = "Membership")]
pub enum Membership {
    #[default]
    Leave = 0,
    Join = 1,
    Invite = 2,
    Ban = 3,
    Knock = 4,
    Custom = 5,
}

impl From<&MembershipState> for Membership {
    fn from(state: &MembershipState) -> Self {
        match state {
            MembershipState::Leave => Membership::Leave,
            MembershipState::Join => Membership::Join,
            MembershipState::Invite => Membership::Invite,
            MembershipState::Ban => Membership::Ban,
            MembershipState::Knock => Membership::Knock,
            _ => Membership::Custom,
        }
    }
}

impl From<MembershipState> for Membership {
    fn from(state: MembershipState) -> Self {
        Membership::from(&state)
    }
}

mod imp {
    use std::cell::{Cell, OnceCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Member)]
    pub struct Member {
        /// The room of the member.
        #[property(get, construct_only)]
        pub room: OnceCell<Room>,
        /// The power level of the member.
        #[property(get, minimum = POWER_LEVEL_MIN, maximum = POWER_LEVEL_MAX)]
        pub power_level: Cell<PowerLevel>,
        /// This member's membership state.
        #[property(get, builder(Membership::default()))]
        pub membership: Cell<Membership>,
        /// The timestamp of the latest activity of this member.
        #[property(get, set = Self::set_latest_activity, explicit_notify)]
        pub latest_activity: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Member {
        const NAME: &'static str = "Member";
        type Type = super::Member;
        type ParentType = User;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Member {}

    impl PillSourceImpl for Member {
        fn identifier(&self) -> String {
            self.obj().upcast_ref::<User>().user_id_string()
        }
    }

    impl Member {
        /// Set the timestamp of the latest activity of this member.
        fn set_latest_activity(&self, activity: u64) {
            if self.latest_activity.get() >= activity {
                return;
            }

            self.latest_activity.set(activity);
            self.obj().notify_latest_activity();
        }
    }
}

glib::wrapper! {
    /// A User in the context of a given room.
    pub struct Member(ObjectSubclass<imp::Member>) @extends PillSource, User;
}

impl Member {
    pub fn new(room: &Room, user_id: OwnedUserId) -> Self {
        let session = room.session();
        let obj = glib::Object::builder::<Self>()
            .property("session", &session)
            .property("room", room)
            .build();

        obj.upcast_ref::<User>().imp().set_user_id(user_id);
        obj
    }

    /// Set the power level of the member.
    pub(super) fn set_power_level(&self, power_level: PowerLevel) {
        if self.power_level() == power_level {
            return;
        }
        self.imp().power_level.replace(power_level);
        self.notify_power_level();
    }

    pub fn role(&self) -> MemberRole {
        self.power_level().into()
    }

    pub fn is_admin(&self) -> bool {
        self.role().is_admin()
    }

    pub fn is_mod(&self) -> bool {
        self.role().is_mod()
    }

    pub fn is_peasant(&self) -> bool {
        self.role().is_peasant()
    }

    /// Set this member's membership state.
    fn set_membership(&self, membership: Membership) {
        if self.membership() == membership {
            return;
        }
        let imp = self.imp();
        imp.membership.replace(membership);
        self.notify_membership();
    }

    /// Update the user based on the room member.
    pub fn update_from_room_member(&self, member: &RoomMember) {
        if member.user_id() != self.user_id() {
            error!("Tried Member update from RoomMember with wrong user ID.");
            return;
        };

        self.set_name(member.display_name().map(String::from));
        self.avatar_data()
            .image()
            .unwrap()
            .set_uri(member.avatar_url().map(ToString::to_string));
        self.set_power_level(member.power_level());
        self.set_membership(member.membership().into());
    }

    /// Update the user based on the room member state event
    pub fn update_from_member_event(&self, event: &impl MemberEvent) {
        if event.state_key() != self.user_id() {
            error!("Tried Member update from MemberEvent with wrong user ID.");
            return;
        };

        self.set_name(event.display_name());
        self.avatar_data()
            .image()
            .unwrap()
            .set_uri(event.avatar_url().map(String::from));
        self.set_membership((&event.content().membership).into());

        if self.is_own_user() {
            self.session().update_user_profile();
        }
    }
}

pub trait MemberEvent {
    fn sender(&self) -> &OwnedUserId;
    fn content(&self) -> &RoomMemberEventContent;
    fn state_key(&self) -> &OwnedUserId;

    fn avatar_url(&self) -> Option<OwnedMxcUri> {
        self.content().avatar_url.clone()
    }

    fn display_name(&self) -> Option<String> {
        match &self.content().displayname {
            Some(display_name) => Some(display_name.clone()),
            None => self
                .content()
                .third_party_invite
                .as_ref()
                .map(|i| i.display_name.clone()),
        }
    }
}

impl MemberEvent for OriginalSyncStateEvent<RoomMemberEventContent> {
    fn sender(&self) -> &OwnedUserId {
        &self.sender
    }
    fn content(&self) -> &RoomMemberEventContent {
        &self.content
    }
    fn state_key(&self) -> &OwnedUserId {
        &self.state_key
    }
}
impl MemberEvent for StrippedStateEvent<RoomMemberEventContent> {
    fn sender(&self) -> &OwnedUserId {
        &self.sender
    }
    fn content(&self) -> &RoomMemberEventContent {
        &self.content
    }
    fn state_key(&self) -> &OwnedUserId {
        &self.state_key
    }
}

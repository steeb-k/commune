use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::room::RoomMember;
use ruma::{events::room::member::MembershipState, OwnedEventId, OwnedUserId};
use tracing::{debug, error};

use super::{
    permissions::{PowerLevel, POWER_LEVEL_MAX, POWER_LEVEL_MIN},
    MemberRole, Room,
};
use crate::{components::PillSource, prelude::*, session::model::User, spawn, spawn_tokio};

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

    /// Update this member with the data from the given SDK's member.
    pub fn update_from_room_member(&self, member: &RoomMember) {
        if member.user_id() != self.user_id() {
            error!("Tried Member update from RoomMember with wrong user ID.");
            return;
        };

        self.set_name(member.display_name().map(ToOwned::to_owned));
        self.set_is_name_ambiguous(member.name_ambiguous());
        self.avatar_data()
            .image()
            .unwrap()
            .set_uri(member.avatar_url().map(ToString::to_string));
        self.set_power_level(member.power_level());
        self.set_membership(member.membership().into());
    }

    /// Update this member with the SDK's data.
    pub fn update(&self) {
        spawn!(clone!(@weak self as obj => async move {
            obj.update_inner().await;
        }));
    }

    async fn update_inner(&self) {
        let room = self.room();

        let matrix_room = room.matrix_room().clone();
        let user_id = self.user_id().clone();
        let handle = spawn_tokio!(async move { matrix_room.get_member_no_sync(&user_id).await });

        match handle.await.unwrap() {
            Ok(Some(member)) => self.update_from_room_member(&member),
            Ok(None) => {
                debug!("Room member {} not found", self.user_id());
            }
            Err(error) => {
                error!("Failed to load room member {}: {error}", self.user_id());
            }
        }
    }

    /// The IDs of the events sent by this member that can be redacted.
    pub fn redactable_events(&self) -> Vec<OwnedEventId> {
        self.room().timeline().redactable_events_for(self.user_id())
    }
}

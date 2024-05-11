use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::room::RoomMember;
use ruma::{
    events::room::{
        member::MembershipState,
        power_levels::{NotificationPowerLevelType, PowerLevelAction},
    },
    OwnedEventId, OwnedUserId,
};
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
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Member)]
    pub struct Member {
        /// The room of the member.
        #[property(get, set = Self::set_room, construct_only)]
        pub room: OnceCell<Room>,
        /// The power level of the member.
        #[property(get, minimum = POWER_LEVEL_MIN, maximum = POWER_LEVEL_MAX)]
        pub power_level: Cell<PowerLevel>,
        /// The role of the member.
        #[property(get, builder(MemberRole::default()))]
        pub role: Cell<MemberRole>,
        /// This member's membership state.
        #[property(get, builder(Membership::default()))]
        pub membership: Cell<Membership>,
        /// The timestamp of the latest activity of this member.
        #[property(get, set = Self::set_latest_activity, explicit_notify)]
        pub latest_activity: Cell<u64>,
        power_level_handlers: RefCell<Vec<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Member {
        const NAME: &'static str = "Member";
        type Type = super::Member;
        type ParentType = User;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Member {
        fn dispose(&self) {
            if let Some(room) = self.room.get() {
                for handler in self.power_level_handlers.take() {
                    room.permissions().disconnect(handler);
                }
            }
        }
    }

    impl PillSourceImpl for Member {
        fn identifier(&self) -> String {
            self.obj().upcast_ref::<User>().user_id_string()
        }
    }

    impl Member {
        /// Set the room of the member.
        fn set_room(&self, room: Room) {
            let obj = self.obj();

            let default_pl_handler = room.permissions().connect_default_power_level_notify(
                clone!(@weak obj => move |_| {
                    obj.update_role();
                }),
            );
            let mute_pl_handler =
                room.permissions()
                    .connect_mute_power_level_notify(clone!(@weak obj => move |_| {
                        obj.update_role();
                    }));
            self.power_level_handlers
                .replace(vec![default_pl_handler, mute_pl_handler]);

            self.room.set(room).unwrap();
        }

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

        self.imp().power_level.set(power_level);
        self.update_role();
        self.notify_power_level();
    }

    /// Update the role of the member.
    fn update_role(&self) {
        let role = self.room().permissions().role(self.power_level());

        if self.role() == role {
            return;
        }

        self.imp().role.set(role);
        self.notify_role();
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
                error!("Could not load room member {}: {error}", self.user_id());
            }
        }
    }

    /// The IDs of the events sent by this member that can be redacted.
    pub fn redactable_events(&self) -> Vec<OwnedEventId> {
        self.room().timeline().redactable_events_for(self.user_id())
    }

    /// Whether this room member can notify the whole room.
    pub fn can_notify_room(&self) -> bool {
        self.room().permissions().user_is_allowed_to(
            self.user_id(),
            PowerLevelAction::TriggerNotification(NotificationPowerLevelType::Room),
        )
    }
}

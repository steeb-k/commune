use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::room::RoomMember;
use ruma::{
    OwnedEventId, OwnedUserId,
    events::room::{
        member::MembershipState,
        power_levels::{NotificationPowerLevelType, PowerLevelAction},
    },
};
use tracing::{debug, error};

use super::{
    MemberRole, Room,
    permissions::{POWER_LEVEL_MAX, POWER_LEVEL_MIN, PowerLevel},
};
use crate::{components::PillSource, prelude::*, session::model::User, spawn, spawn_tokio};

/// The possible states of membership of a user in a room.
#[derive(Debug, Default, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "Membership")]
pub enum Membership {
    /// The user left the room, or was never in the room.
    #[default]
    Leave,
    /// The user is currently in the room.
    Join,
    /// The user was invited to the room.
    Invite,
    /// The user was baned from the room.
    Ban,
    /// The user knocked on the room.
    Knock,
    /// The user is in an unsupported membership state.
    Unsupported,
}

impl From<&MembershipState> for Membership {
    fn from(state: &MembershipState) -> Self {
        match state {
            MembershipState::Leave => Membership::Leave,
            MembershipState::Join => Membership::Join,
            MembershipState::Invite => Membership::Invite,
            MembershipState::Ban => Membership::Ban,
            MembershipState::Knock => Membership::Knock,
            _ => Membership::Unsupported,
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
        room: OnceCell<Room>,
        /// The power level of the member.
        #[property(get, minimum = POWER_LEVEL_MIN, maximum = POWER_LEVEL_MAX)]
        power_level: Cell<PowerLevel>,
        /// The role of the member.
        #[property(get, builder(MemberRole::default()))]
        role: Cell<MemberRole>,
        /// This membership state of the member.
        #[property(get, builder(Membership::default()))]
        membership: Cell<Membership>,
        /// The timestamp of the latest activity of this member.
        #[property(get, set = Self::set_latest_activity, explicit_notify)]
        latest_activity: Cell<u64>,
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
            let room = self.room.get_or_init(|| room);

            let default_pl_handler = room
                .permissions()
                .connect_default_power_level_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_role();
                    }
                ));
            let mute_pl_handler = room.permissions().connect_mute_power_level_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_role();
                }
            ));
            self.power_level_handlers
                .replace(vec![default_pl_handler, mute_pl_handler]);
        }

        /// Set the power level of the member.
        pub(super) fn set_power_level(&self, power_level: PowerLevel) {
            if self.power_level.get() == power_level {
                return;
            }

            self.power_level.set(power_level);
            self.update_role();
            self.obj().notify_power_level();
        }

        /// Update the role of the member.
        fn update_role(&self) {
            let role = self
                .room
                .get()
                .expect("room is initialized")
                .permissions()
                .role(self.power_level.get());

            if self.role.get() == role {
                return;
            }

            self.role.set(role);
            self.obj().notify_role();
        }

        /// Set this membership state of the member.
        pub(super) fn set_membership(&self, membership: Membership) {
            if self.membership.get() == membership {
                return;
            }

            self.membership.replace(membership);
            self.obj().notify_membership();
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
    /// A `User` in the context of a given room.
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
        self.imp().set_power_level(power_level);
    }

    /// Update this member with the data from the given SDK's member.
    pub(crate) fn update_from_room_member(&self, member: &RoomMember) {
        if member.user_id() != self.user_id() {
            error!("Tried Member update from RoomMember with wrong user ID.");
            return;
        }

        self.set_name(member.display_name().map(ToOwned::to_owned));
        self.set_is_name_ambiguous(member.name_ambiguous());
        self.avatar_data()
            .image()
            .expect("image is set")
            .set_uri_and_info(member.avatar_url().map(ToOwned::to_owned), None);
        self.set_power_level(member.power_level());
        self.imp().set_membership(member.membership().into());
    }

    /// Update this member with data from the SDK.
    pub(crate) fn update(&self) {
        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                obj.update_inner().await;
            }
        ));
    }

    async fn update_inner(&self) {
        let room = self.room();

        let matrix_room = room.matrix_room().clone();
        let user_id = self.user_id().clone();
        let handle = spawn_tokio!(async move { matrix_room.get_member_no_sync(&user_id).await });

        match handle.await.expect("task was not aborted") {
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
    pub(crate) fn redactable_events(&self) -> Vec<OwnedEventId> {
        self.room()
            .live_timeline()
            .redactable_events_for(self.user_id())
    }

    /// Whether this room member can notify the whole room.
    pub(crate) fn can_notify_room(&self) -> bool {
        self.room().permissions().user_is_allowed_to(
            self.user_id(),
            PowerLevelAction::TriggerNotification(NotificationPowerLevelType::Room),
        )
    }

    /// The string to use to search for this member.
    pub(crate) fn search_string(&self) -> String {
        format!(
            "{} {} {} {}",
            self.display_name(),
            self.user_id(),
            self.role(),
            self.power_level(),
        )
    }
}

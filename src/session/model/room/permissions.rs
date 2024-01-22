use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::{
    deserialized_responses::SyncOrStrippedState, event_handler::EventHandlerDropGuard,
};
use ruma::{
    events::{
        room::power_levels::{PowerLevelAction, RoomPowerLevels, RoomPowerLevelsEventContent},
        MessageLikeEventType, StateEventType, SyncStateEvent,
    },
    UserId,
};
use tracing::error;

use super::{Member, Membership, Room};
use crate::{prelude::*, spawn, spawn_tokio};

/// Power level of a user.
///
/// Is usually in the range (0..=100), but can be any JS integer.
pub type PowerLevel = i64;
// Same value as MAX_SAFE_INT from js_int.
pub const POWER_LEVEL_MAX: i64 = 0x001F_FFFF_FFFF_FFFF;
pub const POWER_LEVEL_MIN: i64 = -POWER_LEVEL_MAX;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use glib::subclass::Signal;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::Permissions)]
    pub struct Permissions {
        /// The room where these permissions apply.
        #[property(get)]
        pub room: glib::WeakRef<Room>,
        /// The source of the power levels information.
        pub power_levels: RefCell<RoomPowerLevels>,
        power_levels_drop_guard: OnceCell<EventHandlerDropGuard>,
        /// Whether our own member is joined.
        pub is_joined: Cell<bool>,
        /// Whether our own member can change the room's avatar.
        #[property(get)]
        pub can_change_avatar: Cell<bool>,
        /// Whether our own member can change the room's name.
        #[property(get)]
        pub can_change_name: Cell<bool>,
        /// Whether our own member can change the room's topic.
        #[property(get)]
        pub can_change_topic: Cell<bool>,
        /// Whether our own member can invite another user.
        #[property(get)]
        pub can_invite: Cell<bool>,
        /// Whether our own member can send a message.
        #[property(get)]
        pub can_send_message: Cell<bool>,
        /// Whether our own member can send a reaction.
        #[property(get)]
        pub can_send_reaction: Cell<bool>,
        /// Whether our own member can redact their own event.
        #[property(get)]
        pub can_redact_own: Cell<bool>,
        /// Whether our own member can redact the event of another user.
        #[property(get)]
        pub can_redact_other: Cell<bool>,
    }

    impl Default for Permissions {
        fn default() -> Self {
            Self {
                room: Default::default(),
                power_levels: RefCell::new(RoomPowerLevelsEventContent::default().into()),
                power_levels_drop_guard: Default::default(),
                is_joined: Default::default(),
                can_change_avatar: Default::default(),
                can_change_name: Default::default(),
                can_change_topic: Default::default(),
                can_invite: Default::default(),
                can_send_message: Default::default(),
                can_send_reaction: Default::default(),
                can_redact_own: Default::default(),
                can_redact_other: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Permissions {
        const NAME: &'static str = "RoomPermissions";
        type Type = super::Permissions;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Permissions {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("changed").build()]);
            SIGNALS.as_ref()
        }
    }

    impl Permissions {
        /// Initialize the room.
        pub(super) fn init_own_member(&self, own_member: Member) {
            own_member.connect_membership_notify(clone!(@weak self as imp => move |_| {
                imp.update_is_joined();
            }));

            self.update_is_joined();
        }

        /// The room member for our own user.
        pub(super) fn own_member(&self) -> Option<Member> {
            self.room.upgrade().map(|r| r.own_member())
        }

        /// Initialize the power levels from the store.
        pub(super) async fn init_power_levels(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            let matrix_room = room.matrix_room();

            let matrix_room_clone = matrix_room.clone();
            let handle = spawn_tokio!(async move {
                let state_event = match matrix_room_clone
                    .get_state_event_static::<RoomPowerLevelsEventContent>()
                    .await
                {
                    Ok(state_event) => state_event,
                    Err(error) => {
                        error!("Initial load of room power levels failed: {error}");
                        return None;
                    }
                };

                state_event
                    .and_then(|r| r.deserialize().ok())
                    .and_then(|ev| match ev {
                        SyncOrStrippedState::Sync(e) => Some(e),
                        // The power levels are usually not in the stripped state.
                        _ => None,
                    })
            });

            if let Some(event) = handle.await.unwrap() {
                self.update_power_levels(&event);
            }

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let handle = matrix_room.add_event_handler(
                move |event: SyncStateEvent<RoomPowerLevelsEventContent>| {
                    let obj_weak = obj_weak.clone();
                    async move {
                        let ctx = glib::MainContext::default();
                        ctx.spawn(async move {
                            spawn!(async move {
                                if let Some(obj) = obj_weak.upgrade() {
                                    obj.imp().update_power_levels(&event);
                                }
                            });
                        });
                    }
                },
            );

            let drop_guard = matrix_room.client().event_handler_drop_guard(handle);
            self.power_levels_drop_guard.set(drop_guard).unwrap();
        }

        /// Update whether our own member is joined
        fn update_is_joined(&self) {
            let Some(own_member) = self.own_member() else {
                return;
            };

            let is_joined = own_member.membership() == Membership::Join;

            if self.is_joined.get() == is_joined {
                return;
            }

            self.is_joined.set(is_joined);
            self.permissions_changed();
        }

        /// Update the power levels with the given event.
        fn update_power_levels(&self, event: &SyncStateEvent<RoomPowerLevelsEventContent>) {
            let power_levels = event.power_levels();
            self.power_levels.replace(power_levels.clone());
            self.permissions_changed();

            if let Some(room) = self.room.upgrade() {
                if let Some(members) = room.members() {
                    members.update_power_levels(&power_levels);
                } else {
                    let own_member = room.own_member();
                    let own_user_id = own_member.user_id();
                    own_member.set_power_level(power_levels.for_user(own_user_id).into());
                }
            }
        }

        /// Trigger updates when the permissions changed.
        fn permissions_changed(&self) {
            self.update_can_change_avatar();
            self.update_can_change_name();
            self.update_can_change_topic();
            self.update_can_invite();
            self.update_can_send_message();
            self.update_can_send_reaction();
            self.update_can_redact_own();
            self.update_can_redact_other();
            self.obj().emit_by_name::<()>("changed", &[]);
        }

        /// Returns whether our own member is allowed to do the
        /// given action.
        pub(super) fn is_allowed_to(&self, room_action: PowerLevelAction) -> bool {
            if !self.is_joined.get() {
                // We cannot do anything if the member is not joined.
                return false;
            }

            let Some(own_member) = self.own_member() else {
                return false;
            };

            self.power_levels
                .borrow()
                .user_can_do(own_member.user_id(), room_action)
        }

        /// Update whether our own member can change the room's avatar.
        fn update_can_change_avatar(&self) {
            let can_change_avatar =
                self.is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomAvatar));

            if self.can_change_avatar.get() == can_change_avatar {
                return;
            };

            self.can_change_avatar.set(can_change_avatar);
            self.obj().notify_can_change_avatar();
        }

        /// Update whether our own member can change the room's name.
        fn update_can_change_name(&self) {
            let can_change_name =
                self.is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomName));

            if self.can_change_name.get() == can_change_name {
                return;
            };

            self.can_change_name.set(can_change_name);
            self.obj().notify_can_change_name();
        }

        /// Update whether our own member can change the room's topic.
        fn update_can_change_topic(&self) {
            let can_change_topic =
                self.is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomTopic));

            if self.can_change_topic.get() == can_change_topic {
                return;
            };

            self.can_change_topic.set(can_change_topic);
            self.obj().notify_can_change_topic();
        }

        /// Update whether our own member can invite another user in the room.
        fn update_can_invite(&self) {
            let can_invite = self.is_allowed_to(PowerLevelAction::Invite);

            if self.can_invite.get() == can_invite {
                return;
            };

            self.can_invite.set(can_invite);
            self.obj().notify_can_invite();
        }

        /// Update whether our own member can send a message in the room.
        fn update_can_send_message(&self) {
            let can_send_message = self.is_allowed_to(PowerLevelAction::SendMessage(
                MessageLikeEventType::RoomMessage,
            ));

            if self.can_send_message.get() == can_send_message {
                return;
            };

            self.can_send_message.set(can_send_message);
            self.obj().notify_can_send_message();
        }

        /// Update whether our own member can send a reaction.
        fn update_can_send_reaction(&self) {
            let can_send_reaction = self.is_allowed_to(PowerLevelAction::SendMessage(
                MessageLikeEventType::Reaction,
            ));

            if self.can_send_reaction.get() == can_send_reaction {
                return;
            };

            self.can_send_reaction.set(can_send_reaction);
            self.obj().notify_can_send_reaction();
        }

        /// Update whether our own member can redact their own event.
        fn update_can_redact_own(&self) {
            let can_redact_own = self.is_allowed_to(PowerLevelAction::RedactOwn);

            if self.can_redact_own.get() == can_redact_own {
                return;
            };

            self.can_redact_own.set(can_redact_own);
            self.obj().notify_can_redact_own();
        }

        /// Whether our own member can redact the event of another user.
        fn update_can_redact_other(&self) {
            let can_redact_other = self.is_allowed_to(PowerLevelAction::RedactOther);

            if self.can_redact_other.get() == can_redact_other {
                return;
            };

            self.can_redact_other.set(can_redact_other);
            self.obj().notify_can_redact_other();
        }
    }
}

glib::wrapper! {
    /// The permissions of our own user in a room.
    pub struct Permissions(ObjectSubclass<imp::Permissions>);
}

impl Permissions {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set our own member.
    pub(super) async fn init(&self, room: &Room) {
        let imp = self.imp();

        imp.room.set(Some(room));
        imp.init_own_member(room.own_member());
        imp.init_power_levels().await;
    }

    /// Whether our own user can do the given action on the user with the given
    /// ID.
    pub fn can_do_to_user(&self, user_id: &UserId, action: PowerLevelUserAction) -> bool {
        let imp = self.imp();
        let Some(own_member) = imp.own_member() else {
            return false;
        };
        let own_user_id = own_member.user_id();

        let power_levels = imp.power_levels.borrow();

        if own_user_id == user_id {
            // The only action we can do for our own user is change the power level.
            return action == PowerLevelUserAction::ChangePowerLevel
                && power_levels.user_can_send_state(own_user_id, StateEventType::RoomPowerLevels);
        }

        let own_pl = power_levels.for_user(own_user_id);
        let other_pl = power_levels.for_user(user_id);

        // TODO: Use Ruma's type and RoomPowerLevels methods when we use a recent enough
        // version.
        match action {
            PowerLevelUserAction::Ban => {
                power_levels.user_can_ban(own_user_id) && own_pl > other_pl
            }
            PowerLevelUserAction::Unban => {
                power_levels.user_can_ban(own_user_id)
                    && power_levels.user_can_kick(own_user_id)
                    && own_pl > other_pl
            }
            PowerLevelUserAction::Invite => power_levels.user_can_invite(own_user_id),
            PowerLevelUserAction::Kick => {
                power_levels.user_can_kick(own_user_id) && own_pl > other_pl
            }
            PowerLevelUserAction::ChangePowerLevel => {
                power_levels.user_can_send_state(own_user_id, StateEventType::RoomPowerLevels)
                    && own_pl > other_pl
            }
        }
    }

    /// Connect to the signal emitted when the permissions changed.
    pub fn connect_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

impl Default for Permissions {
    fn default() -> Self {
        Self::new()
    }
}

/// The actions to other users that can be limited by power levels.
// TODO: Use Ruma's type and RoomPowerLevels methods when we use a recent enough version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerLevelUserAction {
    /// Ban a user.
    Ban,

    /// Unban a user.
    Unban,

    /// Invite a user.
    Invite,

    /// Kick a user.
    Kick,

    /// Change a user's power level.
    ChangePowerLevel,
}

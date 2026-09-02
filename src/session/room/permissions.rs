use std::fmt;

use commune_core::session::{Permissions as CorePermissions, PermissionsState};
use gettextrs::{gettext, pgettext};
use gtk::{glib, glib::closure_local, prelude::*, subclass::prelude::*};
use ruma::{
    Int, OwnedUserId, UserId,
    events::room::power_levels::{
        PowerLevelAction, PowerLevelUserAction, RoomPowerLevels, UserPowerLevel,
    },
    int,
};
use tokio::task::AbortHandle;
use tracing::error;

use super::Room;
use crate::{core_bridge::ObjectWatcher, spawn_tokio};

/// The maximum power level that can be set, according to the Matrix
/// specification.
///
/// This is the same value as `MAX_SAFE_INT` from the `js_int` crate.
pub const POWER_LEVEL_MAX: i64 = 0x001F_FFFF_FFFF_FFFF;
/// The minimum power level to have the role of Administrator, according to the
/// Matrix specification.
pub const POWER_LEVEL_ADMIN: i64 = 100;
/// The minimum power level to have the role of Moderator, according to the
/// Matrix specification.
pub const POWER_LEVEL_MOD: i64 = 50;

/// Role of a room member, like admin or moderator.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "MemberRole")]
pub enum MemberRole {
    /// A room member with the default power level.
    #[default]
    Default,
    /// A room member with a non-default power level, but lower than and a
    /// moderator.
    Custom,
    /// A moderator.
    Moderator,
    /// An administrator.
    Administrator,
    /// A creator.
    Creator,
    /// A room member that cannot send messages.
    Muted,
}

impl From<commune_core::session::MemberRole> for MemberRole {
    fn from(role: commune_core::session::MemberRole) -> Self {
        use commune_core::session::MemberRole as Core;

        match role {
            Core::Default => Self::Default,
            Core::Custom => Self::Custom,
            Core::Moderator => Self::Moderator,
            Core::Administrator => Self::Administrator,
            Core::Creator => Self::Creator,
            Core::Muted => Self::Muted,
        }
    }
}

impl fmt::Display for MemberRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            // Translators: As in 'Default power level', meaning permissions.
            Self::Default => write!(f, "{}", gettext("Default")),
            // Translators: As in, 'Custom power level', meaning permissions.
            Self::Custom => write!(f, "{}", pgettext("power level", "Custom")),
            Self::Moderator => write!(f, "{}", gettext("Moderator")),
            Self::Administrator => write!(f, "{}", gettext("Admin")),
            Self::Creator => write!(f, "{}", gettext("Creator")),
            // Translators: As in 'Muted room member', a member that cannot send messages.
            Self::Muted => write!(f, "{}", gettext("Muted")),
        }
    }
}

/// Set a mirrored property and notify it if it changed.
macro_rules! mirror {
    ($imp:ident, $obj:ident, $field:ident, $value:expr, $notify:ident) => {
        if $imp.$field.get() != $value {
            $imp.$field.set($value);
            $obj.$notify();
        }
    };
}

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::Permissions)]
    pub struct Permissions {
        /// The room where these permissions apply.
        #[property(get)]
        pub(super) room: glib::WeakRef<Room>,
        /// The core's permissions, which this presents.
        pub(super) core: OnceCell<CorePermissions>,
        /// The task following the core's state.
        watch_handle: RefCell<Option<AbortHandle>>,
        /// Whether our own member is joined.
        #[property(get)]
        is_joined: Cell<bool>,
        /// The power level of our own member.
        pub(super) own_power_level: Cell<UserPowerLevel>,
        /// The default power level for members.
        #[property(get)]
        default_power_level: Cell<i64>,
        /// The power level to mute members.
        #[property(get)]
        mute_power_level: Cell<i64>,
        /// Whether our own member can change the room's avatar.
        #[property(get)]
        can_change_avatar: Cell<bool>,
        /// Whether our own member can change the room's name.
        #[property(get)]
        can_change_name: Cell<bool>,
        /// Whether our own member can change the room's topic.
        #[property(get)]
        can_change_topic: Cell<bool>,
        /// Whether our own member can invite another user.
        #[property(get)]
        can_invite: Cell<bool>,
        /// Whether our own member can send a message.
        #[property(get)]
        can_send_message: Cell<bool>,
        /// Whether our own member can send a sticker.
        #[property(get)]
        can_send_sticker: Cell<bool>,
        /// Whether our own member can send a reaction.
        #[property(get)]
        can_send_reaction: Cell<bool>,
        /// Whether our own member can change the image packs of the room.
        #[property(get)]
        can_change_image_packs: Cell<bool>,
        /// Whether our own member can pin and unpin the events of the room.
        #[property(get)]
        can_pin_events: Cell<bool>,
        /// Whether our own member can redact their own event.
        #[property(get)]
        can_redact_own: Cell<bool>,
        /// Whether our own member can redact the event of another user.
        #[property(get)]
        can_redact_other: Cell<bool>,
        /// Whether our own member can notify the whole room.
        #[property(get)]
        can_notify_room: Cell<bool>,
    }

    impl Default for Permissions {
        fn default() -> Self {
            Self {
                room: Default::default(),
                core: Default::default(),
                watch_handle: Default::default(),
                is_joined: Default::default(),
                own_power_level: Cell::new(UserPowerLevel::Int(int!(0))),
                default_power_level: Default::default(),
                mute_power_level: Default::default(),
                can_change_avatar: Default::default(),
                can_change_name: Default::default(),
                can_change_topic: Default::default(),
                can_invite: Default::default(),
                can_send_message: Default::default(),
                can_send_sticker: Default::default(),
                can_send_reaction: Default::default(),
                can_change_image_packs: Default::default(),
                can_pin_events: Default::default(),
                can_redact_own: Default::default(),
                can_redact_other: Default::default(),
                can_notify_room: Default::default(),
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
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("changed").build(),
                    Signal::builder("own-power-level-changed").build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl Permissions {
        /// Follow the core's state.
        pub(super) fn watch_core(&self, core: &CorePermissions) {
            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe(), |obj: &super::Permissions, state| {
                    obj.imp().set_state(&state);
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.set_state(&core.state());
        }

        /// Mirror the core's state into the properties.
        #[allow(
            clippy::too_many_lines,
            reason = "one mirror per property, fourteen properties"
        )]
        fn set_state(&self, state: &PermissionsState) {
            let obj = self.obj();

            mirror!(self, obj, is_joined, state.is_joined, notify_is_joined);
            mirror!(
                self,
                obj,
                default_power_level,
                state.default_power_level,
                notify_default_power_level
            );
            mirror!(
                self,
                obj,
                mute_power_level,
                state.mute_power_level,
                notify_mute_power_level
            );
            mirror!(
                self,
                obj,
                can_change_avatar,
                state.can_change_avatar,
                notify_can_change_avatar
            );
            mirror!(
                self,
                obj,
                can_change_name,
                state.can_change_name,
                notify_can_change_name
            );
            mirror!(
                self,
                obj,
                can_change_topic,
                state.can_change_topic,
                notify_can_change_topic
            );
            mirror!(self, obj, can_invite, state.can_invite, notify_can_invite);
            mirror!(
                self,
                obj,
                can_send_message,
                state.can_send_message,
                notify_can_send_message
            );
            mirror!(
                self,
                obj,
                can_send_sticker,
                state.can_send_sticker,
                notify_can_send_sticker
            );
            mirror!(
                self,
                obj,
                can_send_reaction,
                state.can_send_reaction,
                notify_can_send_reaction
            );
            mirror!(
                self,
                obj,
                can_change_image_packs,
                state.can_change_image_packs,
                notify_can_change_image_packs
            );
            mirror!(
                self,
                obj,
                can_pin_events,
                state.can_pin_events,
                notify_can_pin_events
            );
            mirror!(
                self,
                obj,
                can_redact_own,
                state.can_redact_own,
                notify_can_redact_own
            );
            mirror!(
                self,
                obj,
                can_redact_other,
                state.can_redact_other,
                notify_can_redact_other
            );
            mirror!(
                self,
                obj,
                can_notify_room,
                state.can_notify_room,
                notify_can_notify_room
            );

            let own_power_level = state
                .own_power_level
                .unwrap_or(UserPowerLevel::Int(int!(0)));
            if self.own_power_level.get() != own_power_level {
                self.own_power_level.set(own_power_level);
                obj.emit_by_name::<()>("own-power-level-changed", &[]);

                // Our own member is here before any member list is; a listed
                // member's power level arrives with the core's snapshot too.
                if let Some(room) = self.room.upgrade() {
                    room.own_member().set_power_level(own_power_level);
                }
            }

            obj.emit_by_name::<()>("changed", &[]);
        }
    }
}

glib::wrapper! {
    /// The permissions of our own user in a room.
    ///
    /// The permissions themselves are the core's; this presents them.
    pub struct Permissions(ObjectSubclass<imp::Permissions>);
}

impl Permissions {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the room, load the core's permissions and follow them.
    pub(super) async fn init(&self, room: &Room) {
        let imp = self.imp();

        imp.room.set(Some(room));

        let core = room.core().permissions().clone();

        // The power levels come from the store, which wants the runtime.
        let load = core.clone();
        spawn_tokio!(async move { load.ensure_loaded().await })
            .await
            .expect("task was not aborted");

        imp.watch_core(&core);
        imp.core
            .set(core)
            .expect("core permissions are uninitialized");
    }

    /// The core's permissions, once the room is known.
    fn core(&self) -> Option<&CorePermissions> {
        self.imp().core.get()
    }

    /// The source of the power levels information.
    pub(crate) fn power_levels(&self) -> RoomPowerLevels {
        self.core().map_or_else(
            || {
                RoomPowerLevels::new(
                    ruma::events::room::power_levels::RoomPowerLevelsSource::None,
                    &ruma::room_version_rules::AuthorizationRules::V1,
                    None,
                )
            },
            CorePermissions::power_levels,
        )
    }

    /// The power level of our own member.
    pub(crate) fn own_power_level(&self) -> UserPowerLevel {
        self.imp().own_power_level.get()
    }

    /// The power level for the user with the given ID.
    pub(crate) fn user_power_level(&self, user_id: &UserId) -> UserPowerLevel {
        self.core().map_or(UserPowerLevel::Int(int!(0)), |core| {
            core.user_power_level(user_id)
        })
    }

    /// The current [`MemberRole`] for the given power level.
    pub(crate) fn role(&self, power_level: UserPowerLevel) -> MemberRole {
        self.core()
            .map_or(MemberRole::Default, |core| core.role(power_level).into())
    }

    /// Whether our own member is allowed to do the given action.
    pub(crate) fn is_allowed_to(&self, room_action: PowerLevelAction) -> bool {
        self.core()
            .is_some_and(|core| core.is_allowed_to(room_action))
    }

    /// Whether our own user can do the given action on the user with the given
    /// ID.
    pub(crate) fn can_do_to_user(&self, user_id: &UserId, action: PowerLevelUserAction) -> bool {
        self.core()
            .is_some_and(|core| core.can_do_to_user(user_id, action))
    }

    /// Whether our user can set the given power level for another user.
    pub(crate) fn can_set_user_power_level_to(&self, power_level: i64) -> bool {
        self.core()
            .is_some_and(|core| core.can_set_user_power_level_to(power_level))
    }

    /// Set the power level of the room member with the given user ID.
    pub(crate) async fn set_user_power_level(
        &self,
        user_id: OwnedUserId,
        power_level: Int,
    ) -> Result<(), ()> {
        let Some(core) = self.core().cloned() else {
            return Err(());
        };

        let handle =
            spawn_tokio!(async move { core.set_user_power_level(user_id, power_level).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not set user power level: {error}");
            })
    }

    /// Set the power levels.
    pub(crate) async fn set_power_levels(&self, power_levels: RoomPowerLevels) -> Result<(), ()> {
        let Some(core) = self.core().cloned() else {
            return Err(());
        };

        let handle = spawn_tokio!(async move { core.set_power_levels(power_levels).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not set power levels: {error}");
            })
    }

    /// Whether the user with the given ID is allowed to do the given action.
    pub(crate) fn user_is_allowed_to(
        &self,
        user_id: &UserId,
        room_action: PowerLevelAction,
    ) -> bool {
        self.core()
            .is_some_and(|core| core.user_is_allowed_to(user_id, room_action))
    }

    /// Connect to the signal emitted when the permissions changed.
    pub(crate) fn connect_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }

    /// Connect to the signal emitted when the power level of our own member
    /// changed.
    pub(crate) fn connect_own_power_level_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "own-power-level-changed",
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

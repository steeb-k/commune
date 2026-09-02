//! The permissions of our own user in a room, headless.
//!
//! The application's `Permissions` object: the room's power levels,
//! followed through the `m.room.power_levels` state event, and the
//! fourteen answers the interface asks of them — whether our own member
//! may change the name, invite, send a message, pin, redact, notify the
//! room — kept as one observable state. The permissions subpage's rows are
//! here too, as [`PowerLevelsMatrix`]: the page reads the power levels into
//! its rows and collects the rows back into power levels by rules that are
//! not the widget's, and those rules are what an embedder needs.

use std::sync::{Arc, Mutex};

use eyeball::{SharedObservable, Subscriber};
use matrix_sdk::{RoomState, event_handler::EventHandlerDropGuard, room::Room as MatrixRoom};
use ruma::{
    Int, OwnedUserId, UserId,
    events::{
        MessageLikeEventType, StateEventType, StaticEventContent, SyncStateEvent,
        TimelineEventType,
        room::power_levels::{
            NotificationPowerLevelType, PowerLevelAction, PowerLevelUserAction, RoomPowerLevels,
            RoomPowerLevelsEventContent, RoomPowerLevelsSource, UserPowerLevel,
        },
    },
    room_version_rules::AuthorizationRules,
};
use tracing::error;

use super::member::{MemberRole, role};
use crate::{UserFacingError, events::image_packs::RoomImagePackEventContent, spawn_tokio};

/// The maximum power level that can be set, according to the Matrix
/// specification.
///
/// This is the same value as `MAX_SAFE_INT` from the `js_int` crate.
pub const POWER_LEVEL_MAX: i64 = 0x001F_FFFF_FFFF_FFFF;

/// An error encountered while changing the power levels of a room.
#[derive(Debug, thiserror::Error)]
pub enum PermissionsError {
    /// The power levels could not be turned into an event: a level is out
    /// of the range an event can carry.
    #[error("the power levels cannot be sent: {0}")]
    Invalid(String),
    /// The request failed.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` here expensive.
    #[error(transparent)]
    Server(Box<matrix_sdk::Error>),
}

impl UserFacingError for PermissionsError {
    fn to_user_facing(&self) -> String {
        "Could not save permissions".to_owned()
    }
}

/// What our own member may do in the room, as the application's object
/// presents it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the application's fourteen properties, one per question the interface asks"
)]
pub struct PermissionsState {
    /// Whether our own member is joined.
    pub is_joined: bool,
    /// The power level of our own member.
    pub own_power_level: Option<UserPowerLevel>,
    /// The default power level for members.
    pub default_power_level: i64,
    /// The power level to mute members.
    pub mute_power_level: i64,
    /// Whether our own member can change the room's avatar.
    pub can_change_avatar: bool,
    /// Whether our own member can change the room's name.
    pub can_change_name: bool,
    /// Whether our own member can change the room's topic.
    pub can_change_topic: bool,
    /// Whether our own member can invite another user.
    pub can_invite: bool,
    /// Whether our own member can send a message.
    pub can_send_message: bool,
    /// Whether our own member can send a sticker.
    pub can_send_sticker: bool,
    /// Whether our own member can send a reaction.
    pub can_send_reaction: bool,
    /// Whether our own member can change the image packs of the room.
    pub can_change_image_packs: bool,
    /// Whether our own member can pin and unpin the events of the room.
    pub can_pin_events: bool,
    /// Whether our own member can redact their own event.
    pub can_redact_own: bool,
    /// Whether our own member can redact the event of another user.
    pub can_redact_other: bool,
    /// Whether our own member can notify the whole room.
    pub can_notify_room: bool,
}

/// The permissions of our own user in a room.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct Permissions {
    inner: Arc<PermissionsInner>,
}

#[derive(Debug)]
struct PermissionsInner {
    /// The room API of the SDK.
    matrix_room: MatrixRoom,
    /// The source of the power levels information.
    power_levels: Mutex<RoomPowerLevels>,
    /// The current state.
    state: SharedObservable<PermissionsState>,
    /// The guard of the power-levels event handler.
    drop_guard: Mutex<Option<EventHandlerDropGuard>>,
    /// The one-time load.
    loaded: tokio::sync::OnceCell<()>,
}

impl Permissions {
    /// Create the permissions of our own user in the given room.
    pub(crate) fn new(matrix_room: MatrixRoom) -> Self {
        Self {
            inner: Arc::new(PermissionsInner {
                matrix_room,
                power_levels: Mutex::new(RoomPowerLevels::new(
                    RoomPowerLevelsSource::None,
                    &AuthorizationRules::V1,
                    None,
                )),
                state: SharedObservable::new(PermissionsState::default()),
                drop_guard: Mutex::new(None),
                loaded: tokio::sync::OnceCell::new(),
            }),
        }
    }

    /// The current state.
    #[must_use]
    pub fn state(&self) -> PermissionsState {
        self.inner.state.get()
    }

    /// Subscribe to the state.
    pub fn subscribe(&self) -> Subscriber<PermissionsState> {
        self.inner.state.subscribe()
    }

    /// Load the power levels from the store and follow the
    /// `m.room.power_levels` event from then on.
    ///
    /// The application does this when the room is built; the core does it
    /// on first use, and every reader that wants a current answer waits
    /// here first.
    pub async fn ensure_loaded(&self) {
        let inner = &self.inner;
        inner
            .loaded
            .get_or_init(|| async {
                // We will probably not be able to load the power levels if
                // we were never in the room, so skip this. We should get the
                // power levels when we join the room.
                if !matches!(
                    inner.matrix_room.state(),
                    RoomState::Invited | RoomState::Knocked
                ) {
                    inner.update_power_levels().await;
                }

                let weak = Arc::downgrade(inner);
                let handle = inner.matrix_room.add_event_handler(
                    move |_event: SyncStateEvent<RoomPowerLevelsEventContent>| {
                        let weak = weak.clone();
                        async move {
                            if let Some(inner) = weak.upgrade() {
                                inner.update_power_levels().await;
                            }
                        }
                    },
                );
                let drop_guard = inner.matrix_room.client().event_handler_drop_guard(handle);
                *inner.drop_guard.lock().expect("mutex is not poisoned") = Some(drop_guard);
            })
            .await;
    }

    /// Update whether our own member is joined.
    ///
    /// The application watches its own member; the core hears it through
    /// the room info.
    pub(crate) fn update_is_joined(&self) {
        self.inner.permissions_changed();
    }

    /// The source of the power levels information.
    #[must_use]
    pub fn power_levels(&self) -> RoomPowerLevels {
        self.inner
            .power_levels
            .lock()
            .expect("mutex is not poisoned")
            .clone()
    }

    /// The power level of our own member.
    #[must_use]
    pub fn own_power_level(&self) -> UserPowerLevel {
        self.user_power_level(self.inner.matrix_room.own_user_id())
    }

    /// The power level for the user with the given ID.
    #[must_use]
    pub fn user_power_level(&self, user_id: &UserId) -> UserPowerLevel {
        self.inner
            .power_levels
            .lock()
            .expect("mutex is not poisoned")
            .for_user(user_id)
    }

    /// The current [`MemberRole`] for the given power level.
    #[must_use]
    pub fn role(&self, power_level: UserPowerLevel) -> MemberRole {
        role(
            power_level,
            &self
                .inner
                .power_levels
                .lock()
                .expect("mutex is not poisoned"),
        )
    }

    /// Whether our own member is allowed to do the given action.
    #[must_use]
    pub fn is_allowed_to(&self, room_action: PowerLevelAction) -> bool {
        self.inner.is_allowed_to(room_action)
    }

    /// Whether our own user can do the given action on the user with the
    /// given ID.
    #[must_use]
    pub fn can_do_to_user(&self, user_id: &UserId, action: PowerLevelUserAction) -> bool {
        let inner = &self.inner;

        if !inner.is_joined() {
            // We cannot do anything if the member is not joined.
            return false;
        }

        let own_user_id = inner.matrix_room.own_user_id();
        let power_levels = inner.power_levels.lock().expect("mutex is not poisoned");

        if own_user_id == user_id {
            // The only action we can do for our own user is change the
            // power level, if it's not a creator.
            return action == PowerLevelUserAction::ChangePowerLevel
                && power_levels.user_can_change_user_power_level(own_user_id, own_user_id);
        }

        power_levels.user_can_do_to_user(own_user_id, user_id, action)
    }

    /// Whether our user can set the given power level for another user.
    #[must_use]
    pub fn can_set_user_power_level_to(&self, power_level: i64) -> bool {
        self.is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomPowerLevels))
            && self.own_power_level() >= Int::new_saturating(power_level)
    }

    /// Whether the user with the given ID is allowed to do the given action.
    #[must_use]
    pub fn user_is_allowed_to(&self, user_id: &UserId, room_action: PowerLevelAction) -> bool {
        self.inner
            .power_levels
            .lock()
            .expect("mutex is not poisoned")
            .user_can_do(user_id, room_action)
    }

    /// Set the power level of the room member with the given user ID.
    pub async fn set_user_power_level(
        &self,
        user_id: OwnedUserId,
        power_level: Int,
    ) -> Result<(), PermissionsError> {
        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            matrix_room
                .update_power_levels(vec![(&user_id, power_level)])
                .await
        });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|set_error| {
                error!("Could not set user power level: {set_error}");
                PermissionsError::Server(Box::new(set_error))
            })
    }

    /// Set the power levels.
    pub async fn set_power_levels(
        &self,
        power_levels: RoomPowerLevels,
    ) -> Result<(), PermissionsError> {
        let event = RoomPowerLevelsEventContent::try_from(power_levels).map_err(|error| {
            error!("Could not set power levels: {error}");
            PermissionsError::Invalid(error.to_string())
        })?;

        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(event).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|send_error| {
                error!("Could not set power levels: {send_error}");
                PermissionsError::Server(Box::new(send_error))
            })
    }
}

impl PermissionsInner {
    /// Whether our own member is joined.
    fn is_joined(&self) -> bool {
        self.matrix_room.state() == RoomState::Joined
    }

    /// Update the power levels with the data from the SDK's room.
    async fn update_power_levels(&self) {
        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.power_levels().await });

        let power_levels = match handle.await.expect("task was not aborted") {
            Ok(power_levels) => power_levels,
            Err(load_error) => {
                error!("Could not load room power levels: {load_error}");
                return;
            }
        };

        *self.power_levels.lock().expect("mutex is not poisoned") = power_levels;
        self.permissions_changed();
    }

    /// Whether our own member is allowed to do the given action.
    fn is_allowed_to(&self, room_action: PowerLevelAction) -> bool {
        if !self.is_joined() {
            // We cannot do anything if the member is not joined.
            return false;
        }

        self.power_levels
            .lock()
            .expect("mutex is not poisoned")
            .user_can_do(self.matrix_room.own_user_id(), room_action)
    }

    /// Recompute the state when the permissions changed.
    fn permissions_changed(&self) {
        let own_user_id = self.matrix_room.own_user_id();
        let (own_power_level, default_power_level, mute_power_level) = {
            let power_levels = self.power_levels.lock().expect("mutex is not poisoned");
            (
                power_levels.for_user(own_user_id),
                i64::from(power_levels.users_default),
                mute_power_level(&power_levels),
            )
        };

        // The packs that we create use the stable event type. A pack that
        // was created under the unstable one is written back under that
        // name, and a room that gives the two types different power levels
        // would need this to be per pack, which is not worth the trouble:
        // the request fails and the error is reported.
        let state = PermissionsState {
            is_joined: self.is_joined(),
            own_power_level: Some(own_power_level),
            default_power_level,
            mute_power_level,
            can_change_avatar: self
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomAvatar)),
            can_change_name: self
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomName)),
            can_change_topic: self
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomTopic)),
            can_invite: self.is_allowed_to(PowerLevelAction::Invite),
            can_send_message: self.is_allowed_to(PowerLevelAction::SendMessage(
                MessageLikeEventType::RoomMessage,
            )),
            can_send_sticker: self
                .is_allowed_to(PowerLevelAction::SendMessage(MessageLikeEventType::Sticker)),
            can_send_reaction: self.is_allowed_to(PowerLevelAction::SendMessage(
                MessageLikeEventType::Reaction,
            )),
            can_change_image_packs: self.is_allowed_to(PowerLevelAction::SendState(
                StateEventType::from(RoomImagePackEventContent::TYPE),
            )),
            can_pin_events: self.is_allowed_to(PowerLevelAction::SendState(
                StateEventType::RoomPinnedEvents,
            )),
            can_redact_own: self.is_allowed_to(PowerLevelAction::RedactOwn),
            can_redact_other: self.is_allowed_to(PowerLevelAction::RedactOther),
            can_notify_room: self.is_allowed_to(PowerLevelAction::TriggerNotification(
                NotificationPowerLevelType::Room,
            )),
        };

        self.state.set_if_not_eq(state);
    }
}

/// The power level to mute members: they must not have enough power to
/// send messages.
fn mute_power_level(power_levels: &RoomPowerLevels) -> i64 {
    let message_power_level = power_levels
        .events
        .get(&MessageLikeEventType::RoomMessage.into())
        .copied()
        .unwrap_or(power_levels.events_default);
    (-1).min(message_power_level.into())
}

/// The permissions subpage's rows: the power levels a room's members need
/// for each action the page names, flattened.
///
/// Reading fills the rows from the power levels as the page does; applying
/// collects them back as the page's save does — an override equal to its
/// default is removed rather than written, and redacting one's own message
/// can never need more power than redacting somebody else's, since the
/// latter is what the former is measured against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerLevelsMatrix {
    /// The default level of a member.
    pub users_default: i64,
    /// The level needed to send messages.
    pub events_default: i64,
    /// The default level needed to change state.
    pub state_default: i64,
    /// The level needed to invite.
    pub invite: i64,
    /// The level needed to kick.
    pub kick: i64,
    /// The level needed to ban.
    pub ban: i64,
    /// The level needed to remove others' messages.
    pub redact_others: i64,
    /// The level needed to remove one's own messages.
    pub redact_own: i64,
    /// The level needed to notify the whole room.
    pub notify_room: i64,
    /// The level needed to change the name.
    pub name: i64,
    /// The level needed to change the topic.
    pub topic: i64,
    /// The level needed to change the avatar.
    pub avatar: i64,
    /// The level needed to change the addresses.
    pub aliases: i64,
    /// The level needed to change the history visibility.
    pub history_visibility: i64,
    /// The level needed to enable encryption.
    pub encryption: i64,
    /// The level needed to change the permissions.
    pub power_levels: i64,
    /// The level needed to change the server ACL.
    pub server_acl: i64,
    /// The level needed to upgrade the room.
    pub upgrade: i64,
}

impl PowerLevelsMatrix {
    /// The rows for the given power levels, as the page fills them.
    #[must_use]
    pub fn from_power_levels(power_levels: &RoomPowerLevels) -> Self {
        let events_default = i64::from(power_levels.events_default);
        let state_default = i64::from(power_levels.state_default);

        let redact_own = event_power_level(
            power_levels,
            &TimelineEventType::RoomRedaction,
            events_default,
        );
        let redact_others = redact_own.max(i64::from(power_levels.redact));

        Self {
            users_default: power_levels.users_default.into(),
            events_default,
            state_default,
            invite: power_levels.invite.into(),
            kick: power_levels.kick.into(),
            ban: power_levels.ban.into(),
            redact_others,
            redact_own,
            notify_room: power_levels.notifications.room.into(),
            name: event_power_level(power_levels, &TimelineEventType::RoomName, state_default),
            topic: event_power_level(power_levels, &TimelineEventType::RoomTopic, state_default),
            avatar: event_power_level(power_levels, &TimelineEventType::RoomAvatar, state_default),
            aliases: event_power_level(
                power_levels,
                &TimelineEventType::RoomCanonicalAlias,
                state_default,
            ),
            history_visibility: event_power_level(
                power_levels,
                &TimelineEventType::RoomHistoryVisibility,
                state_default,
            ),
            encryption: event_power_level(
                power_levels,
                &TimelineEventType::RoomEncryption,
                state_default,
            ),
            power_levels: event_power_level(
                power_levels,
                &TimelineEventType::RoomPowerLevels,
                state_default,
            ),
            server_acl: event_power_level(
                power_levels,
                &TimelineEventType::RoomServerAcl,
                state_default,
            ),
            upgrade: event_power_level(
                power_levels,
                &TimelineEventType::RoomTombstone,
                state_default,
            ),
        }
    }

    /// Collect the rows into the given power levels, as the page's save
    /// does.
    ///
    /// The per-user levels are not the page's rows and are left as they
    /// are.
    pub fn apply_to(&self, power_levels: &mut RoomPowerLevels) {
        let int = Int::new_saturating;

        power_levels.events_default = int(self.events_default);

        // `redact_own` cannot be higher than `redact_others` because
        // `redact_others` depends also on `redact_own`.
        let redact_own = self.redact_own.min(self.redact_others);
        set_event_power_level(
            power_levels,
            TimelineEventType::RoomRedaction,
            redact_own,
            self.events_default,
        );
        power_levels.redact = int(self.redact_others);

        power_levels.notifications.room = int(self.notify_room);
        power_levels.state_default = int(self.state_default);

        for (event_type, value) in [
            (TimelineEventType::RoomName, self.name),
            (TimelineEventType::RoomTopic, self.topic),
            (TimelineEventType::RoomAvatar, self.avatar),
            (TimelineEventType::RoomCanonicalAlias, self.aliases),
            (
                TimelineEventType::RoomHistoryVisibility,
                self.history_visibility,
            ),
            (TimelineEventType::RoomEncryption, self.encryption),
            (TimelineEventType::RoomPowerLevels, self.power_levels),
            (TimelineEventType::RoomServerAcl, self.server_acl),
            (TimelineEventType::RoomTombstone, self.upgrade),
        ] {
            set_event_power_level(power_levels, event_type, value, self.state_default);
        }

        power_levels.invite = int(self.invite);
        power_levels.kick = int(self.kick);
        power_levels.ban = int(self.ban);
        power_levels.users_default = int(self.users_default);
    }
}

/// Get the necessary power level for the given event type in the given
/// power levels.
fn event_power_level(
    power_levels: &RoomPowerLevels,
    event_type: &TimelineEventType,
    default: i64,
) -> i64 {
    power_levels
        .events
        .get(event_type)
        .copied()
        .map_or(default, Into::into)
}

/// Set the power level for the given event type in the given power levels.
fn set_event_power_level(
    power_levels: &mut RoomPowerLevels,
    event_type: TimelineEventType,
    value: i64,
    default: i64,
) {
    if value == default {
        power_levels.events.remove(&event_type);
    } else {
        power_levels
            .events
            .insert(event_type, Int::new_saturating(value));
    }
}

#[cfg(test)]
mod tests {
    use ruma::int;

    use super::*;

    fn power_levels() -> RoomPowerLevels {
        RoomPowerLevels::new(RoomPowerLevelsSource::None, &AuthorizationRules::V1, None)
    }

    /// The rows of the defaults collect back into the defaults, so opening
    /// the page and saving it writes nothing new.
    #[test]
    fn the_defaults_round_trip() {
        let mut levels = power_levels();
        let matrix = PowerLevelsMatrix::from_power_levels(&levels);

        let before = levels.clone();
        matrix.apply_to(&mut levels);

        assert_eq!(levels.events, before.events);
        assert_eq!(levels.events_default, before.events_default);
        assert_eq!(levels.state_default, before.state_default);
        assert_eq!(levels.redact, before.redact);
        assert_eq!(levels.users_default, before.users_default);
    }

    /// An override equal to its default is not written.
    #[test]
    fn an_override_equal_to_the_default_is_elided() {
        let mut levels = power_levels();
        let mut matrix = PowerLevelsMatrix::from_power_levels(&levels);

        matrix.name = matrix.state_default;
        matrix.topic = matrix.state_default + 1;
        matrix.apply_to(&mut levels);

        assert!(!levels.events.contains_key(&TimelineEventType::RoomName));
        assert_eq!(
            levels.events.get(&TimelineEventType::RoomTopic).copied(),
            Some(Int::new_saturating(matrix.state_default + 1))
        );
    }

    /// Redacting one's own message can never need more power than
    /// redacting somebody else's.
    #[test]
    fn redact_own_is_clamped_to_redact_others() {
        let mut levels = power_levels();
        let mut matrix = PowerLevelsMatrix::from_power_levels(&levels);

        matrix.redact_own = 75;
        matrix.redact_others = 50;
        matrix.apply_to(&mut levels);

        assert_eq!(
            levels
                .events
                .get(&TimelineEventType::RoomRedaction)
                .copied(),
            Some(int!(50))
        );
        assert_eq!(levels.redact, int!(50));
    }

    /// The page shows redacting others as at least redacting one's own.
    #[test]
    fn redact_others_reads_as_at_least_redact_own() {
        let mut levels = power_levels();
        levels
            .events
            .insert(TimelineEventType::RoomRedaction, int!(60));
        levels.redact = int!(50);

        let matrix = PowerLevelsMatrix::from_power_levels(&levels);

        assert_eq!(matrix.redact_own, 60);
        assert_eq!(matrix.redact_others, 60);
    }
}

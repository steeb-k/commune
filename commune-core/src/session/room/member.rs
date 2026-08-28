//! The members of a room.
//!
//! The headless counterpart of the application's `Member`/`MemberList`
//! `GObject`s: members are value snapshots, and the list is an
//! [`ObservableVector`] whose `VectorDiff`s are the change signal — a
//! member update arrives as a `Set` at the member's index.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use eyeball::SharedObservable;
use eyeball_im::{ObservableVector, VectorDiff};
use futures_util::Stream;
use matrix_sdk::{RoomMemberships, room::RoomMember};
use ruma::{
    OwnedMxcUri, OwnedUserId, UserId,
    events::room::{
        member::MembershipState,
        power_levels::{RoomPowerLevels, UserPowerLevel},
    },
};
use tracing::error;

use crate::{spawn_tokio, utils::LoadingState};

/// The minimum power level of an administrator.
pub const POWER_LEVEL_ADMIN: i64 = 100;

/// The minimum power level of a moderator.
pub const POWER_LEVEL_MOD: i64 = 50;

/// The possible states of membership of a user in a room.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    /// The user left the room, or was never in the room.
    #[default]
    Leave,
    /// The user is currently in the room.
    Join,
    /// The user was invited to the room.
    Invite,
    /// The user was banned from the room.
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

/// The role of a room member, derived from their power level.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MemberRole {
    /// A room creator, with infinite power level.
    Creator,
    /// An administrator.
    Administrator,
    /// A moderator.
    Moderator,
    /// A member with the room's default power level.
    #[default]
    Default,
    /// A member without enough power to send messages.
    Muted,
    /// A member with a power level that matches no other role.
    Custom,
}

/// The role for the given power level under the given room power levels —
/// the application's `Permissions::role()` rules.
pub fn role(power_level: UserPowerLevel, power_levels: &RoomPowerLevels) -> MemberRole {
    let UserPowerLevel::Int(power_level) = power_level else {
        return MemberRole::Creator;
    };

    let power_level = i64::from(power_level);
    let default_power_level = i64::from(power_levels.users_default);
    let mute_power_level = mute_power_level(power_levels);

    if power_level >= POWER_LEVEL_ADMIN {
        MemberRole::Administrator
    } else if power_level >= POWER_LEVEL_MOD {
        MemberRole::Moderator
    } else if power_level == default_power_level {
        MemberRole::Default
    } else if power_level < default_power_level && power_level <= mute_power_level {
        // Only set role as muted for members below default, to avoid visual
        // noise in rooms where muted is the default.
        MemberRole::Muted
    } else {
        MemberRole::Custom
    }
}

/// The power level at which a member counts as muted: not enough power to
/// send messages.
fn mute_power_level(power_levels: &RoomPowerLevels) -> i64 {
    let message_power_level = power_levels
        .events
        .get(&ruma::events::MessageLikeEventType::RoomMessage.into())
        .copied()
        .unwrap_or(power_levels.events_default);
    (-1).min(message_power_level.into())
}

/// A member of a room, as a value snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    /// The Matrix ID of the member.
    pub user_id: OwnedUserId,
    /// The display name of the member, if any.
    pub display_name: Option<String>,
    /// Whether the display name is shared with another member.
    pub is_name_ambiguous: bool,
    /// The avatar of the member, if any.
    pub avatar_url: Option<OwnedMxcUri>,
    /// The power level of the member.
    pub power_level: UserPowerLevel,
    /// The role of the member under the room's power levels.
    pub role: MemberRole,
    /// The membership state of the member.
    pub membership: Membership,
}

impl Member {
    /// Build a snapshot from the SDK's member under the given power levels.
    fn from_room_member(member: &RoomMember, power_levels: &RoomPowerLevels) -> Self {
        Self {
            user_id: member.user_id().to_owned(),
            display_name: member.display_name().map(ToOwned::to_owned),
            is_name_ambiguous: member.name_ambiguous(),
            avatar_url: member.avatar_url().map(ToOwned::to_owned),
            power_level: member.power_level(),
            role: role(member.power_level(), power_levels),
            membership: member.membership().into(),
        }
    }

    /// The name this member displays as.
    #[must_use]
    pub fn display_name_or_localpart(&self) -> String {
        self.display_name
            .clone()
            .unwrap_or_else(|| self.user_id.localpart().to_owned())
    }
}

/// The list of members of a room.
///
/// Members arrive in insertion order, like the application's list; new
/// members are appended, updates are `Set` diffs in place. Members are
/// never removed — a leave or ban is a membership change, not a removal,
/// as the application has it.
#[derive(Debug, Clone)]
pub struct MemberList {
    inner: Arc<MemberListInner>,
}

#[derive(Debug)]
struct MemberListInner {
    matrix_room: matrix_sdk::room::Room,
    members: Mutex<MemberStore>,
    state: SharedObservable<LoadingState>,
}

#[derive(Debug, Default)]
struct MemberStore {
    list: ObservableVector<Member>,
    index: HashMap<OwnedUserId, usize>,
}

impl MemberStore {
    /// Insert or update the given member snapshot.
    fn upsert(&mut self, member: Member) {
        if let Some(&index) = self.index.get(&member.user_id) {
            if self.list[index] != member {
                self.list.set(index, member);
            }
        } else {
            self.index.insert(member.user_id.clone(), self.list.len());
            self.list.push_back(member);
        }
    }
}

impl MemberList {
    pub(crate) fn new(matrix_room: matrix_sdk::room::Room) -> Self {
        Self {
            inner: Arc::new(MemberListInner {
                matrix_room,
                members: Mutex::new(MemberStore::default()),
                state: SharedObservable::new(LoadingState::Initial),
            }),
        }
    }

    /// Subscribe to the list: the current members and the diffs that follow.
    pub fn subscribe(
        &self,
    ) -> (
        eyeball_im::Vector<Member>,
        impl Stream<Item = Vec<VectorDiff<Member>>> + use<>,
    ) {
        self.inner
            .members
            .lock()
            .expect("mutex is not poisoned")
            .list
            .subscribe()
            .into_values_and_batched_stream()
    }

    /// A snapshot of the members known right now.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Member> {
        self.inner
            .members
            .lock()
            .expect("mutex is not poisoned")
            .list
            .iter()
            .cloned()
            .collect()
    }

    /// The member with the given ID, if it is known.
    #[must_use]
    pub fn get(&self, user_id: &UserId) -> Option<Member> {
        let members = self.inner.members.lock().expect("mutex is not poisoned");
        members
            .index
            .get(user_id)
            .map(|&index| members.list[index].clone())
    }

    /// The loading state of the list.
    #[must_use]
    pub fn state(&self) -> LoadingState {
        self.inner.state.get()
    }

    /// Load the list: what the store has first, then the server if the
    /// members are not all synced — the application's two-phase load.
    pub async fn load(&self) {
        if matches!(self.state(), LoadingState::Loading | LoadingState::Ready) {
            return;
        }
        self.inner.state.set_if_not_eq(LoadingState::Loading);

        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let mut memberships = RoomMemberships::all();
            memberships.remove(RoomMemberships::LEAVE);

            let members = matrix_room.members_no_sync(memberships).await?;
            let power_levels = matrix_room.power_levels().await?;
            Ok::<_, matrix_sdk::Error>((members, power_levels))
        });

        match handle.await.expect("task was not aborted") {
            Ok((members, power_levels)) => {
                self.update_from_room_members(&members, &power_levels);

                if self.inner.matrix_room.are_members_synced() {
                    self.inner.state.set_if_not_eq(LoadingState::Ready);
                    return;
                }
            }
            Err(load_error) => {
                error!("Could not load room members from store: {load_error}");
            }
        }

        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let mut memberships = RoomMemberships::all();
            memberships.remove(RoomMemberships::LEAVE);

            let members = matrix_room.members(memberships).await?;
            let power_levels = matrix_room.power_levels().await?;
            Ok::<_, matrix_sdk::Error>((members, power_levels))
        });

        match handle.await.expect("task was not aborted") {
            Ok((members, power_levels)) => {
                self.update_from_room_members(&members, &power_levels);
                self.inner.state.set_if_not_eq(LoadingState::Ready);
            }
            Err(load_error) => {
                self.inner.state.set_if_not_eq(LoadingState::Error);
                error!("Could not load room members from server: {load_error}");
            }
        }
    }

    /// Refresh the list from the store — the room info changed, so
    /// memberships or power levels may have.
    pub(crate) async fn refresh(&self) {
        if self.state() != LoadingState::Ready {
            return;
        }

        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let mut memberships = RoomMemberships::all();
            memberships.remove(RoomMemberships::LEAVE);

            let members = matrix_room.members_no_sync(memberships).await?;
            let power_levels = matrix_room.power_levels().await?;
            Ok::<_, matrix_sdk::Error>((members, power_levels))
        });

        match handle.await.expect("task was not aborted") {
            Ok((members, power_levels)) => {
                self.update_from_room_members(&members, &power_levels);
            }
            Err(refresh_error) => {
                error!("Could not refresh room members: {refresh_error}");
            }
        }
    }

    /// Fold the SDK's members into the list.
    fn update_from_room_members(&self, new_members: &[RoomMember], power_levels: &RoomPowerLevels) {
        let mut members = self.inner.members.lock().expect("mutex is not poisoned");
        for room_member in new_members {
            members.upsert(Member::from_room_member(room_member, power_levels));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn power_levels(users_default: i64, message: Option<i64>) -> RoomPowerLevels {
        use ruma::{
            events::room::power_levels::{RoomPowerLevelsEventContent, RoomPowerLevelsSource},
            room_version_rules::AuthorizationRules,
        };

        let mut content = RoomPowerLevelsEventContent::new(&AuthorizationRules::V11);
        content.users_default = users_default.try_into().expect("small test value");
        if let Some(message) = message {
            content.events.insert(
                ruma::events::MessageLikeEventType::RoomMessage.into(),
                message.try_into().expect("small test value"),
            );
        }
        RoomPowerLevels::new(
            RoomPowerLevelsSource::Original(content),
            &AuthorizationRules::V11,
            None,
        )
    }

    fn int_level(level: i64) -> UserPowerLevel {
        UserPowerLevel::Int(level.try_into().expect("small test value"))
    }

    #[test]
    fn roles_follow_the_application_rules() {
        let levels = power_levels(0, None);

        assert_eq!(role(int_level(100), &levels), MemberRole::Administrator);
        assert_eq!(role(int_level(150), &levels), MemberRole::Administrator);
        assert_eq!(role(int_level(50), &levels), MemberRole::Moderator);
        assert_eq!(role(int_level(99), &levels), MemberRole::Moderator);
        assert_eq!(role(int_level(0), &levels), MemberRole::Default);
        assert_eq!(role(int_level(-1), &levels), MemberRole::Muted);
        assert_eq!(role(int_level(25), &levels), MemberRole::Custom);
        assert_eq!(role(UserPowerLevel::Infinite, &levels), MemberRole::Creator);
    }

    #[test]
    fn muted_needs_to_be_below_default() {
        // Default is 5, messages need 10: a member at 5 is Default, not
        // Muted, even though they cannot send messages — the application
        // avoids that visual noise.
        let levels = power_levels(5, Some(10));

        assert_eq!(role(int_level(5), &levels), MemberRole::Default);
        // Mute level is min(-1, 10) = -1, so 3 is between mute and default:
        // custom.
        assert_eq!(role(int_level(3), &levels), MemberRole::Custom);
        assert_eq!(role(int_level(-2), &levels), MemberRole::Muted);
    }

    #[test]
    fn membership_conversion_covers_the_states() {
        assert_eq!(Membership::from(&MembershipState::Join), Membership::Join);
        assert_eq!(Membership::from(&MembershipState::Leave), Membership::Leave);
        assert_eq!(
            Membership::from(&MembershipState::Invite),
            Membership::Invite
        );
        assert_eq!(Membership::from(&MembershipState::Ban), Membership::Ban);
        assert_eq!(Membership::from(&MembershipState::Knock), Membership::Knock);
    }
}

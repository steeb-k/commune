//! The join rule of a room, headless.
//!
//! The application's `JoinRule` object: the simplified value, whether
//! knocking is allowed, the room a restricted rule points at, and whether
//! we — or anyone — may join on our own, all following the SDK's rule as
//! the room info changes. The display name the application renders from
//! these is the embedder's sentence and stays with it. The join-rule
//! subpage's `compute_join_rule` is here too, tests included, because it is
//! the one piece of that page that decides what goes on the wire.

use eyeball::{SharedObservable, Subscriber};
use matrix_sdk::{RoomState, room::Room as MatrixRoom};
use ruma::{
    OwnedRoomId,
    events::room::join_rules::{
        AllowRule, JoinRule as MatrixJoinRule, Restricted, RoomJoinRulesEventContent,
    },
};
use tracing::error;

use super::RoomSettingsError;
use crate::{session::WeakSession, spawn_tokio};

/// Simplified join rules.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy)]
pub enum JoinRuleValue {
    /// Only invited users can join.
    #[default]
    Invite,
    /// Anyone can join.
    Public,
    /// Members of a room can join.
    RoomMembership,
    /// The rule is unsupported.
    Unsupported,
}

impl JoinRuleValue {
    /// Whether we support editing this join rule.
    #[must_use]
    pub fn can_be_edited(self) -> bool {
        matches!(self, Self::Invite | Self::Public | Self::RoomMembership)
    }
}

impl From<&MatrixJoinRule> for JoinRuleValue {
    fn from(value: &MatrixJoinRule) -> Self {
        match value {
            MatrixJoinRule::Invite | MatrixJoinRule::Knock => Self::Invite,
            MatrixJoinRule::Restricted(restricted)
            | MatrixJoinRule::KnockRestricted(restricted) => {
                if has_restricted_membership_room(restricted) {
                    Self::RoomMembership
                } else {
                    Self::Unsupported
                }
            }
            MatrixJoinRule::Public => Self::Public,
            _ => Self::Unsupported,
        }
    }
}

/// The join rule of a room, as the application's object presents it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JoinRuleState {
    /// The current join rule from the SDK.
    pub matrix_join_rule: Option<MatrixJoinRule>,
    /// The value of the join rule.
    pub value: JoinRuleValue,
    /// Whether users can knock.
    pub can_knock: bool,
    /// The room we need to be a member of to match this join rule, if any.
    ///
    /// The application resolves it to a local or remote room for its
    /// display name; the embedder does the same with the ID.
    // TODO: Support multiple rooms.
    pub membership_room_id: Option<OwnedRoomId>,
    /// Whether our own user can join this room on their own.
    pub we_can_join: bool,
    /// Whether anyone can join this room on their own.
    pub anyone_can_join: bool,
}

/// The join rule of a room.
#[derive(Debug)]
pub struct JoinRule {
    /// The room API of the SDK.
    matrix_room: MatrixRoom,
    /// The current session, for the rooms a restricted rule names.
    session: WeakSession,
    /// The current state.
    state: SharedObservable<JoinRuleState>,
}

impl JoinRule {
    /// Create the join rule of the given room.
    pub(crate) fn new(matrix_room: MatrixRoom, session: WeakSession) -> Self {
        Self {
            matrix_room,
            session,
            state: SharedObservable::new(JoinRuleState::default()),
        }
    }

    /// The current state.
    #[must_use]
    pub fn state(&self) -> JoinRuleState {
        self.state.get()
    }

    /// Subscribe to the state.
    pub fn subscribe(&self) -> Subscriber<JoinRuleState> {
        self.state.subscribe()
    }

    /// The current join rule from the SDK.
    #[must_use]
    pub fn matrix_join_rule(&self) -> Option<MatrixJoinRule> {
        self.state.get().matrix_join_rule
    }

    /// Update the join rule with the given value from the SDK.
    ///
    /// Whether we can join is recomputed with it, since our own membership
    /// arrives through the same room info; the application watches its own
    /// member for that.
    pub(crate) fn update(&self, join_rule: Option<&MatrixJoinRule>) {
        let value = join_rule.map(Into::into).unwrap_or_default();
        let can_knock = join_rule.is_some_and(|rule| {
            matches!(
                rule,
                MatrixJoinRule::Knock | MatrixJoinRule::KnockRestricted(_)
            )
        });
        let membership_room_id = join_rule.and_then(|rule| match rule {
            MatrixJoinRule::Restricted(restricted)
            | MatrixJoinRule::KnockRestricted(restricted) => restricted_membership_room(restricted),
            _ => None,
        });
        let we_can_join = self.we_can_join(join_rule);
        let anyone_can_join = join_rule.is_some_and(|rule| *rule == MatrixJoinRule::Public);

        self.state.set_if_not_eq(JoinRuleState {
            matrix_join_rule: join_rule.cloned(),
            value,
            can_knock,
            membership_room_id,
            we_can_join,
            anyone_can_join,
        });
    }

    /// Whether our own user can join this room on their own.
    fn we_can_join(&self, matrix_join_rule: Option<&MatrixJoinRule>) -> bool {
        let Some(matrix_join_rule) = matrix_join_rule else {
            return false;
        };

        if self.matrix_room.state() == RoomState::Banned {
            return false;
        }

        match matrix_join_rule {
            MatrixJoinRule::Public => true,
            MatrixJoinRule::Restricted(rules) | MatrixJoinRule::KnockRestricted(rules) => rules
                .allow
                .iter()
                .any(|rule| self.we_pass_restricted_allow_rule(rule)),
            _ => false,
        }
    }

    /// Whether our account passes the given restricted allow rule.
    fn we_pass_restricted_allow_rule(&self, rule: &AllowRule) -> bool {
        match rule {
            AllowRule::RoomMembership(room_membership) => {
                self.session.upgrade().is_some_and(|session| {
                    session
                        .room_list()
                        .get(&room_membership.room_id)
                        .is_some_and(|room| room.is_joined())
                })
            }
            _ => false,
        }
    }

    /// Change the join rule.
    pub async fn set_matrix_join_rule(
        &self,
        rule: MatrixJoinRule,
    ) -> Result<(), RoomSettingsError> {
        let content = RoomJoinRulesEventContent::new(rule);

        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(content).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|send_error| {
                error!("Could not change join rule: {send_error}");
                RoomSettingsError::JoinRule(Box::new(send_error))
            })
    }
}

/// Whether the given restricted rule allows a room membership.
fn has_restricted_membership_room(restricted: &Restricted) -> bool {
    restricted
        .allow
        .iter()
        .any(|a| matches!(a, AllowRule::RoomMembership(_)))
}

/// The ID of the first room, if the given restricted rule allows a room
/// membership.
fn restricted_membership_room(restricted: &Restricted) -> Option<OwnedRoomId> {
    restricted.allow.iter().find_map(|a| match a {
        AllowRule::RoomMembership(m) => Some(m.room_id.clone()),
        _ => None,
    })
}

/// Compute the join rule to send from the state of the join-rule page.
///
/// `restricted` is the allow list the page holds — the space that was
/// picked, or the room's saved list if none was.
///
/// Returns `None` when the page describes no rule that can be sent: the
/// room membership rule with nothing to allow. That is a half-finished
/// choice, not a rule, and it must never be turned into a restricted rule
/// that admits nobody — a state the page could not undo.
#[must_use]
pub fn compute_join_rule(
    value: JoinRuleValue,
    can_knock: bool,
    restricted: Option<Restricted>,
) -> Option<MatrixJoinRule> {
    let rule = match value {
        JoinRuleValue::RoomMembership => {
            let restricted = restricted?;

            if can_knock {
                MatrixJoinRule::KnockRestricted(restricted)
            } else {
                MatrixJoinRule::Restricted(restricted)
            }
        }
        JoinRuleValue::Public => MatrixJoinRule::Public,
        // An unsupported rule cannot be selected, so it can only be the
        // invite rule here.
        _ => {
            if can_knock {
                MatrixJoinRule::Knock
            } else {
                MatrixJoinRule::Invite
            }
        }
    };

    Some(rule)
}

#[cfg(test)]
mod tests {
    use ruma::owned_room_id;

    use super::*;

    /// A restricted rule allowing the members of one space.
    fn space_membership() -> Restricted {
        Restricted::new(vec![AllowRule::room_membership(owned_room_id!(
            "!space:example.org"
        ))])
    }

    /// A restricted rule allowing the members of two spaces.
    fn two_space_membership() -> Restricted {
        Restricted::new(vec![
            AllowRule::room_membership(owned_room_id!("!space:example.org")),
            AllowRule::room_membership(owned_room_id!("!other:example.org")),
        ])
    }

    #[test]
    fn invite_rule_follows_the_knock_switch() {
        assert_eq!(
            compute_join_rule(JoinRuleValue::Invite, false, None),
            Some(MatrixJoinRule::Invite)
        );
        assert_eq!(
            compute_join_rule(JoinRuleValue::Invite, true, None),
            Some(MatrixJoinRule::Knock)
        );
    }

    #[test]
    fn public_rule_ignores_the_knock_switch() {
        assert_eq!(
            compute_join_rule(JoinRuleValue::Public, false, None),
            Some(MatrixJoinRule::Public)
        );
        // Knocking is meaningless on a room anyone can join, and the switch
        // is insensitive there, but the value must not leak into the rule.
        assert_eq!(
            compute_join_rule(JoinRuleValue::Public, true, None),
            Some(MatrixJoinRule::Public)
        );
    }

    #[test]
    fn room_membership_rule_keeps_the_allow_list() {
        let restricted = space_membership();

        assert_eq!(
            compute_join_rule(
                JoinRuleValue::RoomMembership,
                false,
                Some(restricted.clone())
            ),
            Some(MatrixJoinRule::Restricted(restricted.clone()))
        );
        assert_eq!(
            compute_join_rule(
                JoinRuleValue::RoomMembership,
                true,
                Some(restricted.clone())
            ),
            Some(MatrixJoinRule::KnockRestricted(restricted))
        );
    }

    #[test]
    fn an_allow_list_of_several_spaces_is_carried_over_whole() {
        // The picker replaces the whole list, but a list left alone must
        // survive a change to the knock switch intact — dropping a space
        // would quietly lock people out.
        let restricted = two_space_membership();

        assert_eq!(
            compute_join_rule(
                JoinRuleValue::RoomMembership,
                true,
                Some(restricted.clone())
            ),
            Some(MatrixJoinRule::KnockRestricted(restricted))
        );
    }

    #[test]
    fn turning_knocking_on_and_off_is_lossless() {
        let restricted = space_membership();

        // The round trip a user makes when they flip the switch twice must
        // give back the rule they started from, allow list and all.
        let with_knocking = compute_join_rule(
            JoinRuleValue::RoomMembership,
            true,
            Some(restricted.clone()),
        );
        let Some(MatrixJoinRule::KnockRestricted(carried_over)) = with_knocking else {
            panic!("expected a knock restricted rule");
        };

        assert_eq!(
            compute_join_rule(JoinRuleValue::RoomMembership, false, Some(carried_over)),
            Some(MatrixJoinRule::Restricted(restricted))
        );
    }

    #[test]
    fn room_membership_without_an_allow_list_is_not_a_rule() {
        // The page reaches this state: the rule is selected and no space
        // has been chosen yet. It must produce nothing at all — neither a
        // restricted rule that admits nobody, which the page could not
        // undo, nor a silent fall back to a different rule than the one on
        // screen.
        assert_eq!(
            compute_join_rule(JoinRuleValue::RoomMembership, false, None),
            None
        );
        assert_eq!(
            compute_join_rule(JoinRuleValue::RoomMembership, true, None),
            None
        );
    }

    #[test]
    fn unsupported_rule_is_never_sent() {
        assert_eq!(
            compute_join_rule(JoinRuleValue::Unsupported, false, None),
            Some(MatrixJoinRule::Invite)
        );
    }

    /// A restricted rule without a room membership is unsupported, as the
    /// application's value reads it.
    #[test]
    fn value_of_a_restricted_rule_needs_a_room() {
        assert_eq!(
            JoinRuleValue::from(&MatrixJoinRule::Restricted(space_membership())),
            JoinRuleValue::RoomMembership
        );
        assert_eq!(
            JoinRuleValue::from(&MatrixJoinRule::Restricted(Restricted::new(Vec::new()))),
            JoinRuleValue::Unsupported
        );
        assert_eq!(
            JoinRuleValue::from(&MatrixJoinRule::Knock),
            JoinRuleValue::Invite
        );
    }
}

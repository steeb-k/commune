//! Upgrading a room, headless.
//!
//! The application's `UpgradeInfo`: which room versions an upgrade may go
//! to, computed from the current version and the server's capabilities by
//! the upgrade dialog's rules, with the privileged creators the upgrade
//! would demote and the join rule the dialog warns about. The version
//! ordering is the application's digit-sequence-aware comparison, tests
//! included: the specification defines no order for room versions, and the
//! ones that exist are numbers with occasional prefixes.

use std::cmp::Ordering;

use ruma::{
    OwnedUserId, RoomVersionId, UserId,
    api::client::discovery::get_capabilities::v3::{RoomVersionStability, RoomVersionsCapability},
    events::{StateEventType, room::power_levels::PowerLevelAction},
};

use super::{JoinRuleValue, Room};

/// The information necessary to offer a room upgrade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeInfo {
    /// The current version of the room.
    pub current_room_version: RoomVersionId,
    /// The sorted stable room versions available for the upgrade.
    pub stable_room_versions: Vec<RoomVersionId>,
    /// The sorted unstable room versions available for the upgrade.
    pub unstable_room_versions: Vec<RoomVersionId>,
    /// The position of the room version that should be selected by
    /// default, when `stable_room_versions` and `unstable_room_versions`
    /// are concatenated.
    pub selected: usize,
    /// Whether our own user is a privileged creator in the current room.
    pub own_user_is_creator: bool,
    /// The number of privileged creators that are not our own user in the
    /// current room.
    pub other_creators_count: usize,
    /// The current join rule of the room.
    pub join_rule: JoinRuleValue,
}

impl UpgradeInfo {
    /// Construct an empty `UpgradeInfo` for a room of the given version and
    /// join rule.
    #[must_use]
    pub fn new(current_room_version: RoomVersionId, join_rule: JoinRuleValue) -> Self {
        Self {
            current_room_version,
            stable_room_versions: vec![],
            unstable_room_versions: vec![],
            selected: 0,
            own_user_is_creator: false,
            other_creators_count: 0,
            join_rule,
        }
    }

    /// Add information about the possible room versions for the upgrade.
    ///
    /// We do not allow users to:
    ///
    /// - Downgrade the room, i.e. use a lower room version.
    /// - Upgrade to a version lower than the server's default, if the server's
    ///   default is stable.
    /// - Upgrade to an experimental version, unless it is the current version
    ///   or the server's default.
    ///
    /// If the server's default is experimental, we also allow to upgrade to
    /// the highest stable version.
    #[must_use]
    pub fn with_room_versions(mut self, capability: &RoomVersionsCapability) -> Self {
        let current_room_version = &self.current_room_version;
        let current_is_stable = capability.is_stable_version(current_room_version);
        let default_is_stable = capability.is_stable_version(&capability.default);
        let maximum_stable_version = capability.maximum_stable_version();

        // The minimum stable version is the highest stable version between
        // the current version and the default version.
        let minimum_stable_version = match (current_is_stable, default_is_stable) {
            (true, false) => Some(current_room_version),
            (false, true) => Some(&capability.default),
            (true, true) => Some(
                match cmp_room_versions(current_room_version, &capability.default) {
                    Ordering::Less => &capability.default,
                    Ordering::Equal | Ordering::Greater => current_room_version,
                },
            ),
            (false, false) => None,
        };
        let selected_room_version = minimum_stable_version
            .unwrap_or(&capability.default)
            .clone();

        let mut stable_room_versions: Vec<RoomVersionId> =
            if let Some(minimum) = minimum_stable_version {
                // Keep all the stable versions higher than the minimum.
                capability
                    .available
                    .iter()
                    .filter_map(|(version, stability)| {
                        // Discard unstable versions.
                        if *stability != RoomVersionStability::Stable {
                            return None;
                        }

                        if cmp_room_versions(version, minimum) != Ordering::Less
                            || maximum_stable_version.is_some_and(|maximum| maximum == version)
                        {
                            Some(version)
                        } else {
                            None
                        }
                    })
                    .cloned()
                    .collect()
            } else {
                // The only allowed stable version will be the maximum.
                maximum_stable_version.into_iter().cloned().collect()
            };

        let mut unstable_room_versions = Vec::new();

        // Add the current version if it is unstable.
        if !current_is_stable {
            unstable_room_versions.push(current_room_version.clone());
        }

        // Add the default version if it is different from the current
        // version and it is unstable.
        if *current_room_version != capability.default && !default_is_stable {
            unstable_room_versions.push(capability.default.clone());
        }

        // Sort all the versions.
        stable_room_versions.sort_unstable_by(cmp_room_versions);
        unstable_room_versions.sort_unstable_by(cmp_room_versions);

        // Find the position of the selected version.
        self.selected = stable_room_versions
            .binary_search_by(|version| cmp_room_versions(version, &selected_room_version))
            .or_else(|_| {
                unstable_room_versions
                    .binary_search_by(|version| cmp_room_versions(version, &selected_room_version))
                    .map(|pos| stable_room_versions.len() + pos)
            })
            .unwrap_or_default();
        self.stable_room_versions = stable_room_versions;
        self.unstable_room_versions = unstable_room_versions;

        self
    }

    /// Add information about the privileged creators changes.
    #[must_use]
    pub fn with_privileged_creators(
        mut self,
        own_creator: &UserId,
        privileged_creators: &[OwnedUserId],
    ) -> Self {
        self.own_user_is_creator = privileged_creators
            .iter()
            .any(|creator| creator == own_creator);
        self.other_creators_count =
            privileged_creators.len() - usize::from(self.own_user_is_creator);
        self
    }
}

impl Room {
    /// The information to offer an upgrade of this room, given the server's
    /// room versions.
    ///
    /// `None` for a room whose create event is not known, which cannot be
    /// upgraded. The application's general page computes this when the
    /// room or its join rule changes.
    #[must_use]
    pub fn upgrade_info(&self, capability: &RoomVersionsCapability) -> Option<UpgradeInfo> {
        let room_info = self.inner.matrix_room.clone_info();
        let create_content = room_info.create()?;

        let privileged_creators = room_info
            .room_version_rules_or_default()
            .authorization
            .explicitly_privilege_room_creators
            .then(|| room_info.creators())
            .flatten();

        Some(
            UpgradeInfo::new(
                create_content.room_version.clone(),
                self.join_rule().state().value,
            )
            .with_room_versions(capability)
            .with_privileged_creators(
                self.inner.matrix_room.own_user_id(),
                &privileged_creators.unwrap_or_default(),
            ),
        )
    }

    /// Whether this room can be upgraded by our own user.
    ///
    /// Not a direct chat, not upgraded already, and the power to send the
    /// tombstone; the permissions must be loaded for the last one to be
    /// answered.
    #[must_use]
    pub fn can_upgrade(&self) -> bool {
        !self.is_direct()
            && !self.is_tombstoned()
            && self
                .permissions()
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomTombstone))
    }
}

/// Helper trait for [`RoomVersionsCapability`].
trait RoomVersionsCapabilityExt {
    /// Whether the given room version is stable.
    fn is_stable_version(&self, version: &RoomVersionId) -> bool;

    /// The maximum stable room version in these capabilities.
    fn maximum_stable_version(&self) -> Option<&RoomVersionId>;
}

impl RoomVersionsCapabilityExt for RoomVersionsCapability {
    fn is_stable_version(&self, version: &RoomVersionId) -> bool {
        self.available
            .get(version)
            .is_some_and(|stability| *stability == RoomVersionStability::Stable)
    }

    fn maximum_stable_version(&self) -> Option<&RoomVersionId> {
        self.available
            .iter()
            .fold(None, |maximum, (version, stability)| {
                // Discard unstable versions.
                if *stability != RoomVersionStability::Stable {
                    return maximum;
                }

                // Keep the maximum.
                if maximum
                    .is_none_or(|maximum| cmp_room_versions(version, maximum) == Ordering::Greater)
                {
                    Some(version)
                } else {
                    maximum
                }
            })
    }
}

/// Compare the given room versions to attempt to order them.
///
/// Note that the Matrix specification doesn't actually define a grammar or
/// an order for room versions, but so far they have been numbers that grow
/// incrementally.
#[must_use]
pub fn cmp_room_versions(lhs: &RoomVersionId, rhs: &RoomVersionId) -> Ordering {
    cmp_numbers_aware_str(lhs.as_str(), rhs.as_str())
}

/// Compare the given strings by using numbers-aware comparison.
///
/// When we encounter a sequence of digits at the same position in both
/// strings, we compare the values of the numbers. The rest is sorted
/// lexicographically.
fn cmp_numbers_aware_str(lhs: &str, rhs: &str) -> Ordering {
    let lhs_seq = CharSequence::new(lhs);
    let rhs_seq = CharSequence::new(rhs);

    // Early return for empty strings.
    let (lhs_seq, rhs_seq) = match (lhs_seq, rhs_seq) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Less,
        (Some(_), None) => return Ordering::Greater,
        // We need to compare the sequences now.
        (Some(lhs_seq), Some(rhs_seq)) => (lhs_seq, rhs_seq),
    };

    let cmp = lhs_seq.cmp(rhs_seq);

    if cmp.is_eq() {
        // Compare the next sequences.
        cmp_numbers_aware_str(&lhs[lhs_seq.len()..], &rhs[rhs_seq.len()..])
    } else {
        cmp
    }
}

/// A sequence of consecutive digit or non-digit characters.
#[derive(Debug, Clone, Copy)]
enum CharSequence<'a> {
    /// A sequence of digits characters.
    Digits(&'a str),

    /// A sequence of non-digit characters.
    Other(&'a str),
}

impl<'a> CharSequence<'a> {
    /// Get the first sequence of consecutive digit or non-digit characters.
    ///
    /// Returns `None` if the string is empty.
    fn new(string: &'a str) -> Option<Self> {
        // The first character decides the kind of sequence. If there are no
        // characters, the string is empty so we return early.
        let is_digit_seq = string.chars().next()?.is_ascii_digit();

        // Find the end of the sequence.
        let seq_end = string
            .char_indices()
            .find(|(_, c)| is_digit_seq ^ c.is_ascii_digit())
            .map_or(string.len(), |(pos, _)| pos);

        let seq = &string[..seq_end];

        Some(if is_digit_seq {
            Self::Digits(seq)
        } else {
            Self::Other(seq)
        })
    }

    /// Get the length of this sequence in bytes.
    fn len(self) -> usize {
        match self {
            Self::Digits(s) | Self::Other(s) => s.len(),
        }
    }

    /// Compare this sequence with another one.
    fn cmp(self, other: Self) -> Ordering {
        match (self, other) {
            // If one is a sequence of digits and not the other, we can just
            // compare the strings, it will be ordered lexicographically by
            // the first character. And if they are both non-digit
            // sequences, just compare the strings.
            (Self::Digits(lhs) | Self::Other(lhs), Self::Other(rhs))
            | (Self::Other(lhs), Self::Digits(rhs)) => lhs.cmp(rhs),
            // Compare the actual numeric values of digit sequences. To do
            // that we just remove the leading zeroes and compare the
            // length, then the strings.
            (Self::Digits(lhs), Self::Digits(rhs)) => {
                let lhs = lhs.trim_start_matches('0');
                let rhs = rhs.trim_start_matches('0');

                lhs.len().cmp(&rhs.len()).then_with(|| lhs.cmp(rhs))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::cmp_numbers_aware_str;

    #[test]
    fn compare_room_versions() {
        // Compare digits.
        assert_eq!(cmp_numbers_aware_str("1", "1"), Ordering::Equal);
        assert_eq!(cmp_numbers_aware_str("1", "2"), Ordering::Less);
        assert_eq!(cmp_numbers_aware_str("2", "1"), Ordering::Greater);
        assert_eq!(cmp_numbers_aware_str("2", "10"), Ordering::Less);
        assert_eq!(cmp_numbers_aware_str("10", "2"), Ordering::Greater);
        assert_eq!(cmp_numbers_aware_str("0002", "010"), Ordering::Less);
        assert_eq!(cmp_numbers_aware_str("010", "0002"), Ordering::Greater);

        // Compare non-digits.
        assert_eq!(cmp_numbers_aware_str("", "abc"), Ordering::Less);
        assert_eq!(cmp_numbers_aware_str("abc", ""), Ordering::Greater);
        assert_eq!(cmp_numbers_aware_str("ab", "abc"), Ordering::Less);
        assert_eq!(cmp_numbers_aware_str("abc", "ab"), Ordering::Greater);
        assert_eq!(cmp_numbers_aware_str("abc", "abc"), Ordering::Equal);
        assert_eq!(cmp_numbers_aware_str("abc", "d"), Ordering::Less);
        assert_eq!(cmp_numbers_aware_str("d", "abc"), Ordering::Greater);

        // Compare digits with non-digits.
        assert_eq!(cmp_numbers_aware_str("1", "abc"), Ordering::Less);
        assert_eq!(cmp_numbers_aware_str("abc", "1"), Ordering::Greater);
        assert_eq!(cmp_numbers_aware_str("9999", "a"), Ordering::Less);
        assert_eq!(cmp_numbers_aware_str("a", "9999"), Ordering::Greater);
        assert_eq!(cmp_numbers_aware_str("1", "."), Ordering::Greater);
        assert_eq!(cmp_numbers_aware_str(".", "1"), Ordering::Less);

        // Compare mix of digits and non-digits.
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.msc3757.10", "org.matrix.msc3757.10"),
            Ordering::Equal
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.11.hydra", "org.matrix.11.hydra"),
            Ordering::Equal
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.msc3757.10", "org.matrix.msc3757.11"),
            Ordering::Less
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.msc3757.11", "org.matrix.msc3757.10"),
            Ordering::Greater
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.msc3757.10", "org.matrix.hydra.11"),
            Ordering::Greater
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.hydra.11", "org.matrix.msc3757.10"),
            Ordering::Less
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.msc3757.11", "org.matrix.hydra.11"),
            Ordering::Greater
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.hydra.11", "org.matrix.msc3757.11"),
            Ordering::Less
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.10.msc3757", "org.matrix.11.hydra"),
            Ordering::Less
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.11.hydra", "org.matrix.10.msc3757"),
            Ordering::Greater
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.12.msc3757", "org.matrix.11.hydra"),
            Ordering::Greater
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.11.hydra", "org.matrix.12.msc3757"),
            Ordering::Less
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.11.hydra", "org.matrix.0011.hydra"),
            Ordering::Equal
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.11.hydra", "org.matrix.0010.hydra"),
            Ordering::Greater
        );
        assert_eq!(
            cmp_numbers_aware_str("org.matrix.11.hydra", "org.matrix.1.hydra"),
            Ordering::Greater
        );
    }
}

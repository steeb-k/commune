//! The server ACL of a room, headless.
//!
//! The application's `Room::server_acl` and `Room::set_server_acl`, and the
//! server-ACL subpage's rules about what may be sent: an ACL that allows no
//! server at all is refused outright, because it shuts every homeserver out
//! of the room and nothing can send the repair; one that shuts our own
//! server out is a warning the page confirms. The tests are the page's.

use matrix_sdk::deserialized_responses::RawSyncOrStrippedState;
use ruma::{
    ServerName,
    events::{SyncStateEvent, room::server_acl::RoomServerAclEventContent},
};
use tracing::error;

use super::Room;
use crate::{UserFacingError, spawn_tokio};

/// An error encountered while reading or changing the server ACL of a room.
#[derive(Debug, thiserror::Error)]
pub enum ServerAclError {
    /// The ACL could not be read from the store.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` here expensive.
    #[error(transparent)]
    Read(Box<matrix_sdk::Error>),
    /// The ACL could not be sent.
    #[error(transparent)]
    Server(Box<matrix_sdk::Error>),
}

impl UserFacingError for ServerAclError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::Read(_) => "Could not load server access list".to_owned(),
            Self::Server(_) => "Could not change server access".to_owned(),
        }
    }
}

/// What is wrong with an ACL, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclProblem {
    /// No server is allowed, so the room is unusable by everyone.
    NoServerAllowed,
    /// Our own server would be shut out of the room.
    OwnServerExcluded,
}

/// The ACL of a room that has no `m.room.server_acl` event: nothing is
/// restricted.
#[must_use]
pub fn unrestricted_acl() -> RoomServerAclEventContent {
    RoomServerAclEventContent::new(true, vec!["*".to_owned()], Vec::new())
}

/// Whether the two given ACLs would have the same effect.
#[must_use]
pub fn acls_are_equal(lhs: &RoomServerAclEventContent, rhs: &RoomServerAclEventContent) -> bool {
    lhs.allow_ip_literals == rhs.allow_ip_literals && lhs.allow == rhs.allow && lhs.deny == rhs.deny
}

/// Check the given ACL for the problems we warn about, from the worst down.
#[must_use]
pub fn check_acl(acl: &RoomServerAclEventContent, own_server: &ServerName) -> Option<AclProblem> {
    if acl.allow.is_empty() {
        return Some(AclProblem::NoServerAllowed);
    }

    if !acl.is_allowed(own_server) {
        return Some(AclProblem::OwnServerExcluded);
    }

    None
}

impl Room {
    /// The server ACL of this room, from the store.
    ///
    /// `None` means the room has no `m.room.server_acl` at all, which is
    /// not the same as one that allows nothing.
    pub async fn server_acl(&self) -> Result<Option<RoomServerAclEventContent>, ServerAclError> {
        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            matrix_room
                .get_state_event_static::<RoomServerAclEventContent>()
                .await
        });

        let raw_event = match handle.await.expect("task was not aborted") {
            Ok(Some(RawSyncOrStrippedState::Sync(raw_event))) => raw_event,
            // A room we were never in does not hand us its ACL.
            Ok(_) => return Ok(None),
            Err(read_error) => {
                error!("Could not get server ACL event: {read_error}");
                return Err(ServerAclError::Read(Box::new(read_error)));
            }
        };

        match raw_event.deserialize() {
            Ok(SyncStateEvent::Original(event)) => Ok(Some(event.content)),
            // A redacted event has no content. An ACL should never be
            // redacted, it is in the application's `NON_REDACTABLE_EVENTS`,
            // but a remote one might be.
            Ok(_) => Ok(None),
            Err(deserialize_error) => {
                error!("Could not deserialize server ACL event: {deserialize_error}");
                Err(ServerAclError::Read(Box::new(deserialize_error.into())))
            }
        }
    }

    /// Set the server ACL of this room.
    ///
    /// What the ACL may not be — see [`check_acl`] — is the page's to
    /// refuse or confirm before it gets here, as the application's is.
    pub async fn set_server_acl(
        &self,
        content: RoomServerAclEventContent,
    ) -> Result<(), ServerAclError> {
        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(content).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|send_error| {
                error!("Could not change server ACL: {send_error}");
                ServerAclError::Server(Box::new(send_error))
            })
    }
}

#[cfg(test)]
mod tests {
    use ruma::server_name;

    use super::*;

    #[test]
    fn an_empty_allow_list_is_the_fatal_problem() {
        let acl = RoomServerAclEventContent::new(true, Vec::new(), vec!["evil.example".to_owned()]);

        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::NoServerAllowed)
        );
    }

    #[test]
    fn an_empty_allow_list_wins_over_our_own_server() {
        // Both problems apply, and the fatal one must be the one reported,
        // because it is the one we refuse rather than confirm.
        let acl = RoomServerAclEventContent::new(true, Vec::new(), vec!["example.org".to_owned()]);

        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::NoServerAllowed)
        );
    }

    #[test]
    fn denying_our_own_server_is_a_warning() {
        let acl = RoomServerAclEventContent::new(
            true,
            vec!["*".to_owned()],
            vec!["example.org".to_owned()],
        );

        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::OwnServerExcluded)
        );
    }

    #[test]
    fn not_allowing_our_own_server_is_a_warning() {
        let acl =
            RoomServerAclEventContent::new(true, vec!["other.example".to_owned()], Vec::new());

        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::OwnServerExcluded)
        );
    }

    #[test]
    fn an_ip_literal_of_our_own_is_excluded_by_the_switch() {
        let acl = RoomServerAclEventContent::new(false, vec!["*".to_owned()], Vec::new());

        assert_eq!(
            check_acl(&acl, server_name!("1.1.1.1")),
            Some(AclProblem::OwnServerExcluded)
        );
        assert_eq!(check_acl(&acl, server_name!("example.org")), None);
    }

    #[test]
    fn a_wildcard_covers_our_own_subdomain() {
        let acl = RoomServerAclEventContent::new(
            false,
            vec!["*.example.org".to_owned()],
            vec!["evil.example".to_owned()],
        );

        assert_eq!(check_acl(&acl, server_name!("matrix.example.org")), None);
        assert_eq!(
            check_acl(&acl, server_name!("example.org")),
            Some(AclProblem::OwnServerExcluded)
        );
    }

    #[test]
    fn a_room_without_an_acl_is_not_restricted() {
        let acl = unrestricted_acl();

        assert_eq!(check_acl(&acl, server_name!("example.org")), None);
        assert_eq!(check_acl(&acl, server_name!("1.1.1.1")), None);
    }

    #[test]
    fn the_unrestricted_baseline_is_not_a_change() {
        // Opening the page on a room with no ACL must not offer to save
        // anything, otherwise every visit would write an event.
        assert!(acls_are_equal(&unrestricted_acl(), &unrestricted_acl()));

        let with_a_deny = RoomServerAclEventContent::new(
            true,
            vec!["*".to_owned()],
            vec!["evil.example".to_owned()],
        );
        assert!(!acls_are_equal(&with_a_deny, &unrestricted_acl()));
    }

    #[test]
    fn the_ip_literal_switch_alone_is_a_change() {
        let mut acl = unrestricted_acl();
        acl.allow_ip_literals = false;

        assert!(!acls_are_equal(&acl, &unrestricted_acl()));
    }
}

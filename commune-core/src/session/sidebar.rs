//! The names of the sidebar's sections.
//!
//! Lifted from the application's `session/sidebar_data/section/name.rs`
//! because the per-session settings persist which sections are expanded.
//! The conversions to and from `RoomCategory` follow when the room
//! categorization rules are extracted.

use serde::{Deserialize, Serialize};

/// The name of a section of the sidebar.
#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SidebarSectionName {
    /// The section for verification requests.
    VerificationRequest,
    /// The section for invite requests.
    InviteRequest,
    /// The section for room invites.
    Invited,
    /// The section for the server notices room.
    ServerNotice,
    /// The section for spaces.
    Space,
    /// The section for favorite rooms.
    Favorite,
    /// The section for joined rooms without a tag.
    #[default]
    Normal,
    /// The section for low-priority rooms.
    LowPriority,
    /// The section for room that were left.
    Left,
}

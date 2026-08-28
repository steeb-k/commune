//! The names of the sidebar's sections, and how categories map onto them.
//!
//! Lifted from the application's `session/sidebar_data/section/name.rs`.
//! The GTK sidebar derives its sections through a tower of
//! `SortListModel`s/`FilterListModel`s; headless, the mapping between a
//! room's category and its section is the whole of the sectioning rules,
//! and grouping and ordering are the list consumer's three lines of code.

use serde::{Deserialize, Serialize};

use super::{RoomCategory, TargetRoomCategory};

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

impl SidebarSectionName {
    /// Convert the given `RoomCategory` to a `SidebarSectionName`, if
    /// possible.
    #[must_use]
    pub fn from_room_category(category: RoomCategory) -> Option<Self> {
        let name = match category {
            RoomCategory::Knocked => Self::InviteRequest,
            RoomCategory::Invited => Self::Invited,
            RoomCategory::ServerNotice => Self::ServerNotice,
            RoomCategory::Space => Self::Space,
            RoomCategory::Favorite => Self::Favorite,
            RoomCategory::Normal => Self::Normal,
            RoomCategory::LowPriority => Self::LowPriority,
            RoomCategory::Left => Self::Left,
            RoomCategory::Outdated | RoomCategory::Ignored => return None,
        };

        Some(name)
    }

    /// Convert this `SidebarSectionName` to a `RoomCategory`, if possible.
    #[must_use]
    pub fn into_room_category(self) -> Option<RoomCategory> {
        let category = match self {
            Self::VerificationRequest => return None,
            Self::InviteRequest => RoomCategory::Knocked,
            Self::Invited => RoomCategory::Invited,
            Self::ServerNotice => RoomCategory::ServerNotice,
            Self::Space => RoomCategory::Space,
            Self::Favorite => RoomCategory::Favorite,
            Self::Normal => RoomCategory::Normal,
            Self::LowPriority => RoomCategory::LowPriority,
            Self::Left => RoomCategory::Left,
        };

        Some(category)
    }

    /// Convert this `SidebarSectionName` to a `TargetRoomCategory`, if
    /// possible.
    #[must_use]
    pub fn into_target_room_category(self) -> Option<TargetRoomCategory> {
        let category = match self {
            // A space is joined and left like any other room, but none of the
            // tags apply to it, so it is never a drag-n-drop target.
            Self::VerificationRequest
            | Self::InviteRequest
            | Self::Invited
            | Self::ServerNotice
            | Self::Space => return None,
            Self::Favorite => TargetRoomCategory::Favorite,
            Self::Normal => TargetRoomCategory::Normal,
            Self::LowPriority => TargetRoomCategory::LowPriority,
            Self::Left => TargetRoomCategory::Left,
        };

        Some(category)
    }
}

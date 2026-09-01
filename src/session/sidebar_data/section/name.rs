use std::fmt;

use gettextrs::gettext;
use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::session::{RoomCategory, TargetRoomCategory};

/// The possible names of the sections in the sidebar.
#[derive(
    Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, glib::Enum, Serialize, Deserialize,
)]
#[enum_type(name = "SidebarSectionName")]
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
    /// Convert the given `RoomCategory` to a `SidebarSectionName`, if possible.
    pub(crate) fn from_room_category(category: RoomCategory) -> Option<Self> {
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
    pub(crate) fn into_room_category(self) -> Option<RoomCategory> {
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
    pub(crate) fn into_target_room_category(self) -> Option<TargetRoomCategory> {
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

impl fmt::Display for SidebarSectionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            SidebarSectionName::VerificationRequest => gettext("Verifications"),
            SidebarSectionName::InviteRequest => gettext("Access Requests"),
            SidebarSectionName::Invited => gettext("Invited"),
            SidebarSectionName::ServerNotice => gettext("Server Notices"),
            // Translators: A space is a collection of rooms, presented as a
            // folder-like room that other rooms belong to.
            SidebarSectionName::Space => gettext("Spaces"),
            SidebarSectionName::Favorite => gettext("Favorites"),
            SidebarSectionName::Normal => gettext("Rooms"),
            SidebarSectionName::LowPriority => gettext("Low Priority"),
            SidebarSectionName::Left => gettext("Historical"),
        };
        f.write_str(&label)
    }
}

/// The core's name for the same section.
///
/// The two enums have the same variants and the same kebab-case
/// serialization — the core's is this one with the `glib::Enum` derive and
/// the translated `Display` taken off. They stay separate until `room_list/`
/// moves, because this one is a `GObject` property type and `RoomCategory`,
/// which its two conversions need, is one too.
impl From<SidebarSectionName> for commune_core::session::SidebarSectionName {
    fn from(value: SidebarSectionName) -> Self {
        match value {
            SidebarSectionName::VerificationRequest => Self::VerificationRequest,
            SidebarSectionName::InviteRequest => Self::InviteRequest,
            SidebarSectionName::Invited => Self::Invited,
            SidebarSectionName::ServerNotice => Self::ServerNotice,
            SidebarSectionName::Space => Self::Space,
            SidebarSectionName::Favorite => Self::Favorite,
            SidebarSectionName::Normal => Self::Normal,
            SidebarSectionName::LowPriority => Self::LowPriority,
            SidebarSectionName::Left => Self::Left,
        }
    }
}

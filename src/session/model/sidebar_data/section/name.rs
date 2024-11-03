use std::fmt;

use gettextrs::gettext;
use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::session::model::RoomCategory;

/// The possible names of the sections in the sidebar.
#[derive(
    Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, glib::Enum, Serialize, Deserialize,
)]
#[enum_type(name = "SidebarSectionName")]
#[serde(rename_all = "kebab-case")]
pub enum SidebarSectionName {
    /// The section for verification requests.
    VerificationRequest,
    /// The section for room invites.
    Invited,
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
    pub fn from_room_category(category: RoomCategory) -> Option<Self> {
        let name = match category {
            RoomCategory::Invited => Self::Invited,
            RoomCategory::Favorite => Self::Favorite,
            RoomCategory::Normal => Self::Normal,
            RoomCategory::LowPriority => Self::LowPriority,
            RoomCategory::Left => Self::Left,
            RoomCategory::Outdated | RoomCategory::Space | RoomCategory::Ignored => return None,
        };

        Some(name)
    }

    /// Convert this `SidebarSectionName` to a `RoomCategory`, if possible.
    pub fn into_room_category(self) -> Option<RoomCategory> {
        let category = match self {
            Self::VerificationRequest => return None,
            Self::Invited => RoomCategory::Invited,
            Self::Favorite => RoomCategory::Favorite,
            Self::Normal => RoomCategory::Normal,
            Self::LowPriority => RoomCategory::LowPriority,
            Self::Left => RoomCategory::Left,
        };

        Some(category)
    }
}

impl fmt::Display for SidebarSectionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            SidebarSectionName::VerificationRequest => gettext("Verifications"),
            SidebarSectionName::Invited => gettext("Invited"),
            SidebarSectionName::Favorite => gettext("Favorites"),
            SidebarSectionName::Normal => gettext("Rooms"),
            SidebarSectionName::LowPriority => gettext("Low Priority"),
            SidebarSectionName::Left => gettext("Historical"),
        };
        f.write_str(&label)
    }
}

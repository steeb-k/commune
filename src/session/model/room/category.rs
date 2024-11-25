use std::fmt;

use gtk::glib;
use matrix_sdk::RoomState;

use crate::session::model::SidebarSectionName;

/// The category of a room.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "RoomCategory")]
pub enum RoomCategory {
    /// The user was invited to the room.
    Invited,
    /// The room is joined and has the `m.favourite` tag.
    Favorite,
    /// The room is joined and has no known tag.
    #[default]
    Normal,
    /// The room is joined and has the `m.lowpriority` tag.
    LowPriority,
    /// The room was left by the user, or they were kicked or banned.
    Left,
    /// The room was upgraded and their successor was joined.
    Outdated,
    /// The room is a space.
    Space,
    /// The room should be ignored.
    ///
    /// According to the Matrix specification, invites from ignored users
    /// should be ignored.
    Ignored,
}

impl RoomCategory {
    /// Check whether this `RoomCategory` can be changed to the given category.
    pub(crate) fn can_change_to(self, category: Self) -> bool {
        match self {
            Self::Invited => {
                matches!(
                    category,
                    Self::Favorite | Self::Normal | Self::LowPriority | Self::Left
                )
            }
            Self::Favorite => {
                matches!(category, Self::Normal | Self::LowPriority | Self::Left)
            }
            Self::Normal => {
                matches!(category, Self::Favorite | Self::LowPriority | Self::Left)
            }
            Self::LowPriority => {
                matches!(category, Self::Favorite | Self::Normal | Self::Left)
            }
            Self::Left => {
                matches!(category, Self::Favorite | Self::Normal | Self::LowPriority)
            }
            Self::Ignored | Self::Outdated | Self::Space => false,
        }
    }

    /// Whether this `RoomCategory` corresponds to the given state.
    pub(crate) fn is_state(self, state: RoomState) -> bool {
        match self {
            RoomCategory::Invited | RoomCategory::Ignored => state == RoomState::Invited,
            RoomCategory::Favorite
            | RoomCategory::Normal
            | RoomCategory::LowPriority
            | RoomCategory::Outdated
            | RoomCategory::Space => state == RoomState::Joined,
            RoomCategory::Left => state == RoomState::Left,
        }
    }
}

impl fmt::Display for RoomCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(section_name) = SidebarSectionName::from_room_category(*self) else {
            unimplemented!();
        };

        section_name.fmt(f)
    }
}

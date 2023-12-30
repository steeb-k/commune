use std::fmt;

use gtk::glib;
use matrix_sdk::RoomState;

use crate::session::model::CategoryType;

// TODO: do we also want custom tags support?
// See https://spec.matrix.org/v1.2/client-server-api/#room-tagging
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "RoomType")]
pub enum RoomType {
    /// The user was invited to the room.
    Invited = 0,
    /// The room is joined and has the `m.favourite` tag.
    Favorite = 1,
    /// The room is joined and has no known tag.
    #[default]
    Normal = 2,
    /// The room is joined and has the `m.lowpriority` tag.
    LowPriority = 3,
    /// The room was left by the user, or they were kicked or banned.
    Left = 4,
    /// The room was upgraded and their successor was joined.
    Outdated = 5,
    /// The room is a space.
    Space = 6,
    /// The room should be ignored.
    ///
    /// According to the Matrix specification, invites from ignored users
    /// should be ignored.
    Ignored = 7,
}

impl RoomType {
    /// Check whether this `RoomType` can be changed to `category`.
    pub fn can_change_to(&self, category: RoomType) -> bool {
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

    /// Whether this `RoomType` corresponds to the given state.
    pub fn is_state(&self, state: RoomState) -> bool {
        match self {
            RoomType::Invited | RoomType::Ignored => state == RoomState::Invited,
            RoomType::Favorite
            | RoomType::Normal
            | RoomType::LowPriority
            | RoomType::Outdated
            | RoomType::Space => state == RoomState::Joined,
            RoomType::Left => state == RoomState::Left,
        }
    }
}

impl fmt::Display for RoomType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        CategoryType::from(self).fmt(f)
    }
}

impl TryFrom<CategoryType> for RoomType {
    type Error = &'static str;

    fn try_from(category_type: CategoryType) -> Result<Self, Self::Error> {
        Self::try_from(&category_type)
    }
}

impl TryFrom<&CategoryType> for RoomType {
    type Error = &'static str;

    fn try_from(category_type: &CategoryType) -> Result<Self, Self::Error> {
        match category_type {
            CategoryType::None => Err("CategoryType::None cannot be a RoomType"),
            CategoryType::Invited => Ok(Self::Invited),
            CategoryType::Favorite => Ok(Self::Favorite),
            CategoryType::Normal => Ok(Self::Normal),
            CategoryType::LowPriority => Ok(Self::LowPriority),
            CategoryType::Left => Ok(Self::Left),
            CategoryType::Outdated => Ok(Self::Outdated),
            CategoryType::VerificationRequest => {
                Err("CategoryType::VerificationRequest cannot be a RoomType")
            }
            CategoryType::Space => Ok(Self::Space),
            CategoryType::Ignored => Ok(Self::Ignored),
        }
    }
}

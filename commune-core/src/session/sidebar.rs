//! The names of the sidebar's sections, and how categories map onto them.
//!
//! Lifted from the application's `session/sidebar_data/`. The GTK sidebar
//! derives its sections through a tower of `SortListModel`s and
//! `FilterListModel`s; headless, the rules are these: the mapping between
//! a room's category and its section, the order of the top-level rows,
//! and which rows show while a room is being dragged to a new section.
//! Grouping and ordering are the list consumer's three lines of code.

use serde::{Deserialize, Serialize};

use super::{RoomCategory, TargetRoomCategory};

/// The name of a section of the sidebar.
#[derive(
    Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Serialize, Deserialize,
)]
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

impl SidebarSectionName {
    /// Every section, in the order the sidebar shows them.
    pub const ALL: [Self; 9] = [
        Self::VerificationRequest,
        Self::InviteRequest,
        Self::Invited,
        Self::ServerNotice,
        Self::Space,
        Self::Favorite,
        Self::Normal,
        Self::LowPriority,
        Self::Left,
    ];

    /// Whether a room of the given category may be dropped on this section.
    #[must_use]
    pub fn is_drop_target_for(self, source: RoomCategory) -> bool {
        self.into_target_room_category()
            .is_some_and(|target| source.can_change_to(target))
    }

    /// Whether this section is shown, given whether it is empty and the
    /// category of the room being dragged, if one is.
    ///
    /// A section with rooms in it is always shown; an empty one only while
    /// a room that could land in it is being dragged.
    #[must_use]
    pub fn is_visible(self, is_empty: bool, dragged: Option<RoomCategory>) -> bool {
        !is_empty || dragged.is_some_and(|source| self.is_drop_target_for(source))
    }
}

/// The two rows of the sidebar that are not sections.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy)]
pub enum SidebarIconItemKind {
    /// The explore view.
    #[default]
    Explore,
    /// An action to forget a room.
    Forget,
}

impl SidebarIconItemKind {
    /// Whether this row is shown, given the category of the room being
    /// dragged, if one is.
    ///
    /// Exploring is always offered; forgetting only to a room that was left.
    #[must_use]
    pub fn is_visible(self, dragged: Option<RoomCategory>) -> bool {
        match self {
            Self::Explore => true,
            Self::Forget => dragged == Some(RoomCategory::Left),
        }
    }
}

/// A top-level row of the sidebar.
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub enum SidebarItemKind {
    /// A row with an icon.
    Icon(SidebarIconItemKind),
    /// A section, holding rooms or verifications.
    Section(SidebarSectionName),
}

/// The top-level rows of the sidebar, in order: explore, every section,
/// forget.
pub const SIDEBAR_ITEMS: [SidebarItemKind; 11] = [
    SidebarItemKind::Icon(SidebarIconItemKind::Explore),
    SidebarItemKind::Section(SidebarSectionName::VerificationRequest),
    SidebarItemKind::Section(SidebarSectionName::InviteRequest),
    SidebarItemKind::Section(SidebarSectionName::Invited),
    SidebarItemKind::Section(SidebarSectionName::ServerNotice),
    SidebarItemKind::Section(SidebarSectionName::Space),
    SidebarItemKind::Section(SidebarSectionName::Favorite),
    SidebarItemKind::Section(SidebarSectionName::Normal),
    SidebarItemKind::Section(SidebarSectionName::LowPriority),
    SidebarItemKind::Section(SidebarSectionName::Left),
    SidebarItemKind::Icon(SidebarIconItemKind::Forget),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_are_the_explore_row_the_sections_and_the_forget_row() {
        assert_eq!(SIDEBAR_ITEMS.len(), SidebarSectionName::ALL.len() + 2);
        assert!(matches!(
            SIDEBAR_ITEMS[0],
            SidebarItemKind::Icon(SidebarIconItemKind::Explore)
        ));
        assert!(matches!(
            SIDEBAR_ITEMS[10],
            SidebarItemKind::Icon(SidebarIconItemKind::Forget)
        ));
        for (item, name) in SIDEBAR_ITEMS[1..10].iter().zip(SidebarSectionName::ALL) {
            assert_eq!(*item, SidebarItemKind::Section(name));
        }
    }

    #[test]
    fn an_empty_section_shows_only_for_a_room_that_could_land_in_it() {
        assert!(SidebarSectionName::Favorite.is_visible(false, None));
        assert!(!SidebarSectionName::Favorite.is_visible(true, None));
        assert!(SidebarSectionName::Favorite.is_visible(true, Some(RoomCategory::Normal)));
        // A space is never a drop target.
        assert!(!SidebarSectionName::Space.is_visible(true, Some(RoomCategory::Normal)));
    }

    #[test]
    fn forgetting_is_offered_to_a_room_that_was_left() {
        assert!(SidebarIconItemKind::Explore.is_visible(None));
        assert!(!SidebarIconItemKind::Forget.is_visible(None));
        assert!(!SidebarIconItemKind::Forget.is_visible(Some(RoomCategory::Normal)));
        assert!(SidebarIconItemKind::Forget.is_visible(Some(RoomCategory::Left)));
    }
}

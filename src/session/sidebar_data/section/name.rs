use std::fmt;

use commune_core::session::SidebarSectionName as CoreSidebarSectionName;
use gettextrs::gettext;
use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::session::RoomCategory;

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
        CoreSidebarSectionName::from_room_category(category.into()).map(Into::into)
    }

    /// Convert this `SidebarSectionName` to a `RoomCategory`, if possible.
    pub(crate) fn into_room_category(self) -> Option<RoomCategory> {
        CoreSidebarSectionName::from(self)
            .into_room_category()
            .map(Into::into)
    }

    /// Whether this section is shown, given whether it is empty and the
    /// category of the room being dragged, if one is.
    pub(crate) fn is_visible(self, is_empty: bool, dragged: Option<RoomCategory>) -> bool {
        CoreSidebarSectionName::from(self).is_visible(is_empty, dragged.map(Into::into))
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
/// the translated `Display` taken off; this one is a `GObject` property
/// type, and every rule about it is the core's.
impl From<CoreSidebarSectionName> for SidebarSectionName {
    fn from(value: CoreSidebarSectionName) -> Self {
        match value {
            CoreSidebarSectionName::VerificationRequest => Self::VerificationRequest,
            CoreSidebarSectionName::InviteRequest => Self::InviteRequest,
            CoreSidebarSectionName::Invited => Self::Invited,
            CoreSidebarSectionName::ServerNotice => Self::ServerNotice,
            CoreSidebarSectionName::Space => Self::Space,
            CoreSidebarSectionName::Favorite => Self::Favorite,
            CoreSidebarSectionName::Normal => Self::Normal,
            CoreSidebarSectionName::LowPriority => Self::LowPriority,
            CoreSidebarSectionName::Left => Self::Left,
        }
    }
}

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

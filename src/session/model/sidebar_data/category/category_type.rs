use std::fmt;

use gettextrs::gettext;
use gtk::glib;

use crate::session::model::RoomCategory;

#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(i32)]
#[enum_type(name = "CategoryType")]
pub enum CategoryType {
    #[default]
    None = -1,
    VerificationRequest = 0,
    Invited = 1,
    Favorite = 2,
    Normal = 3,
    LowPriority = 4,
    Left = 5,
    Outdated = 6,
    Space = 7,
    Ignored = 8,
}

impl fmt::Display for CategoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            CategoryType::None => unimplemented!(),
            CategoryType::VerificationRequest => gettext("Verifications"),
            CategoryType::Invited => gettext("Invited"),
            CategoryType::Favorite => gettext("Favorites"),
            CategoryType::Normal => gettext("Rooms"),
            CategoryType::LowPriority => gettext("Low Priority"),
            CategoryType::Left => gettext("Historical"),
            // These categories are hidden.
            CategoryType::Outdated | CategoryType::Space | CategoryType::Ignored => {
                unimplemented!()
            }
        };
        f.write_str(&label)
    }
}

impl From<RoomCategory> for CategoryType {
    fn from(category: RoomCategory) -> Self {
        Self::from(&category)
    }
}

impl From<&RoomCategory> for CategoryType {
    fn from(category: &RoomCategory) -> Self {
        match category {
            RoomCategory::Invited => Self::Invited,
            RoomCategory::Favorite => Self::Favorite,
            RoomCategory::Normal => Self::Normal,
            RoomCategory::LowPriority => Self::LowPriority,
            RoomCategory::Left => Self::Left,
            RoomCategory::Outdated => Self::Outdated,
            RoomCategory::Space => Self::Space,
            RoomCategory::Ignored => Self::Ignored,
        }
    }
}

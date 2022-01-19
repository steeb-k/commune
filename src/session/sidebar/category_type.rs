use crate::session::room::RoomType;
use gettextrs::gettext;
use gtk::glib;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(i32)]
#[enum_type(name = "CategoryType")]
pub enum CategoryType {
    None = -1,
    VerificationRequest = 0,
    Invited = 1,
    Favorite = 2,
    Normal = 3,
    LowPriority = 4,
    Left = 5,
    Outdated = 6,
}

impl Default for CategoryType {
    fn default() -> Self {
        CategoryType::Normal
    }
}

impl ToString for CategoryType {
    fn to_string(&self) -> String {
        match self {
            CategoryType::None => unimplemented!(),
            CategoryType::VerificationRequest => gettext("Verifications"),
            CategoryType::Invited => gettext("Invited"),
            CategoryType::Favorite => gettext("Favorite"),
            CategoryType::Normal => gettext("Rooms"),
            CategoryType::LowPriority => gettext("Low Priority"),
            CategoryType::Left => gettext("Historical"),
            // Translators: This shouldn't ever be visible to the user,
            CategoryType::Outdated => gettext("Outdated"),
        }
    }
}

impl From<RoomType> for CategoryType {
    fn from(room_type: RoomType) -> Self {
        match room_type {
            RoomType::Invited => Self::Invited,
            RoomType::Favorite => Self::Favorite,
            RoomType::Normal => Self::Normal,
            RoomType::LowPriority => Self::LowPriority,
            RoomType::Left => Self::Left,
            RoomType::Outdated => Self::Outdated,
        }
    }
}

impl From<&RoomType> for CategoryType {
    fn from(room_type: &RoomType) -> Self {
        match room_type {
            RoomType::Invited => Self::Invited,
            RoomType::Favorite => Self::Favorite,
            RoomType::Normal => Self::Normal,
            RoomType::LowPriority => Self::LowPriority,
            RoomType::Left => Self::Left,
            RoomType::Outdated => Self::Outdated,
        }
    }
}

use crate::session::room::RoomType;
use gettextrs::gettext;
use gtk::glib;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "CategoryType")]
pub enum CategoryType {
    VerificationRequest = 0,
    Invited = 1,
    Favorite = 2,
    Normal = 3,
    LowPriority = 4,
    Left = 5,
}

impl Default for CategoryType {
    fn default() -> Self {
        CategoryType::Normal
    }
}

impl ToString for CategoryType {
    fn to_string(&self) -> String {
        match self {
            CategoryType::VerificationRequest => gettext("Login Requests"),
            CategoryType::Invited => gettext("Invited"),
            CategoryType::Favorite => gettext("Favorite"),
            CategoryType::Normal => gettext("Rooms"),
            CategoryType::LowPriority => gettext("Low Priority"),
            CategoryType::Left => gettext("Historical"),
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
        }
    }
}

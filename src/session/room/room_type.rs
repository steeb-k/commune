use crate::session::sidebar::CategoryType;
use gtk::glib;

// TODO: do we also want the category `People` and a custom category support?
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "RoomType")]
pub enum RoomType {
    Invited = 0,
    Favorite = 1,
    Normal = 2,
    LowPriority = 3,
    Left = 4,
    Outdated = 5,
}

impl Default for RoomType {
    fn default() -> Self {
        RoomType::Normal
    }
}

impl ToString for RoomType {
    fn to_string(&self) -> String {
        CategoryType::from(self).to_string()
    }
}

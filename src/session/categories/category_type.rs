use gettextrs::gettext;
use gtk::glib;

// TODO: do we also want the categorie `People` and a custom categorie support?
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "CategoryType")]
pub enum CategoryType {
    Invited = 0,
    Favorite = 1,
    Normal = 2,
    LowPriority = 3,
    Left = 4,
}

impl Default for CategoryType {
    fn default() -> Self {
        CategoryType::Normal
    }
}

impl ToString for CategoryType {
    fn to_string(&self) -> String {
        match self {
            CategoryType::Invited => gettext("Invited"),
            CategoryType::Favorite => gettext("Favorite"),
            CategoryType::Normal => gettext("Rooms"),
            CategoryType::LowPriority => gettext("Low Priority"),
            CategoryType::Left => gettext("Historical"),
        }
    }
}

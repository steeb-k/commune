use gettextrs::gettext;
use gtk::glib;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "ContentType")]
pub enum ContentType {
    None = 0,
    Explore = 1,
    Room = 2,
}

impl Default for ContentType {
    fn default() -> Self {
        ContentType::None
    }
}

impl ToString for ContentType {
    fn to_string(&self) -> String {
        match self {
            ContentType::None => gettext("No selection"),
            ContentType::Explore => gettext("Explore"),
            ContentType::Room => gettext("Room"),
        }
    }
}

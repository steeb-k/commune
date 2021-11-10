use gettextrs::gettext;
use gtk::glib;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "EntryType")]
pub enum EntryType {
    Explore = 0,
}

impl Default for EntryType {
    fn default() -> Self {
        EntryType::Explore
    }
}

impl ToString for EntryType {
    fn to_string(&self) -> String {
        match self {
            EntryType::Explore => gettext("Explore"),
        }
    }
}

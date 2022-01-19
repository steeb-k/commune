use gettextrs::gettext;
use gtk::glib;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "EntryType")]
pub enum EntryType {
    Explore = 0,
    Forget = 1,
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
            EntryType::Forget => gettext("Forget Room"),
        }
    }
}

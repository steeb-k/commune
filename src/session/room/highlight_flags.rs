use gtk::glib;

#[glib::flags(name = "HighlightFlags")]
pub enum HighlightFlags {
    HIGHLIGHT = 0b0000_0001,
    BOLD = 0b0000_0010,
}

impl Default for HighlightFlags {
    fn default() -> Self {
        HighlightFlags::empty()
    }
}

impl From<commune_core::session::RoomHighlight> for HighlightFlags {
    fn from(highlight: commune_core::session::RoomHighlight) -> Self {
        use commune_core::session::RoomHighlight;

        match highlight {
            RoomHighlight::None => Self::empty(),
            RoomHighlight::Bold => Self::BOLD,
            RoomHighlight::Highlight => Self::all(),
        }
    }
}

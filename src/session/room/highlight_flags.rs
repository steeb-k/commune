use gtk::glib;

#[glib::flags(name = "HighlightFlags")]
pub enum HighlightFlags {
    #[flags_value(name = "NONE")]
    NONE = 0b00000000,
    #[flags_value(name = "HIGHLIGHT")]
    HIGHLIGHT = 0b00000001,
    #[flags_value(name = "BOLD")]
    BOLD = 0b00000010,
    #[flags_value(skip)]
    HIGHLIGHT_BOLD = Self::HIGHLIGHT.bits() | Self::BOLD.bits(),
}

impl Default for HighlightFlags {
    fn default() -> Self {
        HighlightFlags::NONE
    }
}

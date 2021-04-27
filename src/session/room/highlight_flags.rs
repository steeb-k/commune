use gtk::glib;

#[glib::gflags("HighlightFlags")]
pub enum HighlightFlags {
    #[glib::gflags(name = "NONE")]
    NONE = 0b00000000,
    #[glib::gflags(name = "HIGHLIGHT")]
    HIGHLIGHT = 0b00000001,
    #[glib::gflags(name = "BOLD")]
    BOLD = 0b00000010,
    #[glib::gflags(skip)]
    HIGHLIGHT_BOLD = Self::HIGHLIGHT.bits() | Self::BOLD.bits(),
}

impl Default for HighlightFlags {
    fn default() -> Self {
        HighlightFlags::NONE
    }
}

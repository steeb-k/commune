//! The application's seam onto [`commune_core::klipy`].
//!
//! The client itself — the endpoints, the sponsored-item filtering, the
//! variant picking, the size ceiling — is the core's. What is left here is
//! the same two things every leaf of Track 3 leaves behind: `glib`, and
//! `gettext`.
//!
//! **The wrappers.** [`Gif`] is a property of the button that presents it and
//! [`SelectedGif`] travels through a signal, so both have to be
//! `glib::Boxed`, which the core's structs cannot derive. Each is a newtype
//! with the derive on it and a `Deref` through to the core's.
//!
//! **The one sentence.** The API does not guarantee a title, and the fallback
//! for a GIF that has none is a word a person reads: it becomes the body of
//! the sticker event, which is what a client with no image support shows and
//! what a screen reader announces. The core returns bare English there.
//! [`Gif::title()`] below is the translated one, and it shadows the core's
//! method rather than reaching it through `Deref`, so a caller cannot pick up
//! the wrong one by accident.

use std::ops::Deref;

// Only what the application names. `GifFile` and `MAX_SEND_FILESIZE` are not
// re-exported: the widgets reach a file's fields through `Gif::preview()` and
// the size ceiling is only ever applied inside the core.
pub(crate) use commune_core::klipy::{KlipyError, is_available, report_share};
use gettextrs::gettext;
use gtk::glib;

/// A single GIF.
///
/// This is `pub` rather than `pub(crate)` so that it can be a property of a
/// widget; the module itself is `pub(crate)`, so nothing escapes the crate.
#[derive(Debug, Clone, glib::Boxed)]
#[boxed_type(name = "KlipyGif")]
pub struct Gif(commune_core::klipy::Gif);

impl Gif {
    /// A human-readable description of the GIF, used as the alt text and the
    /// fallback body of the event.
    ///
    /// The API does not guarantee a title. Shadows the core's method of the
    /// same name, which returns the same word untranslated for an embedder
    /// that has no catalog.
    pub(crate) fn title(&self) -> String {
        if self.0.title.is_empty() {
            gettext("GIF")
        } else {
            self.0.title.clone()
        }
    }

    /// The data needed to send this GIF, if it has a variant that can be
    /// sent.
    ///
    /// The core builds this and then the title is replaced with the
    /// translated one: it is the body the room receives, so it has to be in
    /// the sender's language rather than the core's English.
    pub(crate) fn to_selection(&self) -> Option<SelectedGif> {
        let mut selection = self.0.to_selection()?;
        selection.title = self.title();

        Some(SelectedGif(selection))
    }
}

impl Deref for Gif {
    type Target = commune_core::klipy::Gif;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A GIF that the user chose to send.
///
/// This is what the picker hands to the composer.
#[derive(Debug, Clone, glib::Boxed)]
#[boxed_type(name = "SelectedGif")]
pub(crate) struct SelectedGif(commune_core::klipy::SelectedGif);

impl SelectedGif {
    /// The core's selection inside this wrapper.
    ///
    /// Needed where the fields are moved out of it and `Deref` can only lend
    /// them.
    pub(crate) fn into_inner(self) -> commune_core::klipy::SelectedGif {
        self.0
    }
}

impl Deref for SelectedGif {
    type Target = commune_core::klipy::SelectedGif;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A page of GIFs.
///
/// The application's own rather than the core's, only because its `gifs` are
/// the wrapper above.
pub(crate) struct GifPage {
    /// The GIFs of this page, with the sponsored items removed.
    pub(crate) gifs: Vec<Gif>,
    /// Whether another page can be requested.
    pub(crate) has_next: bool,
}

/// Search the GIFs matching the given query.
///
/// Pages are 1-indexed.
pub(crate) async fn search(query: &str, page: u32) -> Result<GifPage, KlipyError> {
    let page = commune_core::klipy::search(query, page).await?;

    Ok(GifPage {
        gifs: page.gifs.into_iter().map(Gif).collect(),
        has_next: page.has_next,
    })
}

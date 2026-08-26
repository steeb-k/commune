//! The image decoder backend.
//!
//! Image decoding is the one part of the media stack that is not portable.
//! Linux decodes with [glycin], which runs format-specific loaders in a
//! sandboxed subprocess; glycin is a set of C libraries plus D-Bus and Flatpak
//! machinery that only exists there. Everywhere else we decode in-process with
//! the pure-Rust `image` crate.
//!
//! Both backends expose the same small API, so callers never mention either by
//! name:
//!
//! * [`Loader`] takes bytes or a file and produces an [`Image`].
//! * [`Image`] knows its natural dimensions and produces [`Frame`]s, either the
//!   next one in sequence or one scaled to a [`FrameRequest`].
//! * [`Frame`] is a [`gdk::Texture`] plus, if the image is animated, how long
//!   to show it for.
//! * [`Error`] reports whether the failure was an unrecognised format, which is
//!   the one case the UI distinguishes.
//!
//! The backends are not identical in what they can decode. glycin handles SVG,
//! HEIC, AVIF and JXL natively; the `image` backend covers the common raster
//! formats itself and hands the rest to `GdkPixbuf`, so its reach depends on
//! which loaders the platform ships. See [`image_rs`](self::image_rs) for the
//! current list.
//!
//! [glycin]: https://gitlab.gnome.org/GNOME/glycin

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(target_os = "linux")] {
        mod glycin;

        pub(crate) use self::glycin::{Error, Frame, FrameRequest, Image, Loader};
    } else {
        mod image_rs;

        pub(crate) use self::image_rs::{Error, Frame, FrameRequest, Image, Loader};
    }
}

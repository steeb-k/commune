//! SVG on Android, drawn by GTK itself.
//!
//! Android is the one platform in this port where nothing else will draw one.
//! The `image` crate has never read SVG. The `GdkPixbuf` fallback needs
//! librsvg's loader module, and there is no librsvg in the APK — `rsvg` is
//! named in pixiewood's dependency list but nothing resolves the wrap, and
//! gdk-pixbuf here has no module directory to find one in either.
//!
//! It does not need librsvg. GTK grew a public SVG renderer of its own in
//! 4.22 — [`gtk::Svg`], a [`gtk::gdk::Paintable`] implementing much of SVG 2 —
//! and it is already linked: `nm -D` on this build's `libgtk-4.so` finds 38
//! `gtk_svg_*` symbols. It is the same renderer that draws the symbolic icons.
//!
//! Two consequences of it being a GTK object rather than a decoder:
//!
//! * **This runs on the main thread.** Everything else in this backend decodes
//!   on the Tokio blocking pool, and a `GtkSvg` cannot go there. So the sniff
//!   and the render happen in [`Loader::load`](super::Loader::load) before the
//!   hop, which is safe because every image request is spawned on the main
//!   context (`utils::media::image::queue`).
//! * **The result is a still.** `GtkSvg` can animate, given a frame clock, and
//!   this asks it for exactly one picture. An animated SVG renders as its first
//!   frame rather than as an error, which is the trade this whole fallback path
//!   makes.

use std::sync::{Arc, OnceLock};

use gtk::{cairo, glib, prelude::*};

use super::{FrameSource, Image, ImageInner, RawFrame};

/// What to draw an SVG at when it declares no size of its own, only a
/// `viewBox`.
///
/// Anything downstream that wants it smaller will scale it; this is only the
/// size the vector is rasterised at once.
const DEFAULT_EXTENT: f64 = 512.0;

/// The largest either axis is rasterised at.
///
/// A declared size is a number in a file we did not write, and four bytes per
/// pixel of it are about to be allocated on a phone.
const MAX_EXTENT: i32 = 2048;

/// Whether the given bytes look enough like an SVG document to be worth
/// handing to GTK.
///
/// Deliberately a glance and not a parse. A false positive costs one failed
/// render and then falls through to the ordinary path, which is what would
/// have happened anyway.
pub(super) fn looks_like_svg(data: &[u8]) -> bool {
    /// Enough room for an XML declaration, a doctype and a comment or two
    /// ahead of the root element.
    const SNIFF: usize = 1024;

    let head = &data[..data.len().min(SNIFF)];

    // An SVG is XML, so the first thing that is not whitespace opens a tag.
    // This rules out the binary formats without reading them.
    let Some(first) = head.iter().find(|byte| !byte.is_ascii_whitespace()) else {
        return false;
    };
    if *first != b'<' {
        return false;
    }

    head.windows(4).any(|window| window == b"<svg")
}

/// Render an SVG to a single frame, or give up quietly.
///
/// Every failure here means the same thing to the caller — this was not an SVG
/// we could draw — and the caller's next move is the ordinary decode path,
/// which will report the unknown format the UI knows how to show. So there is
/// nothing to distinguish and nothing to log.
pub(super) fn probe(data: &Arc<[u8]>) -> Option<Image> {
    let svg = gtk::Svg::from_bytes(&glib::Bytes::from_owned(Arc::clone(data)));
    let (width, height) = natural_size(&svg)?;

    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).ok()?;

    {
        let context = cairo::Context::new(&surface).ok()?;
        let snapshot = gtk::Snapshot::new();

        svg.snapshot(&snapshot, f64::from(width), f64::from(height));

        // No node means the SVG parsed but draws nothing, which is not a
        // picture worth showing in place of the error.
        snapshot.to_node()?.draw(&context);
    }

    let raw = frame_from_surface(surface)?;
    let (width, height) = (raw.width, raw.height);

    Some(Image {
        inner: Arc::new(ImageInner {
            source: FrameSource::Still(raw),
            width,
            height,
            scale: OnceLock::new(),
            // Still, always. See the module documentation.
            animation: None,
        }),
    })
}

/// The size to rasterise the given SVG at.
///
/// [`None`] means there is nothing to draw: a document that failed to parse
/// reports no width, no height and no aspect ratio either, which is the only
/// signal `GtkSvg` gives us — its loading functions return no error.
fn natural_size(svg: &gtk::Svg) -> Option<(i32, i32)> {
    let (width, height) = match (svg.intrinsic_width(), svg.intrinsic_height()) {
        (width, height) if width > 0 && height > 0 => (width, height),
        // Only a `viewBox`, which is the common shape for an icon or a
        // sticker: pick a size and keep the shape.
        _ => {
            let ratio = svg.intrinsic_aspect_ratio();
            if ratio <= 0.0 {
                return None;
            }

            if ratio >= 1.0 {
                (DEFAULT_EXTENT as i32, (DEFAULT_EXTENT / ratio) as i32)
            } else {
                ((DEFAULT_EXTENT * ratio) as i32, DEFAULT_EXTENT as i32)
            }
        }
    };

    Some((width.clamp(1, MAX_EXTENT), height.clamp(1, MAX_EXTENT)))
}

/// Copy a drawn surface into the tightly packed, straight-alpha RGBA that
/// everything downstream expects.
///
/// Cairo hands back rows padded to a stride of its own choosing, holding
/// native-endian `0xAARRGGBB` with the colour already multiplied by the alpha.
/// All three of those have to be undone here.
fn frame_from_surface(mut surface: cairo::ImageSurface) -> Option<RawFrame> {
    let width = usize::try_from(surface.width()).ok()?;
    let height = usize::try_from(surface.height()).ok()?;
    let stride = usize::try_from(surface.stride()).ok()?;

    // Cairo buffers the drawing, and `data` is only correct once it has been
    // flushed out to the surface.
    surface.flush();
    let pixels = surface.data().ok()?;

    let mut data = Vec::with_capacity(width * height * 4);

    for row in 0..height {
        let start = row * stride;

        for column in 0..width {
            let pixel = start + column * 4;
            let Some(bytes) = pixels.get(pixel..pixel + 4) else {
                // A short buffer should not be possible, but a torn image is
                // better than a panic in a decoder.
                break;
            };

            let argb = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let alpha = ((argb >> 24) & 0xff) as u8;

            for shift in [16, 8, 0] {
                let value = ((argb >> shift) & 0xff) as u8;
                data.push(straighten(value, alpha));
            }
            data.push(alpha);
        }
    }

    // However short the copy came out, the dimensions have to describe it.
    let rows = (data.len() / 4).checked_div(width)?;

    Some(RawFrame {
        data,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(rows).ok()?,
        delay: None,
    })
}

/// Undo cairo's premultiplication of one colour channel.
fn straighten(value: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }

    // Rounded rather than truncated: the values are small, and truncating
    // every channel of every pixel darkens a whole image visibly.
    let straightened = (u32::from(value) * 255 + u32::from(alpha) / 2) / u32::from(alpha);
    straightened.min(255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest SVG that draws something.
    // Two hashes, because the colour in it closes a one-hash raw string.
    const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4">
        <rect width="8" height="4" fill="#ff0000"/>
    </svg>"##;

    /// The sniff has to find the root element past whatever precedes it, and
    /// has to leave every binary format alone — this runs before the ordinary
    /// decode path, so a false positive here delays a JPEG rather than only
    /// wasting a render.
    #[test]
    fn an_svg_is_recognised_and_nothing_else_is() {
        assert!(looks_like_svg(SVG));
        assert!(looks_like_svg(
            b"<?xml version=\"1.0\"?>\n<!-- a comment -->\n<svg/>"
        ));

        assert!(!looks_like_svg(b"\x89PNG\r\n\x1a\n"));
        assert!(!looks_like_svg(b"\xff\xd8\xff\xe0"));
        assert!(!looks_like_svg(b"<html><body>not a picture</body></html>"));
        assert!(!looks_like_svg(b""));
    }

    /// Premultiplied black at half alpha is still black; premultiplied white
    /// at half alpha has to come back white, not grey. Getting this backwards
    /// is the kind of thing that looks like a rendering bug in the SVG.
    #[test]
    fn premultiplication_is_undone() {
        assert_eq!(straighten(0, 128), 0);
        assert_eq!(straighten(128, 128), 255);
        assert_eq!(straighten(64, 128), 128);
        assert_eq!(straighten(0, 0), 0, "nothing is visible through no alpha");
    }
}

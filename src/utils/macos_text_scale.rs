//! Text that is the size it is everywhere else, on macOS.
//!
//! GTK sizes a font by resolving its point size against `gtk-xft-dpi`, and the
//! two platforms disagree about what that number is. Linux uses 96 dots per
//! inch, by a convention old enough that nothing measures a real screen with
//! it any more. macOS uses 72, which is not a fudge at all: its coordinate
//! space really is 72 units to the inch, so a point maps to exactly one
//! logical pixel and the backend reports that honestly.
//!
//! The consequence is that the same stylesheet renders a quarter smaller here.
//! Measured on this port: a default label comes out at 12 px against 14.67 px
//! on Linux, and the `15pt` a Markdown `h1` is styled with comes out at 15 px
//! against 20 px.
//!
//! That last part is the reason this is a bug rather than a preference. The
//! timeline sizes its headings in `pt` (`_room_history.scss`), picked to sit
//! above body text at 96 dpi. At 72 they are all compressed toward it, and
//! `h6` — 11 pt, so 11 px — ends up *smaller* than the 12 px body it is
//! supposed to be a heading for. Raising the resolution fixes the whole scale
//! at once, where changing the default font size alone would leave the
//! headings wrong.
//!
//! So macOS is told to resolve fonts the way everywhere else does. This makes
//! Commune's text larger than a typical Mac application's, which is the
//! deliberate trade: the alternative is a window that does not match the same
//! application on any other platform, and a heading hierarchy that reads
//! backwards.

use tracing::{debug, warn};

/// The font resolution GTK assumes on Linux, in units of 1/1024 dot per inch.
const LINUX_DPI: i32 = 96 * 1024;

/// The font resolution GTK's macOS backend reports, in the same units.
///
/// This is only used to recognise that nothing has overridden it. Anything
/// else is somebody's deliberate choice — a `settings.ini`, most likely — and
/// is left alone.
const QUARTZ_DPI: i32 = 72 * 1024;

/// Resolve fonts at the same resolution as every other platform.
///
/// Must be called after `gtk::init()`, which is where the settings this reads
/// come from.
pub(crate) fn init() {
    let Some(settings) = gtk::Settings::default() else {
        warn!("Could not set the text resolution: there are no settings to set it on");
        return;
    };

    let current = settings.gtk_xft_dpi();

    if current != QUARTZ_DPI {
        debug!(
            "Leaving the text resolution at {:.0} dpi, which is not ours to change",
            f64::from(current) / 1024.0,
        );
        return;
    }

    settings.set_gtk_xft_dpi(LINUX_DPI);
    debug!(
        "Raised the text resolution from {:.0} to {:.0} dpi",
        f64::from(QUARTZ_DPI) / 1024.0,
        f64::from(LINUX_DPI) / 1024.0,
    );
}

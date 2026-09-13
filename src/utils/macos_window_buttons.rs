//! Where macOS draws the window's own buttons.
//!
//! On macOS the close, minimise and zoom buttons are drawn by the system at
//! the top left of every window, over whatever GTK puts there. libadwaita's
//! header bars know this and leave the room, through GTK's native window
//! controls. A plain `GtkHeaderBar` can be asked for the same, but the
//! placeholder GTK uses for it also resets the window's titlebar height from
//! its own allocation every time it is laid out, and a header bar that slides
//! out for fullscreen or is closed under the viewer leaves that height wrong
//! for the whole window afterwards — the buttons end up half outside it. So
//! a widget that has to clear the buttons asks here for how far they reach,
//! the same measurement GTK's placeholder takes, and pads itself.

use std::ffi::c_void;

use gtk::prelude::*;
use objc2::{
    encode::{Encode, Encoding},
    msg_send,
    runtime::AnyObject,
};

/// `NSWindowZoomButton`, the rightmost of the three.
const NS_WINDOW_ZOOM_BUTTON: usize = 2;

/// `CGPoint`, spelled out because the foundation crate's copy is behind a
/// feature that pulls in another crate.
#[repr(C)]
#[derive(Clone, Copy)]
struct CgPoint {
    x: f64,
    y: f64,
}

// SAFETY: this is the layout and encoding of `CGPoint`.
unsafe impl Encode for CgPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}

/// `CGSize`.
#[repr(C)]
#[derive(Clone, Copy)]
struct CgSize {
    width: f64,
    height: f64,
}

// SAFETY: this is the layout and encoding of `CGSize`.
unsafe impl Encode for CgSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

/// `CGRect`.
#[repr(C)]
#[derive(Clone, Copy)]
struct CgRect {
    origin: CgPoint,
    size: CgSize,
}

// SAFETY: this is the layout and encoding of `CGRect`.
unsafe impl Encode for CgRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CgPoint::ENCODING, CgSize::ENCODING]);
}

unsafe extern "C" {
    /// The `NSWindow` behind a `GdkMacosSurface`. Public GDK API, in the
    /// library the `gtk` crate already links.
    fn gdk_macos_surface_get_native_window(surface: *mut c_void) -> *mut AnyObject;
}

/// How far from the window's left edge the native buttons reach, in logical
/// pixels, or `None` when the window has no native buttons to measure.
///
/// The window has to be realized.
pub(crate) fn native_controls_end(window: &gtk::Window) -> Option<i32> {
    let surface = window.surface()?;

    // SAFETY: a realized window on macOS has a `GdkMacosSurface`, which is
    // what the function takes, and it returns nil or a window `AppKit` owns
    // for as long as the surface does.
    let ns_window = unsafe { gdk_macos_surface_get_native_window(surface.as_ptr().cast()) };
    if ns_window.is_null() {
        return None;
    }

    // SAFETY: `ns_window` is an `NSWindow`, and `standardWindowButton:`
    // returns nil or a view the window owns.
    let button: *mut AnyObject =
        unsafe { msg_send![ns_window, standardWindowButton: NS_WINDOW_ZOOM_BUTTON] };
    if button.is_null() {
        return None;
    }

    // SAFETY: `button` is an `NSView`, whose frame is a `CGRect` in its
    // superview's coordinates -- the titlebar, which spans the window.
    let frame: CgRect = unsafe { msg_send![button, frame] };
    let end = frame.origin.x + frame.size.width;

    // Logical pixels are points on macOS.
    #[allow(clippy::cast_possible_truncation)]
    Some(end.ceil() as i32)
}

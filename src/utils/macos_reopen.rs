//! Bringing the window back when the application is reopened, on macOS.
//!
//! Closing the window on macOS hides it and leaves the application running
//! (see `Window::close_request`), and the way back is the Dock: clicking the
//! icon of a running application sends it the `'rapp'` Apple Event, which
//! `AppKit` turns into `-applicationShouldHandleReopen:hasVisibleWindows:` on
//! the delegate. The delegate is GTK's, which does not implement it, and
//! `AppKit`'s default for an application that is not document-based is to do
//! nothing. So, as with `matrix:` URLs, the event is taken straight from the
//! Apple Event Manager, and presenting the window is what it does.

use std::{ffi::c_void, ptr};

use gtk::prelude::*;
use tracing::{debug, error};

use super::macos_url_events::{AEInstallEventHandler, AppleEvent, NO_ERR};
use crate::Application;

/// `'aevt'`, the class of the core Apple Events.
const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
/// `'rapp'`, reopen application.
const K_AE_REOPEN_APPLICATION: u32 = u32::from_be_bytes(*b"rapp");

/// Present the main window.
unsafe extern "C" fn handle_reopen(
    _event: *const AppleEvent,
    _reply: *mut AppleEvent,
    _refcon: *mut c_void,
) -> i16 {
    debug!("Reopened from the Dock");
    Application::default().activate();

    NO_ERR
}

/// Start answering the Dock.
///
/// Must be called after `GtkApplication` has started up, which is where GTK
/// sends `-finishLaunching` and `AppKit` installs the handler this replaces.
pub(crate) fn init() {
    // SAFETY: the handler has the signature the Apple Event Manager expects,
    // and takes no reference to anything that could go away.
    let status = unsafe {
        AEInstallEventHandler(
            K_CORE_EVENT_CLASS,
            K_AE_REOPEN_APPLICATION,
            handle_reopen,
            ptr::null_mut(),
            0,
        )
    };

    if status == NO_ERR {
        debug!("Listening for the Dock");
    } else {
        error!("Could not listen for the Dock: OSErr {status}");
    }
}

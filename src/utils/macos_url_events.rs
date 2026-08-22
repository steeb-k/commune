//! Receiving `matrix:` URLs on macOS.
//!
//! Everywhere else this arrives on its own. The desktop file claims the scheme,
//! the launcher hands the URI to a D-Bus activation or to `argv`, and GIO turns
//! it into the `GFile` that `Application::open()` is given.
//!
//! On macOS `LaunchServices` reads the scheme from the bundle's
//! `CFBundleURLTypes` and sends the URL as an Apple Event — class and ID both
//! `'GURL'` — to the running application. `AppKit` would forward that to
//! `-application:openURLs:` on the application delegate, but the delegate is
//! GTK's, and `GtkApplicationQuartzDelegate` implements only
//! `-applicationShouldTerminate:` and `-application:openFiles:`. The URL is
//! dropped, silently, with nothing logged.
//!
//! So we take the event ourselves, straight from the Apple Event Manager, and
//! hand it to [`Application::open()`] — which is the same entry point the Linux
//! path ends at, so everything past this file is shared.

use std::{ffi::c_void, ptr, str};

use gtk::{gio, prelude::*};
use tracing::{debug, error, warn};

use crate::Application;

/// `'GURL'`, the class of the Apple Event that carries a URL.
const K_INTERNET_EVENT_CLASS: u32 = u32::from_be_bytes(*b"GURL");
/// `'GURL'`, its ID, which happens to be the same four characters.
const K_AE_GET_URL: u32 = u32::from_be_bytes(*b"GURL");
/// `'----'`, the parameter of an Apple Event that holds its direct object.
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
/// `'utf8'`, the descriptor type to ask for the URL as.
const TYPE_UTF8_TEXT: u32 = u32::from_be_bytes(*b"utf8");

/// `noErr`.
const NO_ERR: i16 = 0;

/// The longest URL we will accept.
///
/// A `matrix:` URI is a room or user ID and at most a couple of routing
/// servers, so this is far more than one can be.
const MAX_URL_LEN: usize = 4096;

/// An `AppleEvent`, which we never look inside: it is only ever passed back to
/// the Apple Event Manager.
#[repr(C)]
struct AppleEvent {
    _private: [u8; 0],
}

/// The handler the Apple Event Manager calls.
type AeEventHandler = unsafe extern "C" fn(*const AppleEvent, *mut AppleEvent, *mut c_void) -> i16;

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AEInstallEventHandler(
        event_class: u32,
        event_id: u32,
        handler: AeEventHandler,
        handler_refcon: *mut c_void,
        is_sys_handler: u8,
    ) -> i16;

    fn AEGetParamPtr(
        the_apple_event: *const AppleEvent,
        the_ae_keyword: u32,
        desired_type: u32,
        actual_type: *mut u32,
        data_ptr: *mut c_void,
        maximum_size: isize,
        actual_size: *mut isize,
    ) -> i16;
}

/// Read the URL out of a `'GURL'` event and open it.
unsafe extern "C" fn handle_get_url(
    event: *const AppleEvent,
    _reply: *mut AppleEvent,
    _refcon: *mut c_void,
) -> i16 {
    let mut actual_type = 0u32;
    let mut actual_size = 0isize;
    let mut buffer = [0u8; MAX_URL_LEN];

    // SAFETY: `event` is the event the Apple Event Manager is calling us with,
    // and the buffer we describe to `AEGetParamPtr` is the one we own here.
    let status = unsafe {
        AEGetParamPtr(
            event,
            KEY_DIRECT_OBJECT,
            TYPE_UTF8_TEXT,
            &raw mut actual_type,
            buffer.as_mut_ptr().cast(),
            MAX_URL_LEN.cast_signed(),
            &raw mut actual_size,
        )
    };

    if status != NO_ERR {
        error!("Could not read the URL out of an Apple Event: OSErr {status}");
        return status;
    }

    let Ok(len) = usize::try_from(actual_size) else {
        error!("Apple Event reported a negative URL length");
        return NO_ERR;
    };
    if len > MAX_URL_LEN {
        // `AEGetParamPtr` truncates rather than failing, so what is in the
        // buffer is not the URL that was sent.
        warn!("Ignoring a URL of {len} bytes, which is more than we accept");
        return NO_ERR;
    }

    let Ok(uri) = str::from_utf8(&buffer[..len]) else {
        error!("Apple Event carried a URL that is not valid UTF-8");
        return NO_ERR;
    };

    debug!(uri, "Received a URL from LaunchServices");
    Application::default().open(&[gio::File::for_uri(uri)], "");

    NO_ERR
}

/// Start receiving `matrix:` URLs.
///
/// Must be called after `GtkApplication` has started up, which is where GTK
/// sends `-finishLaunching` and `AppKit` installs the handlers of its own that
/// this one replaces.
pub(crate) fn init() {
    // SAFETY: the handler has the signature the Apple Event Manager expects,
    // and takes no reference to anything that could go away, so it stays valid
    // for as long as the process does.
    let status = unsafe {
        AEInstallEventHandler(
            K_INTERNET_EVENT_CLASS,
            K_AE_GET_URL,
            handle_get_url,
            ptr::null_mut(),
            0,
        )
    };

    if status == NO_ERR {
        debug!("Listening for `matrix:` URLs");
    } else {
        error!("Could not listen for `matrix:` URLs: OSErr {status}");
    }
}

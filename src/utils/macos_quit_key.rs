//! Command-Q closes the window, on macOS.
//!
//! Closing the window hides it and leaves the application running (see
//! `Window::close_request`), and Command-Q is meant to do the same, with the
//! Quit item and the Dock the only ways to end the process. That cannot be
//! done with accelerators alone. `AppKit` matches a Command key against the
//! menu bar before anything else sees it, and a combination no item carries
//! goes nowhere: GTK never receives it, so an accelerator on `window.close`
//! is inert here. The Quit item carries Command-Q, put there by GTK's own
//! startup; `Application::startup` takes it away again, which leaves the key
//! unmatched, and unmatched means dropped.
//!
//! So the key is taken before the menu bar looks. A local event monitor is
//! `AppKit`'s hook for exactly this: it sees every key event the application
//! is about to dispatch and can swallow it. This one swallows a bare
//! Command-Q, closes the window, and touches nothing else.

use std::ptr;

use block2::RcBlock;
use gtk::prelude::*;
use objc2::{class, msg_send, runtime::AnyObject};
use objc2_foundation::NSString;
use tracing::{debug, error};

use crate::Application;

/// `NSEventMaskKeyDown`.
const NS_EVENT_MASK_KEY_DOWN: u64 = 1 << 10;

/// `NSEventModifierFlagShift`.
const NS_EVENT_MODIFIER_FLAG_SHIFT: u64 = 1 << 17;
/// `NSEventModifierFlagControl`.
const NS_EVENT_MODIFIER_FLAG_CONTROL: u64 = 1 << 18;
/// `NSEventModifierFlagOption`.
const NS_EVENT_MODIFIER_FLAG_OPTION: u64 = 1 << 19;
/// `NSEventModifierFlagCommand`.
const NS_EVENT_MODIFIER_FLAG_COMMAND: u64 = 1 << 20;

/// The modifiers that make a key a different shortcut.
const SHORTCUT_MODIFIERS: u64 = NS_EVENT_MODIFIER_FLAG_SHIFT
    | NS_EVENT_MODIFIER_FLAG_CONTROL
    | NS_EVENT_MODIFIER_FLAG_OPTION
    | NS_EVENT_MODIFIER_FLAG_COMMAND;

/// Whether `event` is a bare Command-Q.
///
/// # Safety
///
/// `event` must be an `NSEvent`.
unsafe fn is_command_q(event: *mut AnyObject) -> bool {
    // SAFETY: an `NSEvent` answers both of these; the string is owned by the
    // event and only read while it is alive.
    unsafe {
        let flags: u64 = msg_send![event, modifierFlags];
        if flags & SHORTCUT_MODIFIERS != NS_EVENT_MODIFIER_FLAG_COMMAND {
            return false;
        }

        let characters: *mut NSString = msg_send![event, charactersIgnoringModifiers];
        characters
            .as_ref()
            .is_some_and(|characters| characters.to_string() == "q")
    }
}

/// Start closing the window on Command-Q.
///
/// Must be called after `GtkApplication` has started up, so that `NSApp`
/// exists.
pub(crate) fn init() {
    let handler = RcBlock::new(|event: *mut AnyObject| -> *mut AnyObject {
        // SAFETY: the monitor is registered for key events, so this is an
        // `NSEvent`.
        if event.is_null() || !unsafe { is_command_q(event) } {
            return event;
        }

        debug!("Command-Q; closing the window");
        if let Some(window) = Application::default().main_window() {
            window.close();
        }

        // Swallowed: the menu bar never sees it.
        ptr::null_mut()
    });

    // SAFETY: the mask and the block are what the method takes, and `AppKit`
    // copies the block. The monitor is never removed, so its handle is not
    // kept.
    let monitor: *mut AnyObject = unsafe {
        msg_send![
            class!(NSEvent),
            addLocalMonitorForEventsMatchingMask: NS_EVENT_MASK_KEY_DOWN,
            handler: &*handler
        ]
    };

    if monitor.is_null() {
        error!("Could not watch for Command-Q");
    } else {
        debug!("Watching for Command-Q");
    }
}

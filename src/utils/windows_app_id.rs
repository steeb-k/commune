//! Claiming the application user model ID that Windows identifies us by.
//!
//! Windows tells applications apart by an `AppUserModelID`, and for anything to
//! do with notifications it will only deal with an application that has one. A
//! packaged application gets its ID from its package; an unpackaged one like
//! Commune has none at all until it says so — `GetApplicationUserModelId` on
//! our own process returns `APPMODEL_ERROR_NO_APPLICATION`.
//!
//! Without an ID, a notification is accepted and then dropped: nothing appears,
//! nothing is logged, and the application never shows up under
//! `HKCU\…\Notifications\Settings`, which is both where Windows records that an
//! application has sent one and what System Settings lists. So there is not
//! even anything for a user to switch on.
//!
//! The installer declares this same ID on the Start Menu shortcut, which is
//! what makes it an ID Windows will accept rather than an invention. But
//! declaring it there is not claiming it here — the two are separate, and this
//! is the half that was missing.

use tracing::{debug, warn};
use windows::{
    Win32::{
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
            RegCreateKeyExW, RegSetValueExW,
        },
        UI::Shell::SetCurrentProcessExplicitAppUserModelID,
    },
    core::{HSTRING, w},
};

use crate::{APP_ID, APP_NAME, AppProfile, PROFILE};

/// Tell Windows who we are.
///
/// Call this before anything might want to notify, and before `GApplication`
/// starts up. Every failure here is logged and shrugged off: the only thing
/// that stops working is notifications, and refusing to start over that would
/// be worse than not notifying.
pub(crate) fn init() {
    let app_id = HSTRING::from(APP_ID);

    // SAFETY: `app_id` is a null-terminated wide string that outlives the call,
    // which copies what it needs.
    if let Err(error) = unsafe { SetCurrentProcessExplicitAppUserModelID(&app_id) } {
        warn!("Could not set the application user model ID: {error}");
        return;
    }

    debug!("Claimed the application user model ID {APP_ID}");

    if let Err(error) = register_display_name() {
        warn!("Could not register the application user model ID: {error}");
    }
}

/// Give the ID a name, so that Windows has something to call us.
///
/// This is what System Settings shows beside the switch for our notifications,
/// and what a toast is attributed to. Without it the raw ID is shown, which
/// reads as a bug rather than as an application.
fn register_display_name() -> windows::core::Result<()> {
    let subkey = HSTRING::from(format!("Software\\Classes\\AppUserModelId\\{APP_ID}"));
    let mut key = HKEY::default();

    // SAFETY: the two strings outlive the call, and `key` is a valid place to
    // write the handle. It is closed on both paths below.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &raw mut key,
            None,
        )
    }
    .ok()?;

    // The profile rides in the name for the same reason it rides in the
    // application ID: a development build and a stable one can both be
    // installed, and two switches both labelled "Commune" would be a coin toss.
    //
    // Spelled out rather than taken from `AppProfile`'s `Display`, which is
    // lower case because it is used to build identifiers. This is a name, and
    // it has to match the one the installer puts on the Start Menu shortcut.
    let name = HSTRING::from(match PROFILE {
        AppProfile::Stable => APP_NAME.to_owned(),
        AppProfile::Beta => format!("{APP_NAME} Beta"),
        AppProfile::Devel => format!("{APP_NAME} Devel"),
    });

    // `RegSetValueExW` wants a `REG_SZ` as bytes, including the terminator.
    // SAFETY: `name` is a null-terminated wide string of `len() + 1` units, and
    // the slice borrows it for no longer than the call below.
    let bytes =
        unsafe { std::slice::from_raw_parts(name.as_ptr().cast::<u8>(), (name.len() + 1) * 2) };

    // SAFETY: `key` is open, and `bytes` is the wide string described above.
    let result = unsafe { RegSetValueExW(key, w!("DisplayName"), None, REG_SZ, Some(bytes)) };

    // SAFETY: `key` came from the successful `RegCreateKeyExW` above and is
    // closed exactly once.
    unsafe { RegCloseKey(key) }.ok()?;

    result.ok()
}

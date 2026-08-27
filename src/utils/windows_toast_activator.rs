//! Receiving a notification click that has to start Commune first.
//!
//! [`super::windows_notifications`] handles a click while Commune is running,
//! through the toast's own `Activated` event. That event is delivered to the
//! process that sent the toast, so once Commune has quit there is nobody left
//! to receive it — and for a chat client that is the case that matters most, a
//! message arriving while the application is closed.
//!
//! Windows solves it with COM. A toast can name a class to activate, and if no
//! process is serving that class Windows starts one from the registry and calls
//! it. So the pieces are:
//!
//! * a class implementing `INotificationActivationCallback`, whose `Activate`
//!   is handed the same `launch` payload the click would have delivered;
//! * `LocalServer32` under the class ID, naming our executable, so Windows can
//!   start us to serve it;
//! * `CustomActivator` on our `AppUserModelId` key, which is what associates
//!   the class with our notifications. The usual advice is to put a
//!   `ToastActivatorCLSID` on a Start Menu shortcut instead — the installer
//!   does that too — but the registry value needs no installer, so a Commune
//!   run from an unpacked `.zip` behaves the same as an installed one;
//! * a class factory registered with `CoRegisterClassObject`, so that a running
//!   Commune serves the class itself rather than having a second one started to
//!   receive the click.
//!
//! The class ID is fixed forever, and recorded in `doc/rebrand.md`. Changing it
//! orphans the registry of everyone who has ever run Commune.

// `#[implement]` writes the COM plumbing for the two objects below, and the
// plumbing is not written to our lint settings. None of it is ours to fix, and
// the lints cannot be silenced at the macro's use site because the code it
// complains about is emitted beside the type rather than inside it.
#![allow(clippy::inline_always, clippy::ref_as_ptr)]

use std::{env, ffi::c_void};

use tracing::{debug, warn};
use windows::{
    Win32::{
        Foundation::{CLASS_E_NOAGGREGATION, E_POINTER, S_OK},
        System::Com::{
            CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoInitializeEx, CoRegisterClassObject,
            IClassFactory, IClassFactory_Impl, REGCLS_MULTIPLEUSE,
        },
        UI::Notifications::{
            INotificationActivationCallback, INotificationActivationCallback_Impl,
            NOTIFICATION_USER_INPUT_DATA,
        },
    },
    core::{BOOL, GUID, Interface, PCWSTR, Ref, implement},
};

use super::{windows_app_id, windows_notifications};

/// The class Windows activates when one of our notifications is clicked.
///
/// Permanent. See `doc/rebrand.md`.
const TOAST_ACTIVATOR_CLSID: GUID = GUID::from_u128(0x7dc899bf_5566_4bdf_8169_77118efc646b);

/// The class ID as Windows writes it in the registry.
fn clsid_string() -> String {
    format!("{{{TOAST_ACTIVATOR_CLSID:?}}}").to_uppercase()
}

/// The object Windows calls when a notification is clicked.
#[implement(INotificationActivationCallback)]
struct ToastActivator;

impl INotificationActivationCallback_Impl for ToastActivator_Impl {
    fn Activate(
        &self,
        _app_user_model_id: &PCWSTR,
        invoked_args: &PCWSTR,
        _data: *const NOTIFICATION_USER_INPUT_DATA,
        _count: u32,
    ) -> windows::core::Result<()> {
        // SAFETY: Windows hands us a null-terminated wide string, or null when
        // the toast had no arguments.
        let payload = unsafe { invoked_args.to_string() }.unwrap_or_default();

        if payload.is_empty() {
            debug!("A notification was clicked with nothing to act on");
            return Ok(());
        }

        debug!("A notification was clicked from outside the application");
        windows_notifications::activate(&payload);

        Ok(())
    }
}

/// Hands Windows a [`ToastActivator`] when it asks for one.
#[implement(IClassFactory)]
struct ToastActivatorFactory;

impl IClassFactory_Impl for ToastActivatorFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<windows::core::IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if object.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: `object` is a valid out-parameter, checked above.
        unsafe { *object = std::ptr::null_mut() };

        // We do not support being aggregated into another object, and saying so
        // is required rather than optional.
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }

        let activator: INotificationActivationCallback = ToastActivator.into();

        // SAFETY: `iid` and `object` are the interface asked for and the place
        // to put it, both given by COM.
        unsafe { activator.query(iid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> windows::core::Result<()> {
        // Commune's lifetime is its own; nothing COM does should extend or
        // shorten it.
        Ok(())
    }
}

/// Make Commune reachable for a notification click.
///
/// Call this once at startup. Every failure is logged and shrugged off: what
/// stops working is a click on a notification, and that is not worth refusing
/// to start over.
pub(crate) fn init() {
    // GTK initializes COM for this thread as well, and later, so getting there
    // first decides the apartment. Apartment-threaded is what a process with a
    // window and a message pump wants, and it is what GTK would have chosen.
    // `S_FALSE` means it was already initialized, which is not a failure.
    // SAFETY: called once, from the main thread, before anything else uses COM.
    let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if result.is_err() && result != S_OK {
        warn!("Could not initialize COM for notification clicks: {result:?}");
        return;
    }

    if let Err(error) = register_class_object() {
        warn!("Could not serve the notification activator: {error}");
    }

    if let Err(error) = register_in_registry() {
        warn!("Could not register the notification activator: {error}");
    }
}

/// Serve the class from this process, so that a click while Commune is running
/// does not start a second one.
fn register_class_object() -> windows::core::Result<()> {
    let factory: IClassFactory = ToastActivatorFactory.into();

    // `REGCLS_MULTIPLEUSE` so that every activation is served by this process
    // rather than each one starting another.
    //
    // SAFETY: the class ID is a constant, and the factory is kept alive by COM
    // for as long as it is registered — which is until the process exits, since
    // the registration is never revoked.
    let cookie = unsafe {
        CoRegisterClassObject(
            &TOAST_ACTIVATOR_CLSID,
            &factory,
            CLSCTX_LOCAL_SERVER,
            REGCLS_MULTIPLEUSE,
        )?
    };

    debug!("Serving the notification activator, registration {cookie}");
    Ok(())
}

/// Tell Windows where to find us, and that our notifications use us.
fn register_in_registry() -> windows::core::Result<()> {
    let clsid = clsid_string();

    // Windows starts this to serve the class when no process already does. It
    // is given `-Embedding` on the command line, which `Application::run()`
    // knows to drop.
    let exe = env::current_exe()
        .map_err(|error| windows::core::Error::new(E_POINTER, format!("{error}")))?;
    let command = format!("\"{}\" -Embedding", exe.display());

    windows_app_id::write_string_value(
        &format!("Software\\Classes\\CLSID\\{clsid}\\LocalServer32"),
        None,
        &command,
    )?;

    // And this is what ties the class to our notifications rather than to
    // somebody else's.
    windows_app_id::write_string_value(
        &windows_app_id::app_user_model_key(),
        Some("CustomActivator"),
        &clsid,
    )?;

    debug!("Registered the notification activator as {clsid}");
    Ok(())
}

/// Whether the given argument is COM asking us to serve a class rather than a
/// person asking us to open something.
///
/// Windows spells it `-Embedding`, and has been known to spell it `/Embedding`.
pub(crate) fn is_embedding_argument(argument: &str) -> bool {
    matches!(argument, "-Embedding" | "/Embedding")
        || argument.eq_ignore_ascii_case("-embedding")
        || argument.eq_ignore_ascii_case("/embedding")
}

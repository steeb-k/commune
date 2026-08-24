//! Reaching the Java side of an Android application from Rust.
//!
//! GTK's Android glue starts the process, loads the application's shared object
//! and calls `main` on a thread of its own. It keeps a `JNIEnv` for that thread
//! and exposes it as `gdk_android_display_get_env()`, which is public API on
//! [`GdkAndroidDisplay`] and the only supported way in.
//!
//! A `JNIEnv` belongs to the thread it was made for and may not be used from
//! another, and almost nothing here runs on the GTK thread — the secret store
//! goes through `spawn_tokio!`, so it runs on a tokio worker. What crosses
//! threads is the `JavaVM`, which is process-wide. So the VM is captured once,
//! early, on the thread that has an env to take it from, and every later caller
//! attaches itself.
//!
//! [`GdkAndroidDisplay`]: https://docs.gtk.org/gdk4/class.AndroidDisplay.html

use std::{
    ffi::c_void,
    ptr,
    sync::{Mutex, OnceLock},
};

use gtk::{
    gdk,
    glib::{
        self,
        translate::{ToGlibPtr, from_glib_full},
    },
    prelude::*,
};
use jni::{
    AttachGuard, JNIEnv, JavaVM,
    objects::{JObject, JValue},
};
use tokio::sync::oneshot;
use tracing::debug;

unsafe extern "C" {
    /// The `JNIEnv` GTK made for the thread it runs `main` on.
    ///
    /// Public API since GTK 4.18, declared here because the gtk-rs bindings do
    /// not cover the Android backend.
    fn gdk_android_display_get_env(display: *mut c_void) -> *mut jni::sys::JNIEnv;

    /// Launch `intent` with `toplevel`'s Android `Activity` as parent.
    ///
    /// `toplevel` is a `GdkSurface *` that is dynamically a
    /// `GdkAndroidToplevel` — the Android backend's toplevel surface class is
    /// `G_DEFINE_TYPE_WITH_CODE (GdkAndroidToplevel, gdk_android_toplevel,
    /// GDK_TYPE_ANDROID_SURFACE, …)` (`gdkandroidtoplevel.c:332`), the same
    /// GObject a [`gdk::Surface`] already wraps on this backend, so the raw
    /// pointer needs no further conversion. Public API since GTK 4.18,
    /// declared here for the same reason as [`gdk_android_display_get_env`].
    fn gdk_android_toplevel_launch_activity(
        toplevel: *mut c_void,
        intent: jni::sys::jobject,
        error: *mut *mut glib::ffi::GError,
    ) -> glib::ffi::gboolean;
}

/// The `JavaVM` for this process.
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

/// The errors that can occur reaching the Java side.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AndroidJniError {
    /// The `JavaVM` was never captured, because [`init()`] did not run or did
    /// not succeed.
    #[error("the Java VM is not available")]
    NoJavaVm,

    /// There is no default `GdkDisplay` to take a `JNIEnv` from.
    #[error("no default display")]
    NoDisplay,

    /// The window passed to [`launch_uri()`] has no [`gdk::Surface`] to launch
    /// an activity as a parent of.
    #[error("window has no surface")]
    NoSurface,

    /// `gdk_android_toplevel_launch_activity` refused to start the activity —
    /// most likely `ActivityNotFoundException`, thrown when nothing on the
    /// device can handle the `Intent`.
    #[error("could not launch activity: {0}")]
    LaunchRefused(String),

    /// A JNI call failed.
    #[error(transparent)]
    Jni(#[from] jni::errors::Error),
}

/// Capture the `JavaVM` for later use from any thread.
///
/// Must be called on the GTK thread, after `gtk::init()` and before anything
/// asks for [`with_env()`]. Calling it twice is harmless.
///
/// This is separate from the callers that need it because the thread it has to
/// happen on is not the thread any of them run on.
pub(crate) fn init() -> Result<(), AndroidJniError> {
    if JAVA_VM.get().is_some() {
        return Ok(());
    }

    let display = gdk::Display::default().ok_or(AndroidJniError::NoDisplay)?;

    let display_ptr: *mut gdk::ffi::GdkDisplay = display.to_glib_none().0;

    // SAFETY: `gdk_android_display_get_env` takes a `GdkDisplay` and returns
    // the env GTK made for this thread, which is the thread we are on. The
    // pointer is owned by GTK and outlives the display.
    let env_ptr = unsafe { gdk_android_display_get_env(display_ptr.cast()) };
    if env_ptr.is_null() {
        return Err(AndroidJniError::NoJavaVm);
    }

    // SAFETY: the pointer came from GTK for this thread and is non-null.
    let env = unsafe { JNIEnv::from_raw(env_ptr) }?;
    let vm = env.get_java_vm()?;

    // A second caller racing us is fine; the VM is the same either way.
    let _ = JAVA_VM.set(vm);
    debug!("Captured the Java VM");

    Ok(())
}

/// Run the given closure with a `JNIEnv` valid for the calling thread.
///
/// The thread is attached to the `JavaVM` if it was not already, and detached
/// again when the guard drops.
///
/// The closure chooses its own error type so that callers can report what they
/// were doing rather than that JNI was involved; it only has to be able to
/// carry an [`AndroidJniError`] for the cases that happen before the closure
/// runs.
pub(crate) fn with_env<T, E, F>(f: F) -> Result<T, E>
where
    E: From<AndroidJniError>,
    F: FnOnce(&mut AttachGuard<'_>) -> Result<T, E>,
{
    let vm = JAVA_VM.get().ok_or(AndroidJniError::NoJavaVm)?;
    let mut guard = vm.attach_current_thread().map_err(AndroidJniError::from)?;

    f(&mut guard)
}

/// Launch `uri` in a browser, as a new `Activity` with `window`'s as parent.
///
/// `gtk::UriLauncher` cannot do this itself: `gtkurilauncher.c` has no Android
/// branch, so it falls through to `gtk_show_uri_full`, which asks GIO's
/// app-info registry for a handler — empty on Android, since nothing there
/// populates it. This builds the `ACTION_VIEW` `Intent` Android expects and
/// launches it directly through `gdk_android_toplevel_launch_activity`, the
/// same entry point `gtk::FileLauncher` uses for its own Android intents
/// (`gtkfilelauncher.c:511`).
pub(crate) fn launch_uri(window: &gtk::Window, uri: &str) -> Result<(), AndroidJniError> {
    let surface = window.surface().ok_or(AndroidJniError::NoSurface)?;
    let toplevel_ptr: *mut gdk::ffi::GdkSurface = surface.to_glib_none().0;

    with_env(|env| {
        let action = JObject::from(env.new_string("android.intent.action.VIEW")?);
        let uri_string = JObject::from(env.new_string(uri)?);
        let uri_obj = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&uri_string)],
            )?
            .l()?;

        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(&action), JValue::Object(&uri_obj)],
        )?;

        let mut error: *mut glib::ffi::GError = ptr::null_mut();

        // SAFETY: `toplevel_ptr` is the surface of a live `gtk::Window`, which
        // on the Android backend is always a `GdkAndroidToplevel`; `intent` is
        // a real `Intent` object constructed just above, and both outlive this
        // call.
        let ok = unsafe {
            gdk_android_toplevel_launch_activity(
                toplevel_ptr.cast(),
                intent.as_raw(),
                &raw mut error,
            )
        };

        if ok == glib::ffi::GFALSE {
            // SAFETY: the call above just set `error` to a `GError` it owns,
            // and this is the only read of it.
            let message = if error.is_null() {
                "unknown error".to_owned()
            } else {
                let error: glib::Error = unsafe { from_glib_full(error) };
                error.to_string()
            };
            return Err(AndroidJniError::LaunchRefused(message));
        }

        Ok(())
    })
}

/// The pending OAuth 2.0 / Matrix SSO redirect, waiting to be matched against
/// an incoming `Intent`.
///
/// A `Mutex` rather than anything fancier: only one login flow is ever on
/// screen at a time, and the sender is taken exactly once — either by a
/// matching redirect, or by [`await_oauth_redirect()`] being called again and
/// replacing an abandoned wait.
static PENDING_OAUTH_REDIRECT: Mutex<Option<oneshot::Sender<String>>> = Mutex::new(None);

/// Start waiting for the OAuth 2.0 / Matrix SSO redirect to arrive as an
/// incoming URI.
///
/// Called from `login::local_server::spawn_local_server()` just before the
/// browser is launched. Whatever URI later matches the custom scheme — see
/// `Application::process_uri` — is delivered here through
/// [`deliver_oauth_redirect()`].
pub(crate) fn await_oauth_redirect() -> oneshot::Receiver<String> {
    let (sender, receiver) = oneshot::channel();
    *PENDING_OAUTH_REDIRECT
        .lock()
        .expect("mutex should not be poisoned") = Some(sender);
    receiver
}

/// Deliver an incoming URI to whatever is waiting on
/// [`await_oauth_redirect()`].
///
/// Returns whether it was consumed. `false` means no login flow was waiting —
/// the caller has already checked the scheme, so this only happens if the
/// browser page was dismissed before the redirect arrived.
pub(crate) fn deliver_oauth_redirect(uri: String) -> bool {
    let sender = PENDING_OAUTH_REDIRECT
        .lock()
        .expect("mutex should not be poisoned")
        .take();

    match sender {
        Some(sender) => sender.send(uri).is_ok(),
        None => false,
    }
}

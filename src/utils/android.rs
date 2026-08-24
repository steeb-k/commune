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

use std::{ffi::c_void, sync::OnceLock};

use gtk::{gdk, glib::translate::ToGlibPtr};
use jni::{AttachGuard, JNIEnv, JavaVM};
use tracing::debug;

unsafe extern "C" {
    /// The `JNIEnv` GTK made for the thread it runs `main` on.
    ///
    /// Public API since GTK 4.18, declared here because the gtk-rs bindings do
    /// not cover the Android backend.
    fn gdk_android_display_get_env(display: *mut c_void) -> *mut jni::sys::JNIEnv;
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

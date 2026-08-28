//! Reaching the Java side of an Android application from Rust.
//!
//! This is the application's `utils::android` with the GTK entry points cut
//! away. There, the `JavaVM` was captured from
//! `gdk_android_display_get_env()` on the GTK thread, and the application
//! `Context` was dug out of a `GdkAndroidToplevel`'s `Activity`. Neither
//! exists in the Kotlin variant, and neither is needed: the Kotlin side has
//! both in hand at startup, so it calls [`Java_io_github_steeb_1k_commune_core_Native_seed`]
//! once, before touching anything else in the core, and [`JNI_OnLoad`]
//! catches the VM even earlier when the library is loaded with
//! `System.loadLibrary`.
//!
//! A `JNIEnv` belongs to the thread it was made for and may not be used from
//! another, and almost nothing here runs on a Java thread — the secret store
//! goes through `spawn_tokio!`, so it runs on a tokio worker. What crosses
//! threads is the `JavaVM`, which is process-wide. So the VM is captured
//! once and every later caller attaches itself through [`with_env()`].

use std::{ffi::c_void, sync::OnceLock};

use jni::{
    AttachGuard, JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObject},
    sys::{JNI_VERSION_1_6, jint},
};
use tracing::debug;

/// The `JavaVM` for this process.
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

/// The application `Context`, captured when the Kotlin side seeds it.
///
/// The application `Context` rather than an `Activity`'s own because it
/// outlives any `Activity`: Android destroys and recreates one on a
/// configuration change, and a system service held against a dead `Activity`
/// leaks it.
static APPLICATION_CONTEXT: OnceLock<GlobalRef> = OnceLock::new();

/// The errors that can occur reaching the Java side.
#[derive(Debug, thiserror::Error)]
pub enum AndroidJniError {
    /// The `JavaVM` was never captured, because the Kotlin side has not
    /// loaded the library or called `Native.seed()` yet.
    #[error("the Java VM is not available")]
    NoJavaVm,

    /// The application `Context` was never seeded.
    #[error("the application context is not available")]
    NoContext,

    /// A JNI call failed.
    #[error(transparent)]
    Jni(#[from] jni::errors::Error),
}

/// Capture the `JavaVM` when the library is loaded with `System.loadLibrary`.
///
/// `JNI_OnLoad` is only invoked by `System.loadLibrary`/`System.load`; a
/// library reached purely through JNA's own `dlopen` never sees it, which is
/// why [`Java_io_github_steeb_1k_commune_core_Native_seed`] captures the VM
/// too. Whichever runs first wins, and they capture the same VM.
///
/// # Safety
///
/// Called by the JVM with a valid `JavaVM` pointer, per the JNI invocation
/// contract.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn JNI_OnLoad(
    vm: *mut jni::sys::JavaVM,
    _reserved: *mut c_void,
) -> jint {
    // SAFETY: the JVM hands us its own valid, process-lifetime pointer.
    if let Ok(vm) = unsafe { JavaVM::from_raw(vm) } {
        let _ = JAVA_VM.set(vm);
        debug!("Captured the Java VM in JNI_OnLoad");
    }

    JNI_VERSION_1_6
}

/// Capture the `JavaVM` and application `Context` out of a JNI entry point.
///
/// Calling this when both are already captured is free, and a race stores
/// the same values either way.
pub fn seed_from_jni(env: &mut JNIEnv, context: &JObject) -> Result<(), AndroidJniError> {
    if JAVA_VM.get().is_none() {
        let _ = JAVA_VM.set(env.get_java_vm()?);
        debug!("Captured the Java VM from a JNI entry point");
    }

    if APPLICATION_CONTEXT.get().is_none() {
        // The caller may be holding a restricted `Context`; the application
        // one behind it is the one worth keeping, for the reason the
        // static's comment gives.
        let application = env
            .call_method(
                context,
                "getApplicationContext",
                "()Landroid/content/Context;",
                &[],
            )?
            .l()?;
        let _ = APPLICATION_CONTEXT.set(env.new_global_ref(&application)?);
        debug!("Captured the application context from a JNI entry point");
    }

    Ok(())
}

/// The JNI entry the Kotlin side calls once at startup:
/// `io.github.steeb_k.commune.core.Native.seed(context)`.
///
/// # Safety
///
/// Called by the JVM with a valid env and a `Context`, per the JNI calling
/// contract for a `native fun seed(context: Context)` declared on that class.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_io_github_steeb_1k_commune_core_Native_seed(
    mut env: JNIEnv,
    _class: JClass,
    context: JObject,
) {
    if let Err(error) = seed_from_jni(&mut env, &context) {
        // `tracing` may not have a subscriber this early; the error will
        // resurface as `NoJavaVm`/`NoContext` at the first real use anyway.
        debug!("Could not seed the Java VM from Native.seed: {error}");
    }
}

/// Run the given closure with a `JNIEnv` valid for the calling thread.
///
/// The thread is attached to the `JavaVM` if it was not already, and
/// detached again when the guard drops.
///
/// The closure chooses its own error type so that callers can report what
/// they were doing rather than that JNI was involved; it only has to be able
/// to carry an [`AndroidJniError`] for the cases that happen before the
/// closure runs.
pub fn with_env<T, E, F>(f: F) -> Result<T, E>
where
    E: From<AndroidJniError>,
    F: FnOnce(&mut AttachGuard<'_>) -> Result<T, E>,
{
    let vm = JAVA_VM.get().ok_or(AndroidJniError::NoJavaVm)?;
    let mut guard = vm.attach_current_thread().map_err(AndroidJniError::from)?;

    let result = f(&mut guard);

    // A call that threw comes back as `Error::JavaException` with the
    // exception still pending, and the next JNI call made on this thread
    // with one pending aborts the process. So clearing it is not tidiness,
    // it is what keeps a Java-side failure to a failed call.
    // `exception_describe` puts the stack trace in logcat, which is the only
    // place it would otherwise be readable.
    if guard.exception_check().unwrap_or(false) {
        let _ = guard.exception_describe();
        let _ = guard.exception_clear();
    }

    result
}

/// The application `Context` the Kotlin side seeded.
pub fn application_context() -> Result<&'static GlobalRef, AndroidJniError> {
    APPLICATION_CONTEXT.get().ok_or(AndroidJniError::NoContext)
}

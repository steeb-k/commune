//! Keeping Commune running while it is not on screen.
//!
//! Android freezes an application's process as soon as none of it is visible.
//! For a chat client that is not a matter of messages arriving late: the sync
//! loop is a tokio task in this process, and a frozen process runs nothing, so
//! nothing arrives at all until the app is opened again. Everything else in the
//! port works around a missing platform API; this works around the platform
//! doing exactly what it is designed to do.
//!
//! There were three ways out and this is the one that can be measured here. A
//! foreground service needs no push gateway, no distributor application and no
//! co-operation from the homeserver, so it works against any server, including
//! one that has never heard of us. UnifiedPush is better for battery and worse
//! for moving parts — it needs a distributor installed, a gateway the
//! homeserver can reach, and a pusher registered — and FCM needs Play services,
//! a Firebase project and Gradle changes pixiewood cannot express. Both remain
//! possible on top of this: `utils::android_notifications` takes no position on
//! where a notification came from.
//!
//! Three things about it are worth knowing before changing anything here.
//!
//! **It may only be started from the foreground.** Since API 31 a background
//! process calling `startForegroundService()` gets
//! `ForegroundServiceStartNotAllowedException`. When this was written that was
//! not a real constraint — a backgrounded Commune was frozen and could not
//! call anything — but a push wake changed that: a `MESSAGE` broadcast starts
//! the whole application in the background, its session restore reaches
//! [`update()`], and the exception is thrown for real (measured on the
//! emulator, 26 August 2026). The error arm below absorbs it, which is why it
//! is an arm and not a panic; the window-presented caller remains the one that
//! takes.
//!
//! **It is capped at six hours a day.** Android 15 gives a `dataSync`
//! foreground service six hours in any twenty-four, then calls `onTimeout()`,
//! and a service still running when the grace period ends is killed with an
//! ANR. `SyncService.java` stops itself there. So this buys most of a day, not
//! a permanent connection, and real push is the answer rather than an
//! optimisation of this one.
//!
//! **The service does nothing.** It starts no work and holds no state: the sync
//! loop is already running in this process, and all a foreground service has to
//! do to keep it running is exist. Which is why the decision about when it
//! should exist is here rather than in Java.

use std::sync::atomic::{AtomicBool, Ordering};

use gettextrs::gettext;
use jni::{
    AttachGuard,
    objects::{JObject, JValue},
};
use tracing::{debug, warn};

use super::android::{self, AndroidJniError};

/// The class implementing the service.
///
/// In GTK's package because pixiewood symlinks exactly one Java directory into
/// the Gradle project and gives an application no way to add its own — see
/// `build-aux/android/patch-gtk-service.sh`.
const SERVICE_CLASS: &str = "org.gtk.android.SyncService";

/// The extras `SyncService` reads its user-visible text and its icon out of.
///
/// They travel in the `Intent` because the translations are on this side:
/// `gettext` runs against Commune's own catalogues, and Java cannot reach them.
const EXTRA_CHANNEL_NAME: &str = "commune.channel_name";
const EXTRA_TITLE: &str = "commune.title";
const EXTRA_TEXT: &str = "commune.text";
const EXTRA_ICON: &str = "commune.icon";

/// Whether the service is believed to be running.
///
/// Starting one that is already started only re-delivers `onStartCommand`,
/// which is harmless, and stopping one that was never started is a no-op. This
/// is here to keep the log honest rather than to keep Android happy.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Start or stop the service to match whether it is needed.
///
/// Called when the window is presented, whenever the session list changes, and
/// when push delivery becomes available or stops being. Whether it is needed
/// is the caller's judgement — `Application::update_sync_service()` weighs the
/// session count and the delivery mode — and stopping is what makes both
/// logging out of the last session and push taking over tidy up after
/// themselves.
///
/// Must be called on the GTK thread, and (to start) with the application on
/// screen: see the note on API 31 above.
pub(crate) fn update(needed: bool) {
    if needed == RUNNING.load(Ordering::Relaxed) {
        return;
    }

    let result = if needed { start() } else { stop() };

    match result {
        Ok(()) => {
            RUNNING.store(needed, Ordering::Relaxed);
            if needed {
                debug!("Started syncing in the background");
            } else {
                debug!("Stopped syncing in the background");
            }
        }
        // Sessions are restored before there is a window, and a window is where
        // the `Activity` and therefore the `Context` come from. So the first
        // call of a run routinely arrives too early, and says so; the call from
        // `present_main_window()` is the one that takes. Expected, recovered
        // from, and not worth a warning that reads like a failure.
        Err(AndroidJniError::NoWindow) => {
            debug!("Too early to change background syncing; there is no window yet");
        }
        Err(error) => {
            // Not fatal in either direction. Failing to start costs background
            // delivery and nothing else; failing to stop leaves a notification
            // that the next launch will replace.
            warn!("Could not change background syncing: {error}");
        }
    }
}

/// Ask Android to start the service in the foreground.
fn start() -> Result<(), AndroidJniError> {
    let context = android::application_context()?;

    android::with_env(|env| {
        let context = context.as_obj();
        let intent = service_intent(env, context)?;

        put_string(env, &intent, EXTRA_CHANNEL_NAME, &gettext("Sync"))?;
        // Shown in the notification shade for as long as this runs, so it says
        // what is happening rather than naming the mechanism.
        put_string(env, &intent, EXTRA_TITLE, &gettext("Commune is running"))?;
        put_string(
            env,
            &intent,
            EXTRA_TEXT,
            &gettext("Watching for new messages"),
        )?;

        // The same monochrome drawable the message notifications use. Looking
        // it up here rather than in Java keeps the one resource lookup in one
        // place.
        let icon = super::android_notifications::small_icon_resource(env, context)?;
        let name = JObject::from(env.new_string(EXTRA_ICON)?);
        env.call_method(
            &intent,
            "putExtra",
            "(Ljava/lang/String;I)Landroid/content/Intent;",
            &[JValue::Object(&name), JValue::Int(icon)],
        )?;

        env.call_method(
            context,
            "startForegroundService",
            "(Landroid/content/Intent;)Landroid/content/ComponentName;",
            &[JValue::Object(&intent)],
        )?;

        Ok(())
    })
}

/// Ask Android to stop the service.
fn stop() -> Result<(), AndroidJniError> {
    let context = android::application_context()?;

    android::with_env(|env| {
        let context = context.as_obj();
        let intent = service_intent(env, context)?;

        env.call_method(
            context,
            "stopService",
            "(Landroid/content/Intent;)Z",
            &[JValue::Object(&intent)],
        )?;

        Ok(())
    })
}

/// An `Intent` naming the service.
///
/// `setClassName()` rather than the `Class`-taking `Intent` constructor on
/// purpose: that constructor needs a `Class` object, and `FindClass` on a
/// thread JNI attached itself to searches the system class loader, which cannot
/// see the application's own classes. It is the reason GTK's own
/// `gdk_android_initialize()` is handed a class loader. Naming the component as
/// a string sidesteps class loading altogether.
fn service_intent<'a>(
    env: &mut AttachGuard<'a>,
    context: &JObject,
) -> Result<JObject<'a>, AndroidJniError> {
    let intent = env.new_object("android/content/Intent", "()V", &[])?;
    let class = JObject::from(env.new_string(SERVICE_CLASS)?);

    env.call_method(
        &intent,
        "setClassName",
        "(Landroid/content/Context;Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(context), JValue::Object(&class)],
    )?;

    Ok(intent)
}

/// Put a string extra on the given `Intent`.
fn put_string(
    env: &mut AttachGuard<'_>,
    intent: &JObject,
    name: &str,
    value: &str,
) -> Result<(), AndroidJniError> {
    let name = JObject::from(env.new_string(name)?);
    let value = JObject::from(env.new_string(value)?);

    env.call_method(
        intent,
        "putExtra",
        "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&name), JValue::Object(&value)],
    )?;

    Ok(())
}

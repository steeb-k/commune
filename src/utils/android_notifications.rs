//! Native notifications on Android.
//!
//! Everywhere else this is `GNotification`: the application hands GIO a
//! notification and GIO finds a backend for it. Android has no backend to
//! find. GIO ships three — `gtk`, `freedesktop` and `cocoa` — and the first two
//! are D-Bus, which no Android app can reach. So
//! `g_application_send_notification()` there is not a failure, it is a silent
//! no-op: every notification Commune has ever posted on Android went nowhere,
//! without so much as a warning.
//!
//! What replaces it is `NotificationManager`, reached over JNI the same way the
//! Keystore and the SSO redirect are. Four things about it are worth knowing
//! before changing anything here.
//!
//! **A channel is mandatory.** Since API 26 a notification whose channel does
//! not exist is dropped by the system, quietly. The channel also owns the
//! importance, the sound and whether it may interrupt — the user can retune all
//! of that in system settings, and an app that created a channel with low
//! importance cannot talk its way back up. So the channel is created once, at
//! [`init()`], with `IMPORTANCE_HIGH`, which is what `im.received` and
//! `NotificationPriority::High` asked for on the other platforms.
//!
//! **Permission is a runtime permission.** `POST_NOTIFICATIONS` arrived in API
//! 33 and this build targets 36, so the manifest entry only earns the right to
//! ask. [`init()`] asks, and does not wait for the answer: the answer arrives
//! at `Activity.onRequestPermissionsResult`, which would mean another patch to
//! GTK's Java glue for something `checkSelfPermission()` can be asked at any
//! later moment anyway.
//!
//! **A tap has to survive the process.** The action a notification carries is a
//! `GAction` name and a `GVariant` target, and by the time it is tapped the
//! process that posted it may be long dead. So it does not travel in memory: it
//! is written into the `Intent`'s URI, under the same custom scheme the SSO
//! redirect uses, and read back in `Application::process_uri()`. That reuses
//! the whole delivery path the redirect already built — the `intent-filter` in
//! `patch-manifest.sh`, the `onNewIntent` override in `patch-gtk-intent.sh` —
//! and it works whether the tap resumed Commune or started it.
//!
//! **The icon is bytes, not a file.** macOS wants a file URL, so
//! [`super::macos_notifications`] writes the avatar to the cache and cleans up
//! after itself; `BitmapFactory.decodeByteArray` takes the PNG directly, and
//! nothing here touches the disk.

use std::sync::atomic::{AtomicBool, Ordering};

use gettextrs::gettext;
use gtk::{gdk, glib, prelude::*};
use jni::{
    AttachGuard,
    objects::{JObject, JValue},
};
use tracing::{debug, error, warn};

use super::android::{self, AndroidJniError};

/// The URI that a tapped notification opens Commune with.
///
/// The scheme is the one `patch-manifest.sh` registered an `intent-filter` for,
/// and the path is what distinguishes this from the OAuth 2.0 / SSO redirect
/// that arrives through the very same door. Both are recognized in
/// `Application::process_uri()`.
pub(crate) const NOTIFICATION_URI: &str = "io.github.steeb-k.commune:/notification";

/// The query parameter carrying the name of the application action to
/// activate.
const ACTION_PARAMETER: &str = "action";

/// The query parameter carrying that action's target, as printed `GVariant`
/// text.
const TARGET_PARAMETER: &str = "target";

/// The ID of the notification channel messages are posted to.
///
/// The same string GIO is given as the notification category on the other
/// platforms, for want of a reason to invent a second name for the same thing.
const CHANNEL_ID: &str = "im.received";

/// The name of the drawable used as the status bar icon.
///
/// pixiewood generates this from `data/icons/*-symbolic.svg` for the launcher's
/// monochrome layer (`build-aux/android/io.github.steeb_k.Commune.xml`). A
/// small icon is masked down to its alpha channel and tinted by the system, so
/// a monochrome silhouette is exactly the right shape for one — the full-color
/// launcher icon would come out as a white blob.
const SMALL_ICON_NAME: &str = "ic_launcher_monochrome";

/// `android.R.drawable.stat_notify_chat`, used when the drawable above is not
/// there.
///
/// A notification with no small icon is not shown, it is refused with a
/// `NullPointerException` out of `NotificationManager`, so there has to be a
/// fallback and it has to be something the platform always has.
const FALLBACK_SMALL_ICON: i32 = 0x0108_0056;

/// The `Activity` a tap should be delivered to.
///
/// Naming it makes the `Intent` explicit, which matters: an implicit
/// `ACTION_VIEW` is matched against every `intent-filter` on the device and
/// could be answered by another application, while an explicit one goes where
/// it is addressed.
const ACTIVITY_CLASS: &str = "org.gtk.android.ToplevelActivity";

/// The permission needed to post a notification at all, since API 33.
const POST_NOTIFICATIONS: &str = "android.permission.POST_NOTIFICATIONS";

/// `NotificationManager.IMPORTANCE_HIGH`.
///
/// Constants rather than field lookups because these are `static final int`s
/// whose values are part of the platform API and will not change — the same
/// judgement `secret::android::keystore` made.
const IMPORTANCE_HIGH: i32 = 4;

/// `PackageManager.PERMISSION_GRANTED`.
const PERMISSION_GRANTED: i32 = 0;

/// `PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT`.
///
/// Immutable is required since API 31 of any `PendingIntent` that is not
/// explicitly mutable, and is what we want regardless: nothing outside Commune
/// should be able to fill in the blanks of an `Intent` that acts as Commune.
const PENDING_INTENT_FLAGS: i32 = 0x0400_0000 | 0x0800_0000;

/// The numeric ID every notification is posted under.
///
/// `NotificationManager.notify()` identifies a notification by the pair of a
/// tag and an `int`. The tag is the ID the caller would have given
/// `GApplication::send_notification()`, which is already unique, so the `int`
/// is a constant and the tag does all the work — including the replace-by-ID
/// behavior `GNotification` has.
const NOTIFICATION_ID: i32 = 0;

/// Whether permission to post notifications has been asked for this run.
///
/// Asking twice is not harmful — Android shows the prompt at most twice ever,
/// and denies silently afterwards — but there is no reason to spend one of
/// those on a request that was already made and answered.
static PERMISSION_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Create the notification channel, and ask for permission to post.
///
/// Takes the window because that is where the `Activity` comes from, and the
/// `Activity` is what `requestPermissions()` needs — the application `Context`
/// can only be asked whether permission is held, not for permission itself.
///
/// Called every time the main window is presented, and cheap after the first
/// time: creating a channel that already exists is documented as leaving the
/// existing one alone, and the permission is only asked for once.
pub(crate) fn init(window: &gtk::Window) {
    if let Err(error) = set_up(window) {
        warn!("Could not set notifications up: {error}");
    }
}

/// The body of [`init()`], so that one place reports what went wrong.
fn set_up(window: &gtk::Window) -> Result<(), AndroidJniError> {
    let activity = android::activity(window)?;
    let context = android::application_context()?;

    android::with_env(|env| {
        let manager = notification_manager(env, context.as_obj())?;

        let id = JObject::from(env.new_string(CHANNEL_ID)?);
        // The heading of Commune's own section in the system's notification
        // settings, so it is user-facing text.
        let name = JObject::from(env.new_string(gettext("Messages"))?);
        let channel = env.new_object(
            "android/app/NotificationChannel",
            "(Ljava/lang/String;Ljava/lang/CharSequence;I)V",
            &[
                JValue::Object(&id),
                JValue::Object(&name),
                JValue::Int(IMPORTANCE_HIGH),
            ],
        )?;
        env.call_method(
            &manager,
            "createNotificationChannel",
            "(Landroid/app/NotificationChannel;)V",
            &[JValue::Object(&channel)],
        )?;

        if has_permission(env, context.as_obj())? {
            debug!("Allowed to show notifications");
            return Ok(());
        }

        if PERMISSION_REQUESTED.swap(true, Ordering::Relaxed) {
            warn!("Not allowed to show notifications");
            return Ok(());
        }

        // `requestPermissions(new String[] { POST_NOTIFICATIONS }, 0)`. The
        // request code is only ever read back by
        // `onRequestPermissionsResult`, which nothing overrides, so it names
        // nothing.
        let permission = JObject::from(env.new_string(POST_NOTIFICATIONS)?);
        let permissions = env.new_object_array(1, "java/lang/String", JObject::null())?;
        env.set_object_array_element(&permissions, 0, &permission)?;

        env.call_method(
            activity.as_obj(),
            "requestPermissions",
            "([Ljava/lang/String;I)V",
            &[JValue::Object(&JObject::from(permissions)), JValue::Int(0)],
        )?;
        debug!("Asked for permission to show notifications");

        Ok(())
    })
}

/// Whether the user has allowed Commune to post notifications.
fn has_permission(env: &mut AttachGuard<'_>, context: &JObject) -> Result<bool, AndroidJniError> {
    let permission = JObject::from(env.new_string(POST_NOTIFICATIONS)?);

    let result = env
        .call_method(
            context,
            "checkSelfPermission",
            "(Ljava/lang/String;)I",
            &[JValue::Object(&permission)],
        )?
        .i()?;

    Ok(result == PERMISSION_GRANTED)
}

/// The system's `NotificationManager`.
fn notification_manager<'a>(
    env: &mut AttachGuard<'a>,
    context: &JObject,
) -> Result<JObject<'a>, AndroidJniError> {
    // `Context.NOTIFICATION_SERVICE`, whose value is the string "notification".
    // The `Class`-typed overload would be tidier and needs a class object we
    // would have to look up first, for no gain.
    let name = JObject::from(env.new_string("notification")?);

    Ok(env
        .call_method(
            context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&name)],
        )?
        .l()?)
}

/// Show the given notification.
///
/// The ID is the same string the caller would have given
/// `GApplication::send_notification()`, and re-using one replaces the
/// notification it belongs to, as it does there.
///
/// Each button is a label, the name of the application action it activates, and
/// that action's target.
///
/// Must be called on the GTK thread, the only one that can be asked for the
/// `Activity` the application `Context` is first taken from.
pub(crate) fn send(
    id: &str,
    title: &str,
    body: &str,
    action: &str,
    target: &glib::Variant,
    icon: Option<&gdk::Texture>,
    buttons: &[(String, String, glib::Variant)],
) {
    if let Err(error) = show(id, title, body, action, target, icon, buttons) {
        error!(id, "Could not show a notification: {error}");
    }
}

/// The body of [`send()`], so that one place reports what went wrong.
fn show(
    id: &str,
    title: &str,
    body: &str,
    action: &str,
    target: &glib::Variant,
    icon: Option<&gdk::Texture>,
    buttons: &[(String, String, glib::Variant)],
) -> Result<(), AndroidJniError> {
    let context = android::application_context()?;
    let icon_png = icon.map(gdk::Texture::save_to_png_bytes);

    android::with_env(|env| {
        let context = context.as_obj();

        let channel = JObject::from(env.new_string(CHANNEL_ID)?);
        let builder = env.new_object(
            "android/app/Notification$Builder",
            "(Landroid/content/Context;Ljava/lang/String;)V",
            &[JValue::Object(context), JValue::Object(&channel)],
        )?;

        let title = JObject::from(env.new_string(title)?);
        env.call_method(
            &builder,
            "setContentTitle",
            "(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;",
            &[JValue::Object(&title)],
        )?;

        let body = JObject::from(env.new_string(body)?);
        env.call_method(
            &builder,
            "setContentText",
            "(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;",
            &[JValue::Object(&body)],
        )?;

        let small_icon = small_icon(env, context)?;
        env.call_method(
            &builder,
            "setSmallIcon",
            "(I)Landroid/app/Notification$Builder;",
            &[JValue::Int(small_icon)],
        )?;

        // Dismiss the notification when it is tapped, which is what the other
        // platforms do with theirs and what a person expects of one.
        env.call_method(
            &builder,
            "setAutoCancel",
            "(Z)Landroid/app/Notification$Builder;",
            &[JValue::Bool(u8::from(true))],
        )?;

        // `Notification.CATEGORY_MESSAGE`, the counterpart of the `im.received`
        // category the other platforms are given. Do Not Disturb and the
        // shade's own sorting are what read it.
        let category = JObject::from(env.new_string("msg")?);
        env.call_method(
            &builder,
            "setCategory",
            "(Ljava/lang/String;)Landroid/app/Notification$Builder;",
            &[JValue::Object(&category)],
        )?;

        if let Some(png) = &icon_png {
            let bytes = env.byte_array_from_slice(png)?;
            let length = i32::try_from(png.len()).unwrap_or(i32::MAX);
            let bitmap = env
                .call_static_method(
                    "android/graphics/BitmapFactory",
                    "decodeByteArray",
                    "([BII)Landroid/graphics/Bitmap;",
                    &[
                        JValue::Object(&JObject::from(bytes)),
                        JValue::Int(0),
                        JValue::Int(length),
                    ],
                )?
                .l()?;

            // A `Bitmap` that could not be decoded comes back as null rather
            // than as an exception. Worth noticing, not worth refusing the
            // whole notification over.
            if bitmap.is_null() {
                warn!("Could not decode the avatar of a notification");
            } else {
                env.call_method(
                    &builder,
                    "setLargeIcon",
                    "(Landroid/graphics/Bitmap;)Landroid/app/Notification$Builder;",
                    &[JValue::Object(&bitmap)],
                )?;
            }
        }

        let intent = pending_intent(env, context, action, target)?;
        env.call_method(
            &builder,
            "setContentIntent",
            "(Landroid/app/PendingIntent;)Landroid/app/Notification$Builder;",
            &[JValue::Object(&intent)],
        )?;

        for (label, action, target) in buttons {
            let intent = pending_intent(env, context, action, target)?;
            let label = JObject::from(env.new_string(label)?);

            // A null `Icon` is documented as acceptable, and Android has not
            // drawn action icons since API 24 anyway.
            let button = env.new_object(
                "android/app/Notification$Action$Builder",
                "(Landroid/graphics/drawable/Icon;Ljava/lang/CharSequence;Landroid/app/PendingIntent;)V",
                &[
                    JValue::Object(&JObject::null()),
                    JValue::Object(&label),
                    JValue::Object(&intent),
                ],
            )?;
            let button = env
                .call_method(&button, "build", "()Landroid/app/Notification$Action;", &[])?
                .l()?;

            env.call_method(
                &builder,
                "addAction",
                "(Landroid/app/Notification$Action;)Landroid/app/Notification$Builder;",
                &[JValue::Object(&button)],
            )?;
        }

        let notification = env
            .call_method(&builder, "build", "()Landroid/app/Notification;", &[])?
            .l()?;

        let manager = notification_manager(env, context)?;
        let tag = JObject::from(env.new_string(id)?);
        env.call_method(
            &manager,
            "notify",
            "(Ljava/lang/String;ILandroid/app/Notification;)V",
            &[
                JValue::Object(&tag),
                JValue::Int(NOTIFICATION_ID),
                JValue::Object(&notification),
            ],
        )?;

        Ok(())
    })
}

/// The resource ID of the status bar icon.
///
/// Looked up by name rather than read off the generated `R` class, which is
/// Java that the Rust side has no binding for. `getIdentifier()` is discouraged
/// for being slower than a constant, which is true and does not matter once per
/// notification.
fn small_icon(env: &mut AttachGuard<'_>, context: &JObject) -> Result<i32, AndroidJniError> {
    let resources = env
        .call_method(
            context,
            "getResources",
            "()Landroid/content/res/Resources;",
            &[],
        )?
        .l()?;
    let package = env
        .call_method(context, "getPackageName", "()Ljava/lang/String;", &[])?
        .l()?;

    let name = JObject::from(env.new_string(SMALL_ICON_NAME)?);
    let kind = JObject::from(env.new_string("drawable")?);

    let id = env
        .call_method(
            &resources,
            "getIdentifier",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
            &[
                JValue::Object(&name),
                JValue::Object(&kind),
                JValue::Object(&package),
            ],
        )?
        .i()?;

    if id == 0 {
        warn!("No `{SMALL_ICON_NAME}` drawable; falling back to the system's chat icon");
        return Ok(FALLBACK_SMALL_ICON);
    }

    Ok(id)
}

/// A `PendingIntent` that opens Commune on the given action.
fn pending_intent<'a>(
    env: &mut AttachGuard<'a>,
    context: &JObject,
    action: &str,
    target: &glib::Variant,
) -> Result<JObject<'a>, AndroidJniError> {
    let view = JObject::from(env.new_string("android.intent.action.VIEW")?);
    let uri = JObject::from(env.new_string(tap_uri(action, target))?);
    let uri = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&uri)],
        )?
        .l()?;

    let intent = env.new_object(
        "android/content/Intent",
        "(Ljava/lang/String;Landroid/net/Uri;)V",
        &[JValue::Object(&view), JValue::Object(&uri)],
    )?;

    let package = env
        .call_method(context, "getPackageName", "()Ljava/lang/String;", &[])?
        .l()?;
    let class = JObject::from(env.new_string(ACTIVITY_CLASS)?);
    env.call_method(
        &intent,
        "setClassName",
        "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&package), JValue::Object(&class)],
    )?;

    Ok(env
        .call_static_method(
            "android/app/PendingIntent",
            "getActivity",
            "(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
            &[
                JValue::Object(context),
                // The request code only tells two `PendingIntent`s apart when
                // their `Intent`s are otherwise equal, and `Intent.filterEquals`
                // compares the data URI — which differs for every action here.
                // So it names nothing, as above.
                JValue::Int(0),
                JValue::Object(&intent),
                JValue::Int(PENDING_INTENT_FLAGS),
            ],
        )?
        .l()?)
}

/// The URI carrying the given action and target back to
/// `Application::process_uri()`.
fn tap_uri(action: &str, target: &glib::Variant) -> String {
    // `GVariant` text is full of characters a query string cannot carry
    // literally — quotes, commas, an ampersand in a room name — so both halves
    // are escaped, and `glib::Uri::parse_params()` unescapes them again on the
    // way back. What is between the two is a round trip through `GFile`, which
    // is how `GApplication::open()` hands a URI over: GIO decodes it into a
    // `GDummyFile` and re-encodes it on the way out, so the escaping has to
    // survive that and not only the wire.
    let action = glib::Uri::escape_string(action, None, false);
    let target = glib::Uri::escape_string(&target.print(true), None, false);

    format!("{NOTIFICATION_URI}?{ACTION_PARAMETER}={action}&{TARGET_PARAMETER}={target}")
}

/// The action and target carried by the URI of a tapped notification, the
/// target still as printed `GVariant` text.
///
/// The counterpart of [`tap_uri()`], and here rather than with its caller so
/// that the two halves of one format sit together.
///
/// The query is taken apart by hand because `g_uri_parse_params()` is one of
/// the few GLib functions gtk-rs does not bind — it returns a `GHashTable`,
/// which the generator gives up on (`glib-0.22.8/src/auto/uri.rs:291`). Doing
/// it here is a handful of lines and needs no `unsafe`.
pub(crate) fn tapped_action(uri: &str) -> Option<(String, String)> {
    let uri = glib::Uri::parse(uri, glib::UriFlags::NONE).ok()?;
    let query = uri.query()?;

    let mut action = None;
    let mut target = None;

    for parameter in query.split('&') {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        // `None` as the illegal characters means only that no unescaped byte is
        // rejected outright; the escaping itself is undone either way.
        let Some(value) = glib::Uri::unescape_string(value, None) else {
            continue;
        };

        match name {
            ACTION_PARAMETER => action = Some(value.to_string()),
            TARGET_PARAMETER => target = Some(value.to_string()),
            _ => (),
        }
    }

    Some((action?, target?))
}

/// Remove the notification with the given ID, whether it has been shown yet or
/// not.
pub(crate) fn withdraw(id: &str) {
    if let Err(error) = cancel(id) {
        error!(id, "Could not withdraw a notification: {error}");
    }
}

/// The body of [`withdraw()`], so that one place reports what went wrong.
fn cancel(id: &str) -> Result<(), AndroidJniError> {
    let context = android::application_context()?;

    android::with_env(|env| {
        let manager = notification_manager(env, context.as_obj())?;
        let tag = JObject::from(env.new_string(id)?);

        env.call_method(
            &manager,
            "cancel",
            "(Ljava/lang/String;I)V",
            &[JValue::Object(&tag), JValue::Int(NOTIFICATION_ID)],
        )?;

        Ok(())
    })
}

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
//!
//! **A conversation is one notification, not many.** Messages go through
//! [`send_message()`], which keys the notification by room and stacks each
//! new message into a `Notification.MessagingStyle` — the platform's own
//! shape for a chat, sender names and all. The messages already on screen are
//! read back from the active notification rather than remembered here, so the
//! stack survives the process and empties when the user dismisses it. See
//! [`send_message()`] for the rest.

use std::sync::atomic::{AtomicBool, Ordering};

use gettextrs::gettext;
use gtk::{gdk, glib, prelude::*};
use jni::{
    AttachGuard,
    objects::{JObject, JObjectArray, JString, JValue},
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
/// `build-aux/android/patch-notification-icon.sh` writes this between
/// `pixiewood generate` and `pixiewood build`, out of the monochrome layer
/// pixiewood generates from `data/icons/*-symbolic.svg`. A small icon is
/// masked down to its alpha channel and tinted by the system, so a monochrome
/// silhouette is exactly the right shape for one — the full-color launcher
/// icon would come out as a white blob.
///
/// Not `ic_launcher_monochrome` itself, which is that layer unmodified and
/// carries the adaptive launcher icon's inset: it draws the glyph at 45% of
/// its canvas, which is right for a launcher icon and about half the diameter
/// the shade's badge expects. The patch script's header has the measurements.
const SMALL_ICON_NAME: &str = "ic_notification";

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

/// The group key shared by every message notification.
///
/// Without an explicit group, the shade auto-bundles everything of ours under
/// one heading — including the sync service's ongoing notification, whose
/// plain open-the-app intent is what a tap on the collapsed bundle then fires
/// (measured, S5b step 3). With this key, the conversations bundle among
/// themselves under [`SUMMARY_TAG`]'s summary and the ongoing notification
/// stays out of them.
const GROUP_KEY: &str = "conversations";

/// The tag of the summary notification for [`GROUP_KEY`].
///
/// Android requires a group to have a posted summary before it draws the
/// bundle. Real notification tags all carry a `//`, so this cannot collide
/// with one.
const SUMMARY_TAG: &str = "conversations-summary";

/// `Notification.EXTRA_MESSAGES`, where a posted `MessagingStyle` keeps its
/// messages.
const EXTRA_MESSAGES: &str = "android.messages";

/// `Notification.FLAG_GROUP_SUMMARY`.
const FLAG_GROUP_SUMMARY: i32 = 0x200;

/// Whether permission to post notifications has been asked for this run.
///
/// Asking twice is not harmful — Android shows the prompt at most twice ever,
/// and denies silently afterwards — but there is no reason to spend one of
/// those on a request that was already made and answered.
static PERMISSION_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Create the notification channel.
///
/// Called every time the main window is presented, and cheap after the first
/// time: creating a channel that already exists is documented as leaving the
/// existing one alone. Permission is only checked here, not asked for —
/// asking is [`request_permission()`]'s job, from the setup dialog, where the
/// prompt arrives with its reason on screen instead of cold.
pub(crate) fn init(window: &gtk::Window) {
    if let Err(error) = set_up(window) {
        warn!("Could not set notifications up: {error}");
    }
}

/// The body of [`init()`], so that one place reports what went wrong.
fn set_up(window: &gtk::Window) -> Result<(), AndroidJniError> {
    // Not used here, but this is the first moment there is an `Activity` to
    // capture the application `Context` from, and everything below needs it.
    let _ = android::activity(window)?;
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
        } else {
            debug!("Not allowed to show notifications, or not asked yet");
        }

        Ok(())
    })
}

/// Ask for permission to post notifications, once per run.
///
/// Takes the window because that is where the `Activity` comes from, and the
/// `Activity` is what `requestPermissions()` needs — the application `Context`
/// can only be asked whether permission is held, not for permission itself.
pub(crate) fn request_permission(window: &gtk::Window) {
    if let Err(error) = ask_permission(window) {
        warn!("Could not ask for permission to show notifications: {error}");
    }
}

/// The body of [`request_permission()`], so that one place reports what went
/// wrong.
fn ask_permission(window: &gtk::Window) -> Result<(), AndroidJniError> {
    let activity = android::activity(window)?;
    let context = android::application_context()?;

    android::with_env(|env| {
        if has_permission(env, context.as_obj())? {
            debug!("Already allowed to show notifications");
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

        let builder = base_builder(env, context, icon_png.as_deref())?;

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

/// A `Notification.Builder` carrying everything a message-shaped notification
/// shares: the channel, the status bar icon, the tap-to-dismiss behavior, the
/// `msg` category, and the avatar when there is one.
fn base_builder<'a>(
    env: &mut AttachGuard<'a>,
    context: &JObject,
    icon_png: Option<&[u8]>,
) -> Result<JObject<'a>, AndroidJniError> {
    let channel = JObject::from(env.new_string(CHANNEL_ID)?);
    let builder = env.new_object(
        "android/app/Notification$Builder",
        "(Landroid/content/Context;Ljava/lang/String;)V",
        &[JValue::Object(context), JValue::Object(&channel)],
    )?;

    let small_icon = small_icon_resource(env, context)?;
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

    if let Some(png) = icon_png {
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

    Ok(builder)
}

/// Show the given message, stacked into its conversation's notification.
///
/// Where [`send()`] posts one notification per ID, this posts one per
/// conversation: the tag is the room's, and the message joins a
/// `Notification.MessagingStyle` holding whatever the conversation's
/// notification already shows. The already-shown messages are read back out of
/// the active notification itself rather than kept on this side, which is what
/// makes the stacking honest: a push-woken process that never saw the earlier
/// messages still appends to them, and a notification the user dismissed
/// starts over from nothing.
///
/// A message that is already shown — same timestamp, same text — is not posted
/// again at all, so the push-wake path and the sync path, which both come here
/// with the same event, alert once between them.
///
/// Must be called on the GTK thread, like [`send()`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn send_message(
    tag: &str,
    conversation_title: &str,
    self_name: &str,
    sender_name: &str,
    body: &str,
    timestamp_ms: i64,
    is_group_conversation: bool,
    action: &str,
    target: &glib::Variant,
    icon: Option<&gdk::Texture>,
) {
    if let Err(error) = show_message(
        tag,
        conversation_title,
        self_name,
        sender_name,
        body,
        timestamp_ms,
        is_group_conversation,
        action,
        target,
        icon,
    ) {
        error!(tag, "Could not show a message notification: {error}");
    }
}

/// The body of [`send_message()`], so that one place reports what went wrong.
#[allow(clippy::too_many_arguments)]
fn show_message(
    tag: &str,
    conversation_title: &str,
    self_name: &str,
    sender_name: &str,
    body: &str,
    timestamp_ms: i64,
    is_group_conversation: bool,
    action: &str,
    target: &glib::Variant,
    icon: Option<&gdk::Texture>,
) -> Result<(), AndroidJniError> {
    let context = android::application_context()?;
    let icon_png = icon.map(gdk::Texture::save_to_png_bytes);

    android::with_env(|env| {
        let context = context.as_obj();
        let manager = notification_manager(env, context)?;

        let shown = match shown_messages(env, &manager, tag) {
            Ok(shown) => shown,
            Err(error) => {
                // The bundle keys are the framework's own but not API, so a
                // future Android could stop answering to them. Starting the
                // conversation over is better than refusing the new message.
                env.exception_clear()?;
                warn!(tag, "Could not read the shown messages back: {error}");
                Vec::new()
            }
        };

        if shown
            .iter()
            .any(|(text, time, _)| *time == timestamp_ms && text == body)
        {
            // The same event through the second of the two paths; reposting
            // would alert again with nothing new to say.
            debug!(tag, "The pushed event is already shown");
            return Ok(());
        }

        let self_person = person(env, self_name)?;
        let style = env.new_object(
            "android/app/Notification$MessagingStyle",
            "(Landroid/app/Person;)V",
            &[JValue::Object(&self_person)],
        )?;

        for (text, time, sender) in &shown {
            let text = JObject::from(env.new_string(text)?);
            let message = env.new_object(
                "android/app/Notification$MessagingStyle$Message",
                "(Ljava/lang/CharSequence;JLandroid/app/Person;)V",
                &[
                    JValue::Object(&text),
                    JValue::Long(*time),
                    JValue::Object(sender),
                ],
            )?;
            env.call_method(
                &style,
                "addMessage",
                "(Landroid/app/Notification$MessagingStyle$Message;)Landroid/app/Notification$MessagingStyle;",
                &[JValue::Object(&message)],
            )?;
        }

        let sender = person(env, sender_name)?;
        let text = JObject::from(env.new_string(body)?);
        env.call_method(
            &style,
            "addMessage",
            "(Ljava/lang/CharSequence;JLandroid/app/Person;)Landroid/app/Notification$MessagingStyle;",
            &[
                JValue::Object(&text),
                JValue::Long(timestamp_ms),
                JValue::Object(&sender),
            ],
        )?;

        // A conversation title makes the shade render the group-chat layout,
        // so a direct chat gets none — the sender's name is its whole heading.
        // `setGroupConversation` is set explicitly either way because a title
        // alone would imply it.
        if is_group_conversation {
            let title = JObject::from(env.new_string(conversation_title)?);
            env.call_method(
                &style,
                "setConversationTitle",
                "(Ljava/lang/CharSequence;)Landroid/app/Notification$MessagingStyle;",
                &[JValue::Object(&title)],
            )?;
        }
        env.call_method(
            &style,
            "setGroupConversation",
            "(Z)Landroid/app/Notification$MessagingStyle;",
            &[JValue::Bool(u8::from(is_group_conversation))],
        )?;

        let builder = base_builder(env, context, icon_png.as_deref())?;
        env.call_method(
            &builder,
            "setStyle",
            "(Landroid/app/Notification$Style;)Landroid/app/Notification$Builder;",
            &[JValue::Object(&style)],
        )?;

        let group = JObject::from(env.new_string(GROUP_KEY)?);
        env.call_method(
            &builder,
            "setGroup",
            "(Ljava/lang/String;)Landroid/app/Notification$Builder;",
            &[JValue::Object(&group)],
        )?;

        let intent = pending_intent(env, context, action, target)?;
        env.call_method(
            &builder,
            "setContentIntent",
            "(Landroid/app/PendingIntent;)Landroid/app/Notification$Builder;",
            &[JValue::Object(&intent)],
        )?;

        let notification = env
            .call_method(&builder, "build", "()Landroid/app/Notification;", &[])?
            .l()?;

        let tag = JObject::from(env.new_string(tag)?);
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

        post_group_summary(env, context, &manager)?;

        Ok(())
    })
}

/// The messages the conversation notification with the given tag currently
/// shows, oldest first: each one text, timestamp, and sender `Person` (null
/// for a message from the user themself).
///
/// Read out of `EXTRA_MESSAGES` on the active notification. The keys inside
/// each message's `Bundle` — `text`, `time`, `sender_person`, written by
/// `Notification.MessagingStyle.Message.toBundle()` — are the framework's
/// internal ones, unchanged since `Person` arrived in API 28 and relied on by
/// AndroidX's own `extractMessagingStyleFromNotification`, but they are not
/// API: the caller treats a failure here as an empty history, not an error.
fn shown_messages<'a>(
    env: &mut AttachGuard<'a>,
    manager: &JObject,
    tag: &str,
) -> Result<Vec<(String, i64, JObject<'a>)>, AndroidJniError> {
    let actives = env
        .call_method(
            manager,
            "getActiveNotifications",
            "()[Landroid/service/notification/StatusBarNotification;",
            &[],
        )?
        .l()?;
    let actives = JObjectArray::from(actives);
    let count = env.get_array_length(&actives)?;

    for i in 0..count {
        let sbn = env.get_object_array_element(&actives, i)?;

        let sbn_tag = env
            .call_method(&sbn, "getTag", "()Ljava/lang/String;", &[])?
            .l()?;
        // The sync service's notification is posted without a tag.
        if sbn_tag.is_null() {
            continue;
        }
        let sbn_tag = String::from(env.get_string(&JString::from(sbn_tag))?);
        if sbn_tag != tag {
            env.delete_local_ref(sbn)?;
            continue;
        }

        let notification = env
            .call_method(&sbn, "getNotification", "()Landroid/app/Notification;", &[])?
            .l()?;
        let extras = env
            .get_field(&notification, "extras", "Landroid/os/Bundle;")?
            .l()?;

        let key = JObject::from(env.new_string(EXTRA_MESSAGES)?);
        let messages = env
            .call_method(
                &extras,
                "getParcelableArray",
                "(Ljava/lang/String;)[Landroid/os/Parcelable;",
                &[JValue::Object(&key)],
            )?
            .l()?;
        if messages.is_null() {
            return Ok(Vec::new());
        }
        let messages = JObjectArray::from(messages);
        let count = env.get_array_length(&messages)?;

        let mut shown = Vec::with_capacity(count as usize);
        for j in 0..count {
            let bundle = env.get_object_array_element(&messages, j)?;
            if bundle.is_null() {
                continue;
            }

            let text_key = JObject::from(env.new_string("text")?);
            let text = env
                .call_method(
                    &bundle,
                    "getCharSequence",
                    "(Ljava/lang/String;)Ljava/lang/CharSequence;",
                    &[JValue::Object(&text_key)],
                )?
                .l()?;
            if text.is_null() {
                continue;
            }
            let text = env
                .call_method(&text, "toString", "()Ljava/lang/String;", &[])?
                .l()?;
            let text = String::from(env.get_string(&JString::from(text))?);

            let time_key = JObject::from(env.new_string("time")?);
            let time = env
                .call_method(
                    &bundle,
                    "getLong",
                    "(Ljava/lang/String;)J",
                    &[JValue::Object(&time_key)],
                )?
                .j()?;

            let person_key = JObject::from(env.new_string("sender_person")?);
            let sender = env
                .call_method(
                    &bundle,
                    "getParcelable",
                    "(Ljava/lang/String;)Landroid/os/Parcelable;",
                    &[JValue::Object(&person_key)],
                )?
                .l()?;

            shown.push((text, time, sender));
        }

        return Ok(shown);
    }

    Ok(Vec::new())
}

/// An `android.app.Person` with the given name.
fn person<'a>(env: &mut AttachGuard<'a>, name: &str) -> Result<JObject<'a>, AndroidJniError> {
    let builder = env.new_object("android/app/Person$Builder", "()V", &[])?;
    let name = JObject::from(env.new_string(name)?);
    env.call_method(
        &builder,
        "setName",
        "(Ljava/lang/CharSequence;)Landroid/app/Person$Builder;",
        &[JValue::Object(&name)],
    )?;

    Ok(env
        .call_method(&builder, "build", "()Landroid/app/Person;", &[])?
        .l()?)
}

/// Post the summary notification for [`GROUP_KEY`], under which the
/// conversation notifications bundle.
///
/// Posting it again is a cheap replace, so every message posts it. Its tap
/// plainly opens the application: with several conversations collapsed into
/// one card there is no single room to open, and the shade expands the bundle
/// rather than firing this on most Androids anyway. What matters is that the
/// intent is chosen, not inherited from whichever notification the shade
/// happens to promote.
fn post_group_summary(
    env: &mut AttachGuard<'_>,
    context: &JObject,
    manager: &JObject,
) -> Result<(), AndroidJniError> {
    let builder = base_builder(env, context, None)?;

    let group = JObject::from(env.new_string(GROUP_KEY)?);
    env.call_method(
        &builder,
        "setGroup",
        "(Ljava/lang/String;)Landroid/app/Notification$Builder;",
        &[JValue::Object(&group)],
    )?;
    env.call_method(
        &builder,
        "setGroupSummary",
        "(Z)Landroid/app/Notification$Builder;",
        &[JValue::Bool(u8::from(true))],
    )?;

    // `Intent(ACTION_MAIN)` addressed to our own Activity — the launcher's
    // own gesture, which resumes the task where it was.
    let main = JObject::from(env.new_string("android.intent.action.MAIN")?);
    let intent = env.new_object(
        "android/content/Intent",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&main)],
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
    let pending = env
        .call_static_method(
            "android/app/PendingIntent",
            "getActivity",
            "(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
            &[
                JValue::Object(context),
                JValue::Int(0),
                JValue::Object(&intent),
                JValue::Int(PENDING_INTENT_FLAGS),
            ],
        )?
        .l()?;
    env.call_method(
        &builder,
        "setContentIntent",
        "(Landroid/app/PendingIntent;)Landroid/app/Notification$Builder;",
        &[JValue::Object(&pending)],
    )?;

    let notification = env
        .call_method(&builder, "build", "()Landroid/app/Notification;", &[])?
        .l()?;

    let tag = JObject::from(env.new_string(SUMMARY_TAG)?);
    env.call_method(
        manager,
        "notify",
        "(Ljava/lang/String;ILandroid/app/Notification;)V",
        &[
            JValue::Object(&tag),
            JValue::Int(NOTIFICATION_ID),
            JValue::Object(&notification),
        ],
    )?;

    Ok(())
}

/// Whether nothing but the summary itself is left in [`GROUP_KEY`], the
/// just-canceled tag not counting — `cancel()` is asynchronous, so the
/// notification it took may still be listed.
fn group_is_empty(
    env: &mut AttachGuard<'_>,
    manager: &JObject,
    canceled_tag: &str,
) -> Result<bool, AndroidJniError> {
    let actives = env
        .call_method(
            manager,
            "getActiveNotifications",
            "()[Landroid/service/notification/StatusBarNotification;",
            &[],
        )?
        .l()?;
    let actives = JObjectArray::from(actives);
    let count = env.get_array_length(&actives)?;

    for i in 0..count {
        let sbn = env.get_object_array_element(&actives, i)?;
        let notification = env
            .call_method(&sbn, "getNotification", "()Landroid/app/Notification;", &[])?
            .l()?;

        let group = env
            .call_method(&notification, "getGroup", "()Ljava/lang/String;", &[])?
            .l()?;
        if group.is_null() || String::from(env.get_string(&JString::from(group))?) != GROUP_KEY {
            env.delete_local_ref(sbn)?;
            continue;
        }

        let flags = env.get_field(&notification, "flags", "I")?.i()?;
        if flags & FLAG_GROUP_SUMMARY != 0 {
            continue;
        }

        let tag = env
            .call_method(&sbn, "getTag", "()Ljava/lang/String;", &[])?
            .l()?;
        if !tag.is_null() && String::from(env.get_string(&JString::from(tag))?) == canceled_tag {
            continue;
        }

        return Ok(false);
    }

    Ok(true)
}

/// The resource ID of the status bar icon.
///
/// Looked up by name rather than read off the generated `R` class, which is
/// Java that the Rust side has no binding for. `getIdentifier()` is discouraged
/// for being slower than a constant, which is true and does not matter once per
/// notification.
///
/// `pub(super)` because the foreground service wants the same drawable, and one
/// lookup written once is better than the same three JNI calls in two files.
pub(super) fn small_icon_resource(
    env: &mut AttachGuard<'_>,
    context: &JObject,
) -> Result<i32, AndroidJniError> {
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

        // The group's summary is a notification of its own, and one the system
        // will happily keep showing alone after its last conversation is gone.
        if group_is_empty(env, &manager, id)? {
            let summary_tag = JObject::from(env.new_string(SUMMARY_TAG)?);
            env.call_method(
                &manager,
                "cancel",
                "(Ljava/lang/String;I)V",
                &[JValue::Object(&summary_tag), JValue::Int(NOTIFICATION_ID)],
            )?;
        }

        Ok(())
    })
}

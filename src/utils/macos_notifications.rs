//! Native notifications on macOS.
//!
//! Everywhere else this is `GNotification`: the application hands GIO a
//! notification and GIO finds a backend for it. On macOS the backend GIO finds
//! is `GCocoaNotificationBackend`, which is written against
//! `NSUserNotification` — deprecated in 10.14, and by macOS 26 it delivers
//! nothing at all. It is not a matter of the `GLib` version, either: 2.88's
//! `libgio` still names `NSUserNotification` and never mentions
//! `UNUserNotificationCenter`, and links no `UserNotifications.framework` to
//! reach it through.
//!
//! So we do what the media backend and the `matrix:` URL handler did, and talk
//! to the platform ourselves. `UNUserNotificationCenter` is the API macOS still
//! supports, and unlike the Apple Event Manager it is Objective-C only, which
//! is why this file is the one place in the port that needs `objc2`.
//!
//! Two things about it are worth knowing before changing anything here.
//!
//! **Authorization is required and asked for once.** Until the user answers the
//! prompt, `addNotificationRequest` fails with `UNErrorDomain` code 1. The
//! prompt is bound to the bundle identifier _and_ the signing identity, so an
//! ad-hoc signature — a new identity on every build — can make macOS ask again
//! after a rebuild, the same way it does for the Keychain. `doc/macos.md` has
//! the stable self-signed certificate that avoids it.
//!
//! **The delegate has to exist before the app finishes launching.** A tap that
//! launched the application is delivered almost immediately afterwards, and if
//! nothing is listening by then it is delivered to nobody — the same ordering
//! trap as the `'GURL'` handler in [`super::macos_url_events`], which is why
//! [`init()`] is called from `main()` rather than from `Application::startup`.

use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
};

use block2::RcBlock;
use gtk::{gdk, gio, glib, prelude::*};
use objc2::{
    AnyThread, define_class,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString, NSURL,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationAttachment,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tracing::{debug, error, warn};

use super::DataType;

/// The `userInfo` key under which the name of the application action to
/// activate is carried.
const USER_INFO_ACTION_KEY: &str = "commune.action";
/// The `userInfo` key under which the target of that action is carried, as a
/// printed `GVariant`.
///
/// Printing the variant the action already takes, rather than inventing a
/// second encoding for it, keeps [`crate::intent::SessionIntent`] the only
/// description of what a notification means.
const USER_INFO_TARGET_KEY: &str = "commune.target";

thread_local! {
    /// The delegate, kept alive for as long as the process is.
    ///
    /// `UNUserNotificationCenter`'s `delegate` is a weak property, so nothing
    /// on the Objective-C side holds this, and a delegate that is dropped
    /// stops taps from arriving without any error to say so.
    static DELEGATE: RefCell<Option<Retained<NotificationDelegate>>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY:
    // - `NSObject` has no subclassing requirements.
    // - `NotificationDelegate` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CommuneNotificationDelegate"]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        /// Show a banner even when Commune is the active application.
        ///
        /// Without this, macOS drops any notification that arrives while we
        /// are frontmost. That is nearly the right behaviour — but "nearly" is
        /// decided here rather than by the system, because
        /// [`crate::session::Notifications::show_push`] has already made the
        /// finer judgement of whether the room in question is the one on
        /// screen, and anything that reaches this point has passed it.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &objc2_user_notifications::UNNotification,
            completion_handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion_handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::Sound,));
        }

        /// Handle a tap on a notification.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion_handler: &block2::DynBlock<dyn Fn()>,
        ) {
            let user_info = response.notification().request().content().userInfo();
            let action = string_for_key(&user_info, USER_INFO_ACTION_KEY);
            let target = string_for_key(&user_info, USER_INFO_TARGET_KEY);

            if let (Some(action), Some(target)) = (action, target) {
                activate_action(action, target);
            } else {
                warn!("Ignoring a notification tap that carries no action");
            }

            completion_handler.call(());
        }
    }
);

/// Read the string at the given key out of a notification's `userInfo`.
fn string_for_key(user_info: &NSDictionary, key: &str) -> Option<String> {
    let key = NSString::from_str(key);
    let value = user_info.objectForKey(&key)?;

    value
        .downcast::<NSString>()
        .ok()
        .map(|string| string.to_string())
}

/// Activate the application action a tapped notification names.
///
/// This runs on whatever thread `UserNotifications` called the delegate on, so
/// the work is handed to the main context rather than done here. That also
/// takes care of a tap that launched the application: the action is queued and
/// runs once there is a main loop and an `Application` to run it on, which is
/// the same thing the `matrix:` handler relies on.
fn activate_action(action: String, target: String) {
    glib::MainContext::default().invoke(move || {
        let Some(app) = gio::Application::default() else {
            error!("Could not open a notification: the application is gone");
            return;
        };

        // The action was stored with the `app.` prefix it is addressed by, but
        // activating it on the application itself wants the bare name.
        let name = action.strip_prefix("app.").unwrap_or(&action);

        // Reading the target back against the type the action declares, rather
        // than against whatever the string happens to parse as, is what makes
        // the intent types the only description of the payload.
        let Some(parameter_type) = app.action_parameter_type(name) else {
            error!("Could not open a notification: no `{name}` action takes a target");
            return;
        };

        let variant = match glib::Variant::parse(Some(&parameter_type), &target) {
            Ok(variant) => variant,
            Err(error) => {
                error!("Could not read the target of a tapped notification: {error}");
                return;
            }
        };

        debug!(action = name, "Opening a tapped notification");
        app.activate_action(name, Some(&variant));
    });
}

/// Start listening for notification taps, and ask for permission to show
/// notifications at all.
///
/// Must be called on the main thread, before the application finishes
/// launching.
pub(crate) fn init() {
    // Anything left here is the avatar of a notification that was never shown,
    // since macOS takes the file over for the ones that were.
    if let Err(error) = fs::remove_dir_all(icons_dir())
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warn!("Could not clear the notification avatars of the last run: {error}");
    }

    let center = UNUserNotificationCenter::currentNotificationCenter();

    let delegate = NotificationDelegate::alloc().set_ivars(());
    let delegate: Retained<NotificationDelegate> =
        unsafe { objc2::msg_send![super(delegate), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.with_borrow_mut(|slot| *slot = Some(delegate));

    // Ask now rather than when the first notification is sent, so that the
    // prompt is not the thing that swallows it.
    let handler = RcBlock::new(|granted: Bool, error: *mut NSError| {
        if granted.as_bool() {
            debug!("Allowed to show notifications");
        } else {
            // SAFETY: the pointer is the one `UserNotifications` passed us, and
            // is only read while the handler runs.
            let error = unsafe { error.as_ref() };
            let reason = error.map_or_else(
                || "the user said no".to_owned(),
                |error| error.localizedDescription().to_string(),
            );
            warn!("Not allowed to show notifications: {reason}");
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert
            | UNAuthorizationOptions::Sound
            | UNAuthorizationOptions::Badge,
        &handler,
    );

    debug!("Listening for notification taps");
}

/// The directory holding the avatars that notifications are showing.
fn icons_dir() -> PathBuf {
    DataType::Cache.dir_path().join("notification-icons")
}

/// Write the given avatar somewhere a notification can point at it.
///
/// `UNNotificationContent` carries an image as a file URL rather than as
/// pixels, so the texture has to reach the disk before it can be shown.
///
/// The file goes to a directory of its own under the cache, which [`init()`]
/// empties at startup: macOS takes the file over when the request is added, so
/// anything still there afterwards belongs to a request that never made it.
fn write_icon(icon: &gdk::Texture) -> Option<PathBuf> {
    let dir = icons_dir();

    if let Err(error) = fs::create_dir_all(&dir) {
        warn!("Could not make room for a notification avatar: {error}");
        return None;
    }

    // Nothing ever looks this file up again, so it only has to be unique.
    let path = dir.join(format!("{}.png", glib::uuid_string_random()));

    match fs::write(&path, icon.save_to_png_bytes()) {
        Ok(()) => Some(path),
        Err(error) => {
            warn!("Could not write a notification avatar: {error}");
            None
        }
    }
}

/// Make an attachment out of the avatar written to the given path.
fn icon_attachment(path: &Path) -> Option<Retained<UNNotificationAttachment>> {
    let path = NSString::from_str(path.to_str()?);
    let url = NSURL::fileURLWithPath(&path);

    // SAFETY: no options are passed, so there is no generic to get wrong.
    let attachment = unsafe {
        UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(
            // An empty identifier asks macOS to generate one.
            &NSString::from_str(""),
            &url,
            None,
        )
    };

    match attachment {
        Ok(attachment) => Some(attachment),
        Err(error) => {
            warn!(
                "Could not attach an avatar to a notification: {}",
                error.localizedDescription()
            );
            None
        }
    }
}

/// Show the given notification.
///
/// The ID is the same string the caller would have given
/// `GApplication::send_notification()`, and re-using one replaces the
/// notification it belongs to, as it does there.
pub(crate) fn send(
    id: &str,
    title: &str,
    body: &str,
    action: &str,
    target: &glib::Variant,
    icon: Option<&gdk::Texture>,
) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));

    if let Some(attachment) = icon
        .and_then(write_icon)
        .as_deref()
        .and_then(icon_attachment)
    {
        content.setAttachments(&NSArray::from_retained_slice(&[attachment]));
    }

    let action_key = NSString::from_str(USER_INFO_ACTION_KEY);
    let target_key = NSString::from_str(USER_INFO_TARGET_KEY);
    let action = NSString::from_str(action);
    let target = NSString::from_str(&target.print(true));

    let user_info = NSDictionary::from_slices(&[&*action_key, &*target_key], &[&*action, &*target]);
    // SAFETY: `userInfo` is untyped on the Objective-C side, and a dictionary
    // of strings keyed by strings is a property list, which is what it
    // requires.
    let user_info = unsafe { Retained::cast_unchecked::<NSDictionary>(user_info) };
    // SAFETY: as above -- everything in the dictionary is a property list type.
    unsafe { content.setUserInfo(&user_info) };

    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(id),
        &content,
        // No trigger means show it now.
        None,
    );

    let id = id.to_owned();
    let handler = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: the pointer is the one `UserNotifications` passed us, and is
        // only read while the handler runs.
        if let Some(error) = unsafe { error.as_ref() } {
            error!(
                id,
                "Could not show a notification: {}",
                error.localizedDescription()
            );
        }
    });

    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&handler));
}

/// Remove the notification with the given ID, whether it has been shown yet or
/// not.
pub(crate) fn withdraw(id: &str) {
    let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
    let center = UNUserNotificationCenter::currentNotificationCenter();

    center.removeDeliveredNotificationsWithIdentifiers(&ids);
    center.removePendingNotificationRequestsWithIdentifiers(&ids);
}

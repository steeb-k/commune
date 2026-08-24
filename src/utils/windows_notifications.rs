//! Notifications through the Windows toast API.
//!
//! `GLib` has a Windows notification backend, and since claiming an
//! `AppUserModelID` in [`super::windows_app_id`] it does deliver a banner. What
//! it will not do is carry an action — it says so:
//!
//! ```text
//! GLib-GIO-WARNING: Notification actions are unsupported by this Windows backend
//! ```
//!
//! For a chat client that is most of the point. Every notification Commune
//! sends exists to be clicked, and a banner that cannot be is only a
//! distraction. It also emits no buttons and no sender avatar, so a message
//! arrives looking like a system alert rather than like somebody talking.
//!
//! So this sends the toasts itself, through `ToastNotificationManager`. That is
//! the same API `GLib`'s backend uses, reached directly so that the parts it
//! does not expose are available: the `launch` payload that survives a click,
//! the `<actions>` that make buttons, `appLogoOverride` for the avatar, and
//! `ToastNotificationHistory` for withdrawing one when its room is read.
//!
//! The intent travels the way it does on macOS, and for the same reason:
//! `glib::Variant::print(true)` of the action's target, parsed back against the
//! type the action itself declares. One representation, and the intent types
//! stay the only description of the payload.
//!
//! **Only while the application is running.** A click that has to start
//! Commune first is delivered to a COM class registered as the toast's
//! activator, which does not exist yet; until it does, such a click starts the
//! application and is then dropped. See `doc/windows.md`.

use std::{fs, path::PathBuf};

use gtk::{gdk, gio, glib, prelude::*};
use tracing::{debug, error, warn};
use windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{
        ToastActivatedEventArgs, ToastNotification, ToastNotificationManager, ToastNotifier,
    },
    core::{HSTRING, IInspectable, Interface},
};

use crate::{APP_ID, utils::DataType};

/// Every toast we send is put in this group, so that the history can be asked
/// about ours without touching anybody else's.
const GROUP: &str = "commune";

/// Prepare the process to send notifications.
///
/// Empties the directory of avatars from the last run. Windows reads an image
/// off the disk when it shows a toast rather than taking a copy, so the files
/// have to outlive the call — but not the run that made them.
pub(crate) fn init() {
    let dir = icons_dir();
    if dir.exists()
        && let Err(error) = fs::remove_dir_all(&dir)
    {
        warn!("Could not clear the notification avatars: {error}");
    }
}

/// The notifier that sends on our behalf.
///
/// This is looked up per call rather than kept: it is cheap, and holding one
/// across the lifetime of the process is a handle to a COM object that has to
/// be released on the thread that made it.
fn notifier() -> Option<ToastNotifier> {
    match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID)) {
        Ok(notifier) => Some(notifier),
        Err(error) => {
            // The usual cause is an application user model ID that Windows does
            // not recognise, which `windows_app_id::init()` is what prevents.
            error!("Could not create a toast notifier: {error}");
            None
        }
    }
}

/// The directory holding the avatars that notifications are showing.
fn icons_dir() -> PathBuf {
    DataType::Cache.dir_path().join("notification-icons")
}

/// Write the given avatar somewhere a toast can point at it.
///
/// A toast carries an image as a URI rather than as pixels, so the texture has
/// to reach the disk before it can be shown.
fn write_icon(icon: &gdk::Texture) -> Option<PathBuf> {
    let dir = icons_dir();
    if let Err(error) = fs::create_dir_all(&dir) {
        warn!("Could not create the notification avatars directory: {error}");
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

/// The tag identifying the toast for the given notification ID.
///
/// Windows caps a tag at 64 characters and the IDs the caller generates are
/// longer than that — they carry a session, a room and sometimes an event — so
/// the tag is a digest of the ID rather than the ID. It only has to be stable
/// and unique, which a digest is, and it is what [`withdraw()`] finds a toast
/// by afterwards.
fn tag_for(id: &str) -> String {
    glib::compute_checksum_for_string(glib::ChecksumType::Sha256, id).map_or_else(
        // SHA-256 is always available, so this is unreachable — but a
        // notification is not worth a panic, and the tail of an ID is stable
        // and distinctive enough to stand in for a digest of it.
        || id.chars().rev().take(32).collect(),
        |digest| digest.chars().take(32).collect(),
    )
}

/// Escape a string for use as XML text or in an attribute.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Pack an action and its target into the one string a toast hands back when
/// it is clicked.
///
/// The target is `glib::Variant::print(true)`, which can contain anything at
/// all, so the two are separated by a newline — an action name never has one,
/// so the first newline is unambiguously the boundary.
fn pack(action: &str, target: &glib::Variant) -> String {
    format!("{action}\n{}", target.print(true))
}

/// Undo [`pack()`].
fn unpack(payload: &str) -> Option<(&str, &str)> {
    payload.split_once('\n')
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
    buttons: &[(String, glib::Variant, String)],
) {
    let Some(notifier) = notifier() else {
        return;
    };

    // The avatar, if there is one. `hint-crop="circle"` is what makes it read
    // as a person rather than as a logo.
    let avatar = icon.and_then(write_icon).and_then(|path| {
        let uri = glib::filename_to_uri(&path, None).ok()?;
        Some(format!(
            r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#,
            escape(&uri)
        ))
    });

    let mut actions = String::new();
    for (label, target, action) in buttons {
        use std::fmt::Write as _;

        // Writing into a string cannot fail.
        let _ = write!(
            actions,
            r#"<action content="{}" arguments="{}" activationType="foreground"/>"#,
            escape(label),
            escape(&pack(action, target))
        );
    }

    let xml = format!(
        r#"<toast launch="{launch}" activationType="foreground">
            <visual><binding template="ToastGeneric">
                <text>{title}</text>
                <text>{body}</text>
                {avatar}
            </binding></visual>
            <actions>{actions}</actions>
        </toast>"#,
        launch = escape(&pack(action, target)),
        title = escape(title),
        body = escape(body),
        avatar = avatar.unwrap_or_default(),
    );

    let document = XmlDocument::new().and_then(|document| {
        document.LoadXml(&HSTRING::from(&xml))?;
        Ok(document)
    });
    let document = match document {
        Ok(document) => document,
        Err(error) => {
            error!("Could not build a notification: {error}");
            return;
        }
    };

    if let Err(error) = show(&notifier, &document, id) {
        error!("Could not show a notification: {error}");
    }
}

/// Build the toast and hand it to Windows.
fn show(notifier: &ToastNotifier, document: &XmlDocument, id: &str) -> windows::core::Result<()> {
    let toast = ToastNotification::CreateToastNotification(document)?;

    // Tag and group are how a toast is found again, and re-using a tag replaces
    // the toast that had it — which is the overwriting the caller expects from
    // re-using an ID.
    toast.SetTag(&HSTRING::from(tag_for(id)))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;

    toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
        |_toast, args| {
            if let Some(payload) = clicked_payload(args.as_ref()) {
                activate(&payload);
            }
            Ok(())
        },
    ))?;

    notifier.Show(&toast)
}

/// What the click handed back: the toast's own `launch`, or a button's
/// `arguments`.
fn clicked_payload(args: Option<&IInspectable>) -> Option<String> {
    let args = args?.cast::<ToastActivatedEventArgs>().ok()?;
    let arguments = args.Arguments().ok()?.to_string_lossy();

    (!arguments.is_empty()).then_some(arguments)
}

/// Carry out what a clicked notification asked for.
///
/// Reading the target back against the type the action declares, rather than
/// against whatever the string happens to parse as, is what makes the intent
/// types the only description of the payload.
fn activate(payload: &str) {
    let Some((action, target)) = unpack(payload) else {
        error!("Could not open a notification: its payload has no action");
        return;
    };
    let (action, target) = (action.to_owned(), target.to_owned());

    // The click arrives on a thread of Windows' choosing, and everything below
    // touches the application.
    glib::MainContext::default().invoke(move || {
        let Some(app) = gio::Application::default() else {
            error!("Could not open a notification: the application is gone");
            return;
        };

        // The action was stored with the `app.` prefix it is addressed by, but
        // activating it on the application itself wants the bare name.
        let name = action.strip_prefix("app.").unwrap_or(&action);

        let Some(parameter_type) = app.action_parameter_type(name) else {
            error!("Could not open a notification: no `{name}` action takes a target");
            return;
        };

        let variant = match glib::Variant::parse(Some(&parameter_type), &target) {
            Ok(variant) => variant,
            Err(error) => {
                error!("Could not read the target of a clicked notification: {error}");
                return;
            }
        };

        debug!(action = name, "Opening a clicked notification");
        app.activate_action(name, Some(&variant));
    });
}

/// Ask Windows to remove the notification with the given ID.
pub(crate) fn withdraw(id: &str) {
    let history = match ToastNotificationManager::History() {
        Ok(history) => history,
        Err(error) => {
            error!("Could not reach the notification history: {error}");
            return;
        }
    };

    if let Err(error) = history.RemoveGroupedTagWithId(
        &HSTRING::from(tag_for(id)),
        &HSTRING::from(GROUP),
        &HSTRING::from(APP_ID),
    ) {
        // Removing one that is no longer there is not an error worth reporting
        // loudly: a notification the user has already dismissed is exactly the
        // state we were asking for.
        debug!("Could not withdraw a notification: {error}");
    }
}

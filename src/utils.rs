use sourceview::prelude::BufferExt;

/// FIXME: This should be addressed in ruma directly
#[macro_export]
macro_rules! fn_event {
    ( $event:ident, $fun:ident ) => {
        match &$event {
            AnyRoomEvent::Message(event) => event.$fun(),
            AnyRoomEvent::State(event) => event.$fun(),
            AnyRoomEvent::RedactedMessage(event) => event.$fun(),
            AnyRoomEvent::RedactedState(event) => event.$fun(),
        }
    };
}

/// FIXME: This should be addressed in ruma directly
#[macro_export]
macro_rules! event_from_sync_event {
    ( $event:ident, $room_id:ident) => {
        match $event {
            AnySyncRoomEvent::Message(event) => {
                AnyRoomEvent::Message(event.into_full_event($room_id.clone()))
            }
            AnySyncRoomEvent::State(event) => {
                AnyRoomEvent::State(event.into_full_event($room_id.clone()))
            }
            AnySyncRoomEvent::RedactedMessage(event) => {
                AnyRoomEvent::RedactedMessage(event.into_full_event($room_id.clone()))
            }
            AnySyncRoomEvent::RedactedState(event) => {
                AnyRoomEvent::RedactedState(event.into_full_event($room_id.clone()))
            }
        }
    };
}

/// Spawn a future on the default `MainContext`
///
/// This was taken from `gtk-macors`
/// but allows setting optionally the priority
///
/// FIXME: this should maybe be upstreamed
#[macro_export]
macro_rules! spawn {
    ($future:expr) => {
        let ctx = glib::MainContext::default();
        ctx.spawn_local($future);
    };
    ($priority:expr, $future:expr) => {
        let ctx = glib::MainContext::default();
        ctx.spawn_local_with_priority($priority, $future);
    };
}

/// Spawn a future on the tokio runtime
#[macro_export]
macro_rules! spawn_tokio {
    ($future:expr) => {
        crate::RUNTIME.spawn($future)
    };
}

use std::convert::TryInto;
use std::path::PathBuf;
use std::str::FromStr;

use gettextrs::gettext;
use gtk::gio::{self, prelude::*};
use gtk::glib::{self, Object};
use matrix_sdk::media::MediaType;
use matrix_sdk::ruma::UInt;
use mime::Mime;

/// Returns an expression looking up the given property on `object`.
pub fn prop_expr<T: IsA<Object>>(object: &T, prop: &str) -> gtk::Expression {
    let obj_expr = gtk::ConstantExpression::new(object).upcast();
    gtk::PropertyExpression::new(T::static_type(), Some(&obj_expr), prop).upcast()
}

// Returns an expression that is the and’ed result of the given boolean expressions.
#[allow(dead_code)]
pub fn and_expr(a_expr: gtk::Expression, b_expr: gtk::Expression) -> gtk::Expression {
    gtk::ClosureExpression::new(
        move |args| {
            let a: bool = args[1].get().unwrap();
            let b: bool = args[2].get().unwrap();
            a && b
        },
        &[a_expr, b_expr],
    )
    .upcast()
}

// Returns an expression that is the or’ed result of the given boolean expressions.
pub fn or_expr(a_expr: gtk::Expression, b_expr: gtk::Expression) -> gtk::Expression {
    gtk::ClosureExpression::new(
        move |args| {
            let a: bool = args[1].get().unwrap();
            let b: bool = args[2].get().unwrap();
            a || b
        },
        &[a_expr, b_expr],
    )
    .upcast()
}

// Returns an expression that is the inverted result of the given boolean expressions.
#[allow(dead_code)]
pub fn not_expr(a_expr: gtk::Expression) -> gtk::Expression {
    gtk::ClosureExpression::new(
        move |args| {
            let a: bool = args[1].get().unwrap();
            !a
        },
        &[a_expr],
    )
    .upcast()
}

pub fn cache_dir() -> PathBuf {
    let mut path = glib::user_cache_dir();
    path.push("fractal");

    if !path.exists() {
        let dir = gio::File::for_path(path.clone());
        dir.make_directory_with_parents(gio::NONE_CANCELLABLE)
            .unwrap();
    }

    path
}

/// Converts a `UInt` to `i32`.
///
/// Returns `-1` if the conversion didn't work.
pub fn uint_to_i32(u: Option<UInt>) -> i32 {
    u.and_then(|ui| {
        let u: Option<u16> = ui.try_into().ok();
        u
    })
    .and_then(|u| {
        let i: i32 = u.into();
        Some(i)
    })
    .unwrap_or(-1)
}

pub fn setup_style_scheme(buffer: &sourceview::Buffer) {
    let manager = adw::StyleManager::default().unwrap();

    buffer.set_style_scheme(style_scheme().as_ref());

    manager.connect_dark_notify(glib::clone!(@weak buffer => move |_| {
        buffer.set_style_scheme(style_scheme().as_ref());
    }));
}

pub fn style_scheme() -> Option<sourceview::StyleScheme> {
    let manager = adw::StyleManager::default().unwrap();
    let scheme_name = if manager.is_dark() {
        "Adwaita-dark"
    } else {
        "Adwaita"
    };

    sourceview::StyleSchemeManager::default().and_then(|scm| scm.scheme(scheme_name))
}

/// Get the unique id of the given `MediaType`.
///
/// It is built from the underlying `MxcUri` and can be safely used in a filename.
///
/// The id is not guaranteed to be unique for malformed `MxcUri`s.
pub fn media_type_uid(media_type: Option<MediaType>) -> String {
    if let Some(mxc) = media_type
        .map(|media_type| match media_type {
            MediaType::Uri(uri) => uri,
            MediaType::Encrypted(file) => file.url,
        })
        .filter(|mxc| mxc.is_valid())
    {
        format!("{}_{}", mxc.server_name().unwrap(), mxc.media_id().unwrap())
    } else {
        "media_uid".to_owned()
    }
}

/// Get a default filename for a mime type.
///
/// Tries to guess the file extension, but it might not find it.
///
/// If the mime type is unknown, it uses the name for `fallback`. The fallback
/// mime types that are recognized are `mime::IMAGE`, `mime::VIDEO`
/// and `mime::AUDIO`, other values will behave the same as `None`.
pub fn filename_for_mime(mime_type: Option<&str>, fallback: Option<mime::Name>) -> String {
    let (type_, extension) = if let Some(mime) = mime_type.and_then(|m| Mime::from_str(m).ok()) {
        let extension =
            mime_guess::get_mime_extensions(&mime).map(|extensions| extensions[0].to_owned());

        (Some(mime.type_().as_str().to_owned()), extension)
    } else {
        (fallback.map(|type_| type_.as_str().to_owned()), None)
    };

    let name = match type_.as_deref() {
        // Translators: Default name for image files.
        Some("image") => gettext("image"),
        // Translators: Default name for video files.
        Some("video") => gettext("video"),
        // Translators: Default name for audio files.
        Some("audio") => gettext("audio"),
        // Translators: Default name for files.
        _ => gettext("file"),
    };

    extension
        .map(|extension| format!("{}.{}", name, extension))
        .unwrap_or(name)
}

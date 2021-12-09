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

use gtk::gio::{self, prelude::*};
use gtk::glib::{self, Object};
use matrix_sdk::ruma::UInt;

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

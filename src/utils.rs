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

use gtk::gio::prelude::*;
use gtk::glib::Object;

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

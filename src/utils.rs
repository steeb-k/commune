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

use crate::RUNTIME;
use gtk::gio::prelude::*;
use gtk::glib::{self, Object};
use std::future::Future;
/// Execute a future on a tokio runtime and spawn a future on the local thread to handle the result
pub fn do_async<
    R: Send + 'static,
    F1: Future<Output = R> + Send + 'static,
    F2: Future<Output = ()> + 'static,
    FN: FnOnce(R) -> F2 + 'static,
>(
    priority: glib::source::Priority,
    tokio_fut: F1,
    glib_closure: FN,
) {
    let (sender, receiver) = futures::channel::oneshot::channel();

    glib::MainContext::default().spawn_local_with_priority(priority, async move {
        glib_closure(receiver.await.unwrap()).await
    });

    RUNTIME.spawn(async move { sender.send(tokio_fut.await) });
}

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

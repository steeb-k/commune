//! The headless Commune core.
//!
//! Everything here must build with no GTK, no glib and no display: this
//! crate is what the Kotlin UI reaches over `UniFFI` and what the GTK UI
//! will consume directly once Track 3 of `doc/kotlin-plan.md` lands.
//! The extraction chunks populate it module by module; the modules below
//! are chunk 1, lifted from `src/` with their `GObject` and gettext touches
//! removed and nothing else changed.

use std::sync::LazyLock;

pub mod config;
pub mod events;
pub mod http;
pub mod matrix;
pub mod paths;
pub mod platform;
pub mod secret;
pub mod session;
pub mod session_list;
pub mod settings;
pub mod tls;
pub mod utils;

uniffi::setup_scaffolding!();

/// The default tokio runtime to be used for async tasks.
///
/// Public so that the `spawn_tokio!` macro works from a consuming crate;
/// reach it through the macro rather than directly.
#[doc(hidden)]
pub static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Runtime::new().expect("creating tokio runtime should succeed")
});

/// Spawn a future on the tokio runtime.
#[macro_export]
macro_rules! spawn_tokio {
    ($future:expr) => {
        $crate::RUNTIME.spawn($future)
    };
}

/// An error whose message is meant to be read by a person.
///
/// The application used `gettext` inside these implementations; the core
/// returns plain English and leaves translation to the UI layer, which is
/// the arrangement `doc/kotlin-plan.md` records under "Strings".
pub trait UserFacingError {
    /// A human-readable message for this error.
    fn to_user_facing(&self) -> String;
}

/// The core's own version, exported so the very first generated Kotlin
/// binding has something real to call.
#[uniffi::export]
#[must_use]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

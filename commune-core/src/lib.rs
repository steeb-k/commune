//! The headless Commune core.
//!
//! Everything here must build with no GTK, no glib and no display: this
//! crate is what the Kotlin UI reaches over UniFFI and what the GTK UI
//! will consume directly once Track 3 of `doc/kotlin-plan.md` lands.
//! The extraction chunks populate it module by module; until then it
//! exports just enough surface to prove the bindings pipeline end to end.

uniffi::setup_scaffolding!();

/// The core's own version, exported so the very first generated Kotlin
/// binding has something real to call.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

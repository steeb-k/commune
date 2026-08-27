//! What the embedding application tells the core about itself.
//!
//! The GTK application bakes `APP_ID` and `PROFILE` in at build time from
//! Meson configuration, and derives its data directories from `GLib`. The core
//! cannot know any of that, so the embedder hands the whole set over once,
//! before anything else is called: the GTK side passes its Meson values and
//! `GLib` paths through, and the Kotlin side passes the application id and the
//! `Context`'s `getNoBackupFilesDir()`/`getCacheDir()` — which on Android is
//! the entire replacement for the XDG-derivation contortions documented in
//! the application's `utils::DataType`.

use std::{path::PathBuf, sync::OnceLock};

/// The values the embedder provides.
#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// The application id (e.g. `io.github.steeb_k.Commune`), which
    /// namespaces everything stored on the platform's secret backend.
    /// It must already carry the profile suffix, as the application's ids do.
    pub app_id: String,
    /// The build profile name stored in secret attributes on Linux
    /// (`stable`, `devel`, `hack`).
    pub profile: String,
    /// The directory persistent data lives under, profile already applied.
    pub data_dir: PathBuf,
    /// The directory cached data lives under, profile already applied.
    pub cache_dir: PathBuf,
}

/// The embedder's configuration, set once at startup.
static CONFIG: OnceLock<CoreConfig> = OnceLock::new();

/// Provide the core with the embedder's configuration.
///
/// Must be called once, before anything else in this crate. A second call is
/// ignored, which makes process re-entry (an Android broadcast starting the
/// process a second way) harmless.
pub fn init(config: CoreConfig) {
    let _ = CONFIG.set(config);
}

/// The configuration the embedder provided.
///
/// # Panics
///
/// If [`init()`] has not run. That is a programming error in the embedder,
/// not a runtime condition, so it is not an error value.
pub(crate) fn get() -> &'static CoreConfig {
    CONFIG
        .get()
        .expect("commune_core::config::init() must be called before using the core")
}

/// The application id.
#[must_use]
pub fn app_id() -> &'static str {
    &get().app_id
}

/// The build profile name.
#[must_use]
pub fn profile() -> &'static str {
    &get().profile
}

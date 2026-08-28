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

use std::{fmt, path::PathBuf, sync::Arc, sync::OnceLock};

use crate::settings::{FileSettingsStore, SettingsStore};

/// The values the embedder provides.
#[derive(Clone)]
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
    /// Where named settings live. `None` keeps them in a JSON file under
    /// `data_dir` ([`FileSettingsStore`]); the GTK application passes its
    /// `GSettings` here instead.
    pub settings_store: Option<Arc<dyn SettingsStore>>,
}

impl fmt::Debug for CoreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoreConfig")
            .field("app_id", &self.app_id)
            .field("profile", &self.profile)
            .field("data_dir", &self.data_dir)
            .field("cache_dir", &self.cache_dir)
            .field("settings_store", &self.settings_store.is_some())
            .finish()
    }
}

/// The embedder's configuration with the defaults resolved.
pub(crate) struct ResolvedConfig {
    /// The application id.
    pub(crate) app_id: String,
    /// The build profile name.
    pub(crate) profile: String,
    /// The directory persistent data lives under.
    pub(crate) data_dir: PathBuf,
    /// The directory cached data lives under.
    pub(crate) cache_dir: PathBuf,
    /// Where named settings live.
    pub(crate) settings_store: Arc<dyn SettingsStore>,
}

/// The embedder's configuration, set once at startup.
static CONFIG: OnceLock<ResolvedConfig> = OnceLock::new();

/// Provide the core with the embedder's configuration.
///
/// Must be called once, before anything else in this crate. A second call is
/// ignored, which makes process re-entry (an Android broadcast starting the
/// process a second way) harmless.
pub fn init(config: CoreConfig) {
    let CoreConfig {
        app_id,
        profile,
        data_dir,
        cache_dir,
        settings_store,
    } = config;

    let settings_store =
        settings_store.unwrap_or_else(|| Arc::new(FileSettingsStore::new(&data_dir)));

    let _ = CONFIG.set(ResolvedConfig {
        app_id,
        profile,
        data_dir,
        cache_dir,
        settings_store,
    });
}

/// The configuration the embedder provided.
///
/// # Panics
///
/// If [`init()`] has not run. That is a programming error in the embedder,
/// not a runtime condition, so it is not an error value.
pub(crate) fn get() -> &'static ResolvedConfig {
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

/// Where named settings live.
pub(crate) fn settings_store() -> Arc<dyn SettingsStore> {
    get().settings_store.clone()
}

/// Initialize the configuration for this crate's tests.
///
/// The config is process-global and set once, so every test that needs it
/// calls this and they all share the same values.
#[cfg(test)]
pub(crate) fn init_test_config() {
    init(CoreConfig {
        app_id: "io.github.steeb_k.Commune.CoreTest".to_owned(),
        profile: "test".to_owned(),
        data_dir: std::env::temp_dir().join("commune-core-test").join("data"),
        cache_dir: std::env::temp_dir().join("commune-core-test").join("cache"),
        settings_store: None,
    });
}

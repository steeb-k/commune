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

use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use url::Url;

use crate::settings::{FileSettingsStore, SettingsStore};

/// How the embedder registers itself as an OAuth 2.0 client.
///
/// A login through the OAuth 2.0 API ends with the browser redirected back
/// to the application, and where it can be redirected to is an embedder
/// fact: the desktop application listens on a loopback address, and
/// registers the IPv4 and IPv6 loopback URIs; Android has no loopback a
/// browser will follow, so it registers a fixed custom-scheme URI, alone,
/// since it has to match exactly what the authorization request sends. The
/// client URI is checked by matrix.org's authorization server against a
/// custom scheme read as reverse DNS, so it is the embedder's too — see the
/// application's `client_registration_data` for the whole story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClientConfig {
    /// The URI identifying the client — the project's homepage, or a page
    /// under the domain the redirect scheme is derived from.
    pub client_uri: Url,
    /// The redirect URIs to register.
    pub redirect_uris: Vec<Url>,
}

/// The values the embedder provides.
#[derive(Clone)]
pub struct CoreConfig {
    /// The application id (e.g. `io.github.steeb_k.Commune`), which
    /// namespaces everything stored on the platform's secret backend.
    /// It must already carry the profile suffix, as the application's ids do.
    pub app_id: String,
    /// The name of the application, as it presents itself to homeservers:
    /// the display name of a device it logs in as, the name of the OAuth 2.0
    /// client it registers, the application name on the pusher it sets.
    ///
    /// Not translated, on purpose: these are read in other clients' session
    /// lists and in authorization pages, by people who may not share this
    /// user's locale.
    pub app_name: String,
    /// The name the pusher gives this device, shown in other clients'
    /// session lists when the account audits what pushes to it.
    ///
    /// `None` uses the application name alone. The Android embedder says
    /// which platform it is; the desktop never registers a pusher.
    pub device_display_name: Option<String>,
    /// The embedder's OAuth 2.0 client registration.
    pub oauth_client: OAuthClientConfig,
    /// The build profile name stored in secret attributes on Linux
    /// (`stable`, `devel`, `hack`).
    pub profile: String,
    /// The number of commits behind this build.
    ///
    /// The version alone cannot order two builds: every nightly in a series
    /// carries the same one, and `crate::updates` has to be able to tell
    /// them apart. The embedder knows this number because its build system
    /// already asks git for it — Meson at configure time, Gradle for
    /// `versionCode`, `bundle.sh` for `CFBundleVersion` — and this crate
    /// cannot ask, because a compiled crate has no idea when it was
    /// compiled. `0` is the honest answer for a build that has no count,
    /// and means the update check will only ever see a different version as
    /// newer, never a different build of the same one.
    pub build_number: u64,
    /// The directory persistent data lives under, profile already applied.
    pub data_dir: PathBuf,
    /// The directory cached data lives under, profile already applied.
    pub cache_dir: PathBuf,
    /// Where named settings live. `None` keeps them in a JSON file under
    /// `data_dir` ([`FileSettingsStore`]); the GTK application passes its
    /// `GSettings` here instead.
    pub settings_store: Option<Arc<dyn SettingsStore>>,
    /// The label the platform's secret backend shows for a stored session, as
    /// a template with `{user_id}` standing in for the Matrix ID.
    ///
    /// It is the only sentence this crate writes that a person reads outside
    /// the application — in Seahorse, in Keychain Access — and the GTK
    /// application has always translated it. The core cannot: `gettext`
    /// belongs to the UI layer, which is the rule `doc/track3-convergence.md`
    /// states as anything that renders a sentence staying where `gettext` can
    /// reach it. So the embedder hands the sentence over already translated
    /// and the core only substitutes into it. `None` uses the English below,
    /// which is what the untranslated Kotlin application wants today.
    pub credential_label: Option<String>,
    /// The KLIPY API key the GIF search uses.
    ///
    /// It is a credential, so it is not allowed to be in this repository:
    /// the GTK application takes it from the `klipy-api-key` Meson option
    /// into a generated, git-ignored `src/config.rs`, and the Kotlin
    /// application takes it from a Gradle property into `BuildConfig`. Both
    /// default to empty. `None` or an empty string means
    /// [`crate::klipy::is_available()`] is `false` and the feature is inert,
    /// which is what a build by anyone without a key of their own gets.
    pub klipy_api_key: Option<String>,
    /// The name of the room this client creates image packs in, and its
    /// topic.
    ///
    /// Written into `m.room.name` and `m.room.topic` on the server the one
    /// time the room is created, and shown in the sidebar beside the
    /// conversations from then on — a user whose room was created in one
    /// language keeps that name after they change it, because nothing
    /// re-creates the room. So the embedder hands them over translated, as
    /// with `credential_label`; `None` uses the English below, which is
    /// what the untranslated Kotlin application wants today.
    pub packs_room_name: Option<String>,
    /// See `packs_room_name`.
    pub packs_room_topic: Option<String>,
}

impl fmt::Debug for CoreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoreConfig")
            .field("app_id", &self.app_id)
            .field("app_name", &self.app_name)
            .field("device_display_name", &self.device_display_name)
            .field("oauth_client", &self.oauth_client)
            .field("profile", &self.profile)
            .field("build_number", &self.build_number)
            .field("data_dir", &self.data_dir)
            .field("cache_dir", &self.cache_dir)
            .field("settings_store", &self.settings_store.is_some())
            .field("credential_label", &self.credential_label)
            // Never the value: it is a credential, and this type is Debug.
            .field("klipy_api_key", &self.klipy_api_key.is_some())
            .field("packs_room_name", &self.packs_room_name)
            .field("packs_room_topic", &self.packs_room_topic)
            .finish()
    }
}

/// The embedder's configuration with the defaults resolved.
pub(crate) struct ResolvedConfig {
    /// The application id.
    pub(crate) app_id: String,
    /// The name of the application.
    pub(crate) app_name: String,
    /// The name the pusher gives this device.
    pub(crate) device_display_name: String,
    /// The embedder's OAuth 2.0 client registration.
    pub(crate) oauth_client: OAuthClientConfig,
    /// The build profile name.
    pub(crate) profile: String,
    /// The number of commits behind this build.
    pub(crate) build_number: u64,
    /// The directory persistent data lives under.
    pub(crate) data_dir: PathBuf,
    /// The directory cached data lives under.
    pub(crate) cache_dir: PathBuf,
    /// Where named settings live.
    pub(crate) settings_store: Arc<dyn SettingsStore>,
    /// The secret backend's label for a session, with `{user_id}` in it.
    ///
    /// Only Linux and macOS have anywhere to show it. The Windows Credential
    /// Manager has no label field at all, and the Android backend writes
    /// files nobody browses, so both carry the value and never read it.
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos")),
        expect(dead_code, reason = "no label to set on this platform's backend")
    )]
    pub(crate) credential_label: String,
    /// The KLIPY API key, empty when the embedder provided none.
    pub(crate) klipy_api_key: String,
    /// The name of the room image packs are created in.
    pub(crate) packs_room_name: String,
    /// The topic of the room image packs are created in.
    pub(crate) packs_room_topic: String,
}

/// The English name of the packs room, used when the embedder provides none
/// of its own.
const DEFAULT_PACKS_ROOM_NAME: &str = "Sticker Packs";

/// The English topic of the packs room, used when the embedder provides none
/// of its own.
const DEFAULT_PACKS_ROOM_TOPIC: &str =
    "The sticker and emoticon packs that you created. Invite someone here to share them.";

/// The English label for a stored session, used when the embedder provides
/// none of its own.
const DEFAULT_CREDENTIAL_LABEL: &str = "Commune: Matrix credentials for {user_id}";

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
        app_name,
        device_display_name,
        oauth_client,
        profile,
        build_number,
        data_dir,
        cache_dir,
        settings_store,
        credential_label,
        klipy_api_key,
        packs_room_name,
        packs_room_topic,
    } = config;

    let settings_store =
        settings_store.unwrap_or_else(|| Arc::new(FileSettingsStore::new(&data_dir)));
    let device_display_name = device_display_name.unwrap_or_else(|| app_name.clone());
    let credential_label = credential_label.unwrap_or_else(|| DEFAULT_CREDENTIAL_LABEL.to_owned());
    let klipy_api_key = klipy_api_key.unwrap_or_default();
    let packs_room_name = packs_room_name.unwrap_or_else(|| DEFAULT_PACKS_ROOM_NAME.to_owned());
    let packs_room_topic = packs_room_topic.unwrap_or_else(|| DEFAULT_PACKS_ROOM_TOPIC.to_owned());

    let _ = CONFIG.set(ResolvedConfig {
        app_id,
        app_name,
        device_display_name,
        oauth_client,
        profile,
        build_number,
        data_dir,
        cache_dir,
        settings_store,
        credential_label,
        klipy_api_key,
        packs_room_name,
        packs_room_topic,
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

/// The name of the application, as it presents itself to homeservers.
#[must_use]
pub fn app_name() -> &'static str {
    &get().app_name
}

/// The name the pusher gives this device.
pub(crate) fn device_display_name() -> &'static str {
    &get().device_display_name
}

/// The embedder's OAuth 2.0 client registration.
pub(crate) fn oauth_client() -> &'static OAuthClientConfig {
    &get().oauth_client
}

/// The build profile name.
#[must_use]
pub fn profile() -> &'static str {
    &get().profile
}

/// The number of commits behind this build.
#[must_use]
pub fn build_number() -> u64 {
    get().build_number
}

/// Where named settings live.
pub(crate) fn settings_store() -> Arc<dyn SettingsStore> {
    get().settings_store.clone()
}

/// The label the secret backend should show for the session of the given user.
///
/// Only `{user_id}` is substituted, and a template that does not contain it
/// is used as it stands — a translation that dropped the placeholder gives a
/// label with no Matrix ID in it rather than a panic.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn credential_label(user_id: &str) -> String {
    get().credential_label.replace("{user_id}", user_id)
}

/// The KLIPY API key, or the empty string if the embedder provided none.
pub(crate) fn klipy_api_key() -> &'static str {
    &get().klipy_api_key
}

/// The name of the room image packs are created in.
pub(crate) fn packs_room_name() -> &'static str {
    &get().packs_room_name
}

/// The topic of the room image packs are created in.
pub(crate) fn packs_room_topic() -> &'static str {
    &get().packs_room_topic
}

/// Initialize the configuration for this crate's tests.
///
/// The config is process-global and set once, so every test that needs it
/// calls this and they all share the same values.
#[cfg(test)]
pub(crate) fn init_test_config() {
    init(CoreConfig {
        app_id: "io.github.steeb_k.Commune.CoreTest".to_owned(),
        app_name: "Commune".to_owned(),
        device_display_name: None,
        oauth_client: OAuthClientConfig {
            client_uri: Url::parse("https://github.com/steeb-k/commune").expect("valid URL"),
            redirect_uris: vec![Url::parse("http://127.0.0.1/").expect("valid URL")],
        },
        profile: "test".to_owned(),
        build_number: 0,
        data_dir: std::env::temp_dir().join("commune-core-test").join("data"),
        cache_dir: std::env::temp_dir().join("commune-core-test").join("cache"),
        settings_store: None,
        credential_label: None,
        klipy_api_key: None,
        packs_room_name: None,
        packs_room_topic: None,
    });
}

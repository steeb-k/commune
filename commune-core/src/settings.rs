//! Application settings, and where they are stored.
//!
//! The application keeps its settings in `GSettings`; Android wants
//! `SharedPreferences` or a file of its own. The core cares about neither —
//! it reads and writes named string values through [`SettingsStore`], which
//! the embedder may provide at [`crate::config::init()`]. When it does not,
//! [`FileSettingsStore`] keeps a JSON file under the data directory, which
//! is a perfectly good answer on Android where the directory is app-private.
//!
//! The per-session settings themselves are the application's
//! `SessionSettings`/`StoredSessionSettings`/`SessionListSettings` with the
//! `GObject` shells removed: the same JSON document, under the same
//! `"sessions"` key, so a GTK profile's settings carry over unchanged.
//!
//! Change notification is deliberately absent for now: these values are
//! read when a screen opens and written when the user flips a switch, and
//! nothing binds to them live yet. Wrap the hot ones in
//! `eyeball::SharedObservable` when the settings UI chunk needs it.

use std::{
    collections::{BTreeSet, HashMap},
    fs, io,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};

use indexmap::{IndexMap, IndexSet};
use ruma::{OwnedServerName, events::media_preview_config::MediaPreviews};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::{config, secret::SESSION_ID_LENGTH, session::SidebarSectionName};

/// The settings key holding the serialized list of session settings.
const SESSIONS_KEY: &str = "sessions";

/// The current version of the stored session settings.
const CURRENT_VERSION: u8 = 1;

/// Where named string settings live.
///
/// The GTK application implements this over `GSettings`; the default is
/// [`FileSettingsStore`]. Implementations must be cheap enough to call from
/// async contexts — every write here is a few hundred bytes.
pub trait SettingsStore: Send + Sync {
    /// The value stored for the given key, if any.
    fn get(&self, key: &str) -> Option<String>;

    /// Store the given value under the given key.
    fn set(&self, key: &str, value: &str);
}

/// A [`SettingsStore`] over a single JSON file in the data directory.
pub struct FileSettingsStore {
    /// The path of the settings file.
    path: PathBuf,
    /// The values, loaded once and kept in sync with the file.
    values: Mutex<Option<HashMap<String, String>>>,
}

impl FileSettingsStore {
    /// Create a store over `core-settings.json` in the given directory.
    pub(crate) fn new(dir: &std::path::Path) -> Self {
        Self {
            path: dir.join("core-settings.json"),
            values: Mutex::new(None),
        }
    }

    /// Run the given closure on the loaded values, loading them first if
    /// this is the first access.
    fn with_values<T>(&self, f: impl FnOnce(&mut HashMap<String, String>) -> T) -> T {
        let mut guard = self.values.lock().expect("mutex is not poisoned");

        let values = guard.get_or_insert_with(|| match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|parse_error| {
                error!("Could not parse the settings file, starting fresh: {parse_error}");
                HashMap::new()
            }),
            Err(read_error) if read_error.kind() == io::ErrorKind::NotFound => HashMap::new(),
            Err(read_error) => {
                error!("Could not read the settings file, starting fresh: {read_error}");
                HashMap::new()
            }
        });

        f(values)
    }

    /// Write the current values back to the file.
    fn save(&self, values: &HashMap<String, String>) {
        let write = || -> io::Result<()> {
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)?;
            }

            let bytes = serde_json::to_vec(values).expect("string map serializes");
            fs::write(&self.path, bytes)
        };

        if let Err(write_error) = write() {
            error!("Could not save the settings file: {write_error}");
        }
    }
}

impl SettingsStore for FileSettingsStore {
    fn get(&self, key: &str) -> Option<String> {
        self.with_values(|values| values.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) {
        self.with_values(|values| {
            values.insert(key.to_owned(), value.to_owned());
            self.save(values);
        });
    }
}

/// The serialized settings of one session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct StoredSessionSettings {
    /// The version of the stored settings.
    #[serde(default)]
    version: u8,

    /// Custom servers to explore.
    #[serde(default, skip_serializing_if = "IndexSet::is_empty")]
    explore_custom_servers: IndexSet<OwnedServerName>,

    /// Whether notifications are enabled for this session.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    notifications_enabled: bool,

    /// Whether public read receipts are enabled for this session.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    public_read_receipts_enabled: bool,

    /// Whether typing notifications are enabled for this session.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    typing_enabled: bool,

    /// The sections that are expanded.
    #[serde(default)]
    sections_expanded: SectionsExpanded,

    /// Which rooms display media previews for this session.
    ///
    /// Legacy setting from version 0 of the stored settings.
    #[serde(skip_serializing)]
    media_previews_enabled: Option<MediaPreviewsSetting>,

    /// Whether to display avatars in invites.
    ///
    /// Legacy setting from version 0 of the stored settings.
    #[serde(skip_serializing)]
    invite_avatars_enabled: Option<bool>,
}

impl StoredSessionSettings {
    /// The stored settings version, for the account-data migration that
    /// consumes the legacy values below (not yet extracted).
    #[must_use]
    pub fn version(&self) -> u8 {
        self.version
    }

    /// The legacy version-0 media-previews setting, for the account-data
    /// migration.
    #[must_use]
    pub fn legacy_media_previews_enabled(&self) -> Option<MediaPreviews> {
        self.media_previews_enabled
            .clone()
            .map(|setting| setting.global.into())
    }

    /// The legacy version-0 invite-avatars setting, for the account-data
    /// migration.
    #[must_use]
    pub fn legacy_invite_avatars_enabled(&self) -> Option<bool> {
        self.invite_avatars_enabled
    }
}

impl Default for StoredSessionSettings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            explore_custom_servers: Default::default(),
            notifications_enabled: true,
            public_read_receipts_enabled: true,
            typing_enabled: true,
            sections_expanded: Default::default(),
            media_previews_enabled: Default::default(),
            invite_avatars_enabled: Default::default(),
        }
    }
}

/// The sections that are expanded.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SectionsExpanded(BTreeSet<SidebarSectionName>);

impl SectionsExpanded {
    /// Whether the section with the given name is expanded.
    #[must_use]
    pub fn is_section_expanded(&self, section_name: SidebarSectionName) -> bool {
        self.0.contains(&section_name)
    }

    /// Set whether the section with the given name is expanded.
    pub fn set_section_expanded(&mut self, section_name: SidebarSectionName, expanded: bool) {
        if expanded {
            self.0.insert(section_name);
        } else {
            self.0.remove(&section_name);
        }
    }
}

impl Default for SectionsExpanded {
    fn default() -> Self {
        Self(BTreeSet::from([
            SidebarSectionName::VerificationRequest,
            SidebarSectionName::InviteRequest,
            SidebarSectionName::Invited,
            SidebarSectionName::ServerNotice,
            SidebarSectionName::Space,
            SidebarSectionName::Favorite,
            SidebarSectionName::Normal,
            SidebarSectionName::LowPriority,
        ]))
    }
}

/// Setting about which rooms display media previews.
///
/// Legacy setting from version 0 of the stored settings.
#[derive(Debug, Clone, Default, Deserialize)]
struct MediaPreviewsSetting {
    /// The default setting for all rooms.
    #[serde(default)]
    global: MediaPreviewsGlobalSetting,
}

/// Possible values of the global setting about which rooms display media
/// previews.
///
/// Legacy setting from version 0 of the stored settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MediaPreviewsGlobalSetting {
    /// All rooms show media previews.
    All,
    /// Only private rooms show media previews.
    #[default]
    Private,
    /// No rooms show media previews.
    None,
}

impl From<MediaPreviewsGlobalSetting> for MediaPreviews {
    fn from(value: MediaPreviewsGlobalSetting) -> Self {
        match value {
            MediaPreviewsGlobalSetting::All => Self::On,
            MediaPreviewsGlobalSetting::Private => Self::Private,
            MediaPreviewsGlobalSetting::None => Self::Off,
        }
    }
}

/// The settings of a [`Session`](crate::session::Session).
///
/// Cheap to clone; every clone shares the same state, and every setter
/// persists through the owning [`SessionListSettings`].
#[derive(Debug, Clone)]
pub struct SessionSettings(Arc<SessionSettingsInner>);

#[derive(Debug)]
struct SessionSettingsInner {
    /// The ID of the session these settings are for.
    session_id: String,
    /// The stored settings.
    stored: Mutex<StoredSessionSettings>,
    /// The list these settings persist through.
    list: Weak<SessionListSettingsInner>,
}

impl SessionSettings {
    /// Create new default settings for the given session ID.
    fn new(session_id: &str, list: Weak<SessionListSettingsInner>) -> Self {
        Self::restore(session_id, StoredSessionSettings::default(), list)
    }

    /// Restore existing settings with the given session ID and stored
    /// settings.
    fn restore(
        session_id: &str,
        stored: StoredSessionSettings,
        list: Weak<SessionListSettingsInner>,
    ) -> Self {
        Self(Arc::new(SessionSettingsInner {
            session_id: session_id.to_owned(),
            stored: Mutex::new(stored),
            list,
        }))
    }

    /// The ID of the session these settings are for.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.0.session_id
    }

    /// The stored settings.
    #[must_use]
    pub fn stored_settings(&self) -> StoredSessionSettings {
        self.0.stored.lock().expect("mutex is not poisoned").clone()
    }

    /// Save through the owning list, if it is still alive.
    fn save(&self) {
        if let Some(list) = self.0.list.upgrade() {
            SessionListSettings(list).save();
        }
    }

    /// Read a value out of the stored settings.
    fn read<T>(&self, f: impl FnOnce(&StoredSessionSettings) -> T) -> T {
        f(&self.0.stored.lock().expect("mutex is not poisoned"))
    }

    /// Mutate the stored settings and persist the result.
    fn write(&self, f: impl FnOnce(&mut StoredSessionSettings)) {
        f(&mut self.0.stored.lock().expect("mutex is not poisoned"));
        self.save();
    }

    /// Whether notifications are enabled for this session.
    #[must_use]
    pub fn notifications_enabled(&self) -> bool {
        self.read(|s| s.notifications_enabled)
    }

    /// Set whether notifications are enabled for this session.
    pub fn set_notifications_enabled(&self, enabled: bool) {
        if self.notifications_enabled() == enabled {
            return;
        }
        self.write(|s| s.notifications_enabled = enabled);
    }

    /// Whether public read receipts are enabled for this session.
    #[must_use]
    pub fn public_read_receipts_enabled(&self) -> bool {
        self.read(|s| s.public_read_receipts_enabled)
    }

    /// Set whether public read receipts are enabled for this session.
    pub fn set_public_read_receipts_enabled(&self, enabled: bool) {
        if self.public_read_receipts_enabled() == enabled {
            return;
        }
        self.write(|s| s.public_read_receipts_enabled = enabled);
    }

    /// Whether typing notifications are enabled for this session.
    #[must_use]
    pub fn typing_enabled(&self) -> bool {
        self.read(|s| s.typing_enabled)
    }

    /// Set whether typing notifications are enabled for this session.
    pub fn set_typing_enabled(&self, enabled: bool) {
        if self.typing_enabled() == enabled {
            return;
        }
        self.write(|s| s.typing_enabled = enabled);
    }

    /// Custom servers to explore.
    #[must_use]
    pub fn explore_custom_servers(&self) -> IndexSet<OwnedServerName> {
        self.read(|s| s.explore_custom_servers.clone())
    }

    /// Set the custom servers to explore.
    pub fn set_explore_custom_servers(&self, servers: IndexSet<OwnedServerName>) {
        if self.explore_custom_servers() == servers {
            return;
        }
        self.write(|s| s.explore_custom_servers = servers);
    }

    /// Whether the section with the given name is expanded.
    #[must_use]
    pub fn is_section_expanded(&self, section_name: SidebarSectionName) -> bool {
        self.read(|s| s.sections_expanded.is_section_expanded(section_name))
    }

    /// Set whether the section with the given name is expanded.
    pub fn set_section_expanded(&self, section_name: SidebarSectionName, expanded: bool) {
        self.write(|s| {
            s.sections_expanded
                .set_section_expanded(section_name, expanded);
        });
    }

    /// Apply the migration of the stored settings from version 0 to
    /// version 1.
    ///
    /// Driven by the global-account-data migration, which is not extracted
    /// yet.
    pub fn apply_version_1_migration(&self) {
        let migrated = {
            let mut stored = self.0.stored.lock().expect("mutex is not poisoned");

            if stored.version > 0 {
                false
            } else {
                info!(
                    session = self.0.session_id,
                    "Migrating stored session to version 1"
                );

                stored.media_previews_enabled.take();
                stored.invite_avatars_enabled.take();
                stored.version = 1;
                true
            }
        };

        if migrated {
            self.save();
        }
    }

    /// Delete the settings from the application settings.
    pub fn delete(&self) {
        if let Some(list) = self.0.list.upgrade() {
            SessionListSettings(list).remove(&self.0.session_id);
        }
    }
}

/// The settings of the list of sessions.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone, Default)]
pub struct SessionListSettings(Arc<SessionListSettingsInner>);

#[derive(Debug, Default)]
struct SessionListSettingsInner {
    /// The settings of the sessions.
    sessions: Mutex<IndexMap<String, SessionSettings>>,
}

impl SessionListSettings {
    /// Create a new empty `SessionListSettings`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load these settings from the settings store.
    pub fn load(&self) {
        let serialized = config::settings_store()
            .get(SESSIONS_KEY)
            .unwrap_or_default();

        let stored_sessions =
            match serde_json::from_str::<Vec<(String, StoredSessionSettings)>>(&serialized) {
                Ok(stored_sessions) => stored_sessions,
                Err(parse_error) => {
                    error!(
                        "Could not load sessions settings, fallback to default settings: \
                         {parse_error}"
                    );
                    Default::default()
                }
            };

        // Do we need to update the settings?
        let mut needs_update = false;

        let sessions = stored_sessions
            .into_iter()
            .map(|(mut session_id, stored_session)| {
                // Session IDs have been truncated in version 6 of StoredSession.
                if session_id.len() > SESSION_ID_LENGTH {
                    session_id.truncate(SESSION_ID_LENGTH);
                    needs_update = true;
                }

                let session =
                    SessionSettings::restore(&session_id, stored_session, Arc::downgrade(&self.0));
                (session_id, session)
            })
            .collect();

        *self.0.sessions.lock().expect("mutex is not poisoned") = sessions;

        if needs_update {
            self.save();
        }
    }

    /// Save these settings in the settings store.
    pub fn save(&self) {
        let stored_sessions = self
            .0
            .sessions
            .lock()
            .expect("mutex is not poisoned")
            .iter()
            .map(|(session_id, session)| (session_id.clone(), session.stored_settings()))
            .collect::<Vec<_>>();

        config::settings_store().set(
            SESSIONS_KEY,
            &serde_json::to_string(&stored_sessions).expect("settings serialize"),
        );
    }

    /// Get or create the settings for the session with the given ID.
    #[must_use]
    pub fn get_or_create(&self, session_id: &str) -> SessionSettings {
        if let Some(session) = self
            .0
            .sessions
            .lock()
            .expect("mutex is not poisoned")
            .get(session_id)
        {
            return session.clone();
        }

        let session = SessionSettings::new(session_id, Arc::downgrade(&self.0));
        self.0
            .sessions
            .lock()
            .expect("mutex is not poisoned")
            .insert(session_id.to_owned(), session.clone());
        self.save();

        session
    }

    /// Remove the settings of the session with the given ID.
    pub fn remove(&self, session_id: &str) {
        self.0
            .sessions
            .lock()
            .expect("mutex is not poisoned")
            .shift_remove(session_id);
        self.save();
    }

    /// Get the list of session IDs stored in these settings.
    #[must_use]
    pub fn session_ids(&self) -> IndexSet<String> {
        self.0
            .sessions
            .lock()
            .expect("mutex is not poisoned")
            .keys()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value set on a `FileSettingsStore` comes back, from the same store
    /// and from a fresh one over the same file.
    #[test]
    fn file_store_round_trip() {
        let dir = std::env::temp_dir()
            .join("commune-core-test")
            .join(format!("file-store-{}", std::process::id()));

        let store = FileSettingsStore::new(&dir);
        assert_eq!(store.get("missing"), None);
        store.set("greeting", "hello");
        assert_eq!(store.get("greeting"), Some("hello".to_owned()));

        let reopened = FileSettingsStore::new(&dir);
        assert_eq!(reopened.get("greeting"), Some("hello".to_owned()));

        let _ = fs::remove_dir_all(dir);
    }

    /// Session settings persist through the settings store and survive a
    /// reload, keeping their order.
    #[test]
    fn session_settings_survive_a_reload() {
        crate::config::init_test_config();

        // Unique per run: the store under the test config's data dir
        // persists between runs.
        let id_a = format!("a{:0>7}", std::process::id() % 10_000_000);
        let id_b = format!("b{:0>7}", std::process::id() % 10_000_000);

        let settings = SessionListSettings::new();
        settings.load();
        settings
            .get_or_create(&id_a)
            .set_notifications_enabled(false);
        settings.get_or_create(&id_b).set_typing_enabled(false);

        let reloaded = SessionListSettings::new();
        reloaded.load();

        let session_a = reloaded.get_or_create(&id_a);
        assert!(!session_a.notifications_enabled());
        assert!(session_a.typing_enabled());

        let session_b = reloaded.get_or_create(&id_b);
        assert!(session_b.notifications_enabled());
        assert!(!session_b.typing_enabled());

        let ids = reloaded.session_ids();
        assert!(ids.get_index_of(&id_a) < ids.get_index_of(&id_b));

        // Leave the store tidy for the next run.
        reloaded.remove(&id_a);
        reloaded.remove(&id_b);
    }
}

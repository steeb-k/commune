//! The list of logged-in sessions, headless.
//!
//! The application models this as a `gio::ListModel` of `SessionInfo`
//! `GObjects` with a class hierarchy — `NewSession`, `FailedSession` and
//! `Session` all extending `SessionInfo` so one list can hold sessions in
//! every stage of restoration. Headless, the hierarchy collapses into
//! [`SessionEntry`], and the list model becomes an
//! [`eyeball_im::ObservableVector`], whose subscribers receive the same
//! `VectorDiff`s the SDK's own lists emit.

use std::{cmp::Ordering, ffi::OsString, sync::Arc};

use eyeball::{SharedObservable, Subscriber};
use eyeball_im::{ObservableVector, Vector, VectorDiff};
use futures_util::Stream;
use tracing::{error, info};

use crate::{
    RUNTIME, UserFacingError,
    matrix::ClientSetupError,
    paths::DataType,
    secret::{Secret, SecretExt, StoredSession},
    session::{Session, SessionState},
    settings::SessionListSettings,
    spawn_tokio,
    utils::LoadingState,
};

/// A session in the list, at whatever stage of restoration it has reached.
#[derive(Debug, Clone)]
pub enum SessionEntry {
    /// A stored session that is being restored.
    Loading(StoredSession),
    /// A stored session that could not be restored.
    Failed {
        /// The stored session.
        info: StoredSession,
        /// Why it could not be restored.
        error: Arc<ClientSetupError>,
    },
    /// A restored, running session.
    Ready(Session),
}

impl SessionEntry {
    /// The stored session behind this entry.
    #[must_use]
    pub fn info(&self) -> &StoredSession {
        match self {
            Self::Loading(info) | Self::Failed { info, .. } => info,
            Self::Ready(session) => session.info(),
        }
    }

    /// The local ID of this session.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.info().id
    }

    /// The running session, if this entry is ready.
    #[must_use]
    pub fn session(&self) -> Option<&Session> {
        match self {
            Self::Ready(session) => Some(session),
            _ => None,
        }
    }
}

/// List of all logged in sessions.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct SessionList {
    inner: Arc<SessionListInner>,
}

#[derive(Debug)]
struct SessionListInner {
    /// The sessions, in settings order.
    entries: std::sync::Mutex<ObservableVector<SessionEntry>>,
    /// The loading state of the list.
    state: SharedObservable<LoadingState>,
    /// The error message, if state is set to `LoadingState::Error`.
    error: std::sync::Mutex<Option<String>>,
    /// The settings of the sessions.
    settings: SessionListSettings,
}

impl Default for SessionList {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionList {
    /// Create a new empty `SessionList`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SessionListInner {
                entries: std::sync::Mutex::new(ObservableVector::new()),
                state: SharedObservable::new(LoadingState::default()),
                error: std::sync::Mutex::new(None),
                settings: SessionListSettings::new(),
            }),
        }
    }

    /// The settings of the sessions.
    #[must_use]
    pub fn settings(&self) -> &SessionListSettings {
        &self.inner.settings
    }

    /// The loading state of the list.
    #[must_use]
    pub fn state(&self) -> LoadingState {
        self.inner.state.get()
    }

    /// Subscribe to the loading state of the list.
    pub fn subscribe_state(&self) -> Subscriber<LoadingState> {
        self.inner.state.subscribe()
    }

    /// The error message, if the state is [`LoadingState::Error`].
    #[must_use]
    pub fn error(&self) -> Option<String> {
        self.inner
            .error
            .lock()
            .expect("mutex is not poisoned")
            .clone()
    }

    /// The current entries, and a stream of the changes that follow them.
    pub fn subscribe_entries(
        &self,
    ) -> (
        Vector<SessionEntry>,
        impl Stream<Item = VectorDiff<SessionEntry>> + use<>,
    ) {
        let entries = self.inner.entries.lock().expect("mutex is not poisoned");
        let subscriber = entries.subscribe();
        (entries.clone(), subscriber.into_stream())
    }

    /// Whether this list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner
            .entries
            .lock()
            .expect("mutex is not poisoned")
            .is_empty()
    }

    /// The session with the given ID, if any.
    #[must_use]
    pub fn get(&self, session_id: &str) -> Option<SessionEntry> {
        self.inner
            .entries
            .lock()
            .expect("mutex is not poisoned")
            .iter()
            .find(|entry| entry.session_id() == session_id)
            .cloned()
    }

    /// The index of the session with the given ID, if any.
    #[must_use]
    pub fn index(&self, session_id: &str) -> Option<usize> {
        self.inner
            .entries
            .lock()
            .expect("mutex is not poisoned")
            .iter()
            .position(|entry| entry.session_id() == session_id)
    }

    /// The first session in the list, if any.
    #[must_use]
    pub fn first(&self) -> Option<SessionEntry> {
        self.inner
            .entries
            .lock()
            .expect("mutex is not poisoned")
            .front()
            .cloned()
    }

    /// Insert the given session into the list.
    ///
    /// If a session with the same ID already exists, it is replaced.
    ///
    /// Returns the index of the session, which most callers have no use
    /// for.
    #[allow(clippy::must_use_candidate)]
    pub fn insert(&self, entry: SessionEntry) -> usize {
        if let SessionEntry::Ready(session) = &entry {
            self.watch_logged_out(session);
        }

        let mut entries = self.inner.entries.lock().expect("mutex is not poisoned");
        let session_id = entry.session_id().to_owned();

        if let Some(index) = entries.iter().position(|e| e.session_id() == session_id) {
            entries.set(index, entry);
            index
        } else {
            entries.push_back(entry);
            entries.len() - 1
        }
    }

    /// Remove the session with the given ID from the list.
    pub fn remove(&self, session_id: &str) {
        let mut entries = self.inner.entries.lock().expect("mutex is not poisoned");

        if let Some(index) = entries.iter().position(|e| e.session_id() == session_id) {
            entries.remove(index);
        }
    }

    /// Remove the session with the given ID once it reports being logged
    /// out.
    fn watch_logged_out(&self, session: &Session) {
        let mut subscriber = session.subscribe_state();
        let weak = Arc::downgrade(&self.inner);
        let session_id = session.session_id().to_owned();

        RUNTIME.spawn(async move {
            while let Some(state) = subscriber.next().await {
                if state == SessionState::LoggedOut {
                    if let Some(inner) = weak.upgrade() {
                        SessionList { inner }.remove(&session_id);
                    }
                    break;
                }
            }
        });
    }

    /// Set the error message and put the list in the error state.
    fn set_error(&self, message: String) {
        *self.inner.error.lock().expect("mutex is not poisoned") = Some(message);
        self.inner.state.set(LoadingState::Error);
    }

    /// Restore the logged-in sessions.
    pub async fn restore_sessions(&self) {
        if self.inner.state.get() >= LoadingState::Loading {
            return;
        }

        self.inner.state.set(LoadingState::Loading);

        let mut sessions = match Secret::restore_sessions().await {
            Ok(sessions) => sessions,
            Err(restore_error) => {
                self.set_error(format!(
                    "Could not restore previous sessions\n\n{}",
                    restore_error.to_user_facing(),
                ));
                return;
            }
        };

        let settings = &self.inner.settings;
        settings.load();
        let session_ids = settings.session_ids();

        // Keep the order from the settings.
        sessions.sort_by(|a, b| {
            let pos_a = session_ids.get_index_of(&a.id);
            let pos_b = session_ids.get_index_of(&b.id);

            match (pos_a, pos_b) {
                (Some(pos_a), Some(pos_b)) => pos_a.cmp(&pos_b),
                // Keep unknown sessions at the end.
                (Some(_), None) => Ordering::Greater,
                (None, Some(_)) => Ordering::Less,
                _ => Ordering::Equal,
            }
        });

        // Get the directories present in the data path to only restore sessions with
        // data on the system. This is necessary for users sharing their secrets between
        // devices.
        let mut directories = match data_directories(sessions.len()).await {
            Ok(directories) => directories,
            Err(dir_error) => {
                error!("Could not access data directory: {dir_error}");
                self.set_error(
                    "Could not restore previous sessions\n\nAn unexpected error happened while \
                     accessing the data directory"
                        .to_owned(),
                );
                return;
            }
        };

        for stored_session in sessions {
            if let Some(pos) = directories
                .iter()
                .position(|dir_name| dir_name == stored_session.id.as_str())
            {
                directories.swap_remove(pos);
                info!(
                    "Restoring previous session {} for user {}",
                    stored_session.id, stored_session.user_id,
                );
                self.insert(SessionEntry::Loading(stored_session.clone()));

                let list = self.clone();
                RUNTIME.spawn(async move {
                    list.restore_stored_session(stored_session).await;
                });
            } else {
                info!(
                    "Ignoring session {} for user {}: no data directory",
                    stored_session.id, stored_session.user_id,
                );
            }
        }

        self.inner.state.set(LoadingState::Ready);
    }

    /// Restore a stored session.
    async fn restore_stored_session(&self, session_info: StoredSession) {
        let settings = self.inner.settings.get_or_create(&session_info.id);

        match Session::new(session_info.clone(), settings).await {
            Ok(session) => {
                session.prepare().await;
                self.insert(SessionEntry::Ready(session));
            }
            Err(restore_error) => {
                error!("Could not restore previous session: {restore_error}");
                self.insert(SessionEntry::Failed {
                    info: session_info,
                    error: Arc::new(restore_error),
                });
            }
        }
    }
}

/// The list of directories in the data directory.
async fn data_directories(capacity: usize) -> std::io::Result<Vec<OsString>> {
    let data_path = DataType::Persistent.dir_path();

    if !data_path.try_exists()? {
        return Ok(Vec::new());
    }

    spawn_tokio!(async move {
        let mut read_dir = tokio::fs::read_dir(data_path).await?;
        let mut directories = Vec::with_capacity(capacity);

        loop {
            let Some(entry) = read_dir.next_entry().await? else {
                // We are at the end of the list.
                break;
            };

            if !entry.file_type().await?.is_dir() {
                // We are only interested in directories.
                continue;
            }

            directories.push(entry.file_name());
        }

        std::io::Result::Ok(directories)
    })
    .await
    .expect("task was not aborted")
}

#[cfg(test)]
mod tests {
    use futures_util::{FutureExt, StreamExt};

    use super::*;

    /// A stored session for the tests, with nothing behind it.
    fn stored_session(id: &str) -> StoredSession {
        StoredSession {
            homeserver: "https://example.org".parse().expect("URL is valid"),
            user_id: "@alice:example.org".try_into().expect("user ID is valid"),
            device_id: "ADEVICEID".into(),
            id: id.to_owned(),
            client_id: None,
            passphrase: zeroize::Zeroizing::new("a-passphrase".to_owned()),
        }
    }

    /// Inserting replaces an entry with the same ID, subscribers see the
    /// diffs, and removal empties the list.
    #[test]
    fn entries_are_keyed_by_session_id() {
        let list = SessionList::new();
        let (initial, mut stream) = list.subscribe_entries();
        assert!(initial.is_empty());

        assert_eq!(
            list.insert(SessionEntry::Loading(stored_session("aaaaaaaa"))),
            0
        );
        assert_eq!(
            list.insert(SessionEntry::Loading(stored_session("bbbbbbbb"))),
            1
        );
        // Same ID: replaced in place, not appended.
        assert_eq!(
            list.insert(SessionEntry::Failed {
                info: stored_session("aaaaaaaa"),
                error: Arc::new(ClientSetupError::NoSessionTokens),
            }),
            0
        );

        assert_eq!(list.index("bbbbbbbb"), Some(1));
        assert!(matches!(
            list.get("aaaaaaaa"),
            Some(SessionEntry::Failed { .. })
        ));
        assert!(matches!(list.first(), Some(SessionEntry::Failed { .. })));

        list.remove("aaaaaaaa");
        list.remove("bbbbbbbb");
        assert!(list.is_empty());

        // The diffs arrive in order: two pushes, one set, two removals.
        let mut diffs = Vec::new();
        while let Some(Some(diff)) = stream.next().now_or_never() {
            diffs.push(diff);
        }
        assert!(matches!(diffs[0], VectorDiff::PushBack { .. }));
        assert!(matches!(diffs[1], VectorDiff::PushBack { .. }));
        assert!(matches!(diffs[2], VectorDiff::Set { index: 0, .. }));
        assert!(matches!(diffs[3], VectorDiff::Remove { index: 0 }));
        // "bbbbbbbb" shifted to the front when "aaaaaaaa" left.
        assert!(matches!(diffs[4], VectorDiff::Remove { index: 0 }));
        assert_eq!(diffs.len(), 5);
    }
}

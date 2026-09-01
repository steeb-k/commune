//! A Matrix user session, headless.
//!
//! This is the application's `session/mod.rs` with the `GObject` shell and the
//! GTK main loop removed. Three things changed shape and everything else is
//! the same logic in the same order:
//!
//! * **Change propagation.** `GObject` properties with `notify` became
//!   [`eyeball::SharedObservable`]s — the same primitive the SDK itself uses —
//!   so state, offline-ness, reachability and the user profile are subscribable
//!   streams.
//! * **No UI-thread hop.** The application bounced every sync response and
//!   session change through `glib::MainContext`. Here the handlers run directly
//!   on the tokio task that received the value; whoever subscribes decides
//!   where to consume the streams.
//! * **Reachability.** `gio::NetworkMonitor` became a plain TCP dial to the
//!   homeserver plus [`Session::network_changed()`], which the embedder calls
//!   whenever the platform reports a connectivity change (GTK from
//!   `NetworkMonitor`, Android from `ConnectivityManager`).
//!
//! The subsystems the application hangs off its session — verification,
//! notifications, presence, image packs, account data, calls — arrive with
//! their own extraction chunks. The room list is here: `prepare()` loads it
//! and feeds it the sync loop's room updates.
//!
//! Every async method here must run inside the core's tokio runtime (the
//! sync loop sleeps and dials sockets). `SessionList` already does; the FFI
//! facade will wrap calls in `RUNTIME.spawn`.

mod ignored_users;
mod room;
mod room_list;
mod sidebar;
mod user_sessions;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use eyeball::{SharedObservable, Subscriber};
use futures_util::{StreamExt, future::BoxFuture};
use matrix_sdk::{
    Client, SessionChange,
    config::SyncSettings,
    media::MediaRetentionPolicy,
    sync::{RoomUpdates, SyncResponse},
};
use ruma::{
    OwnedDeviceId, OwnedMxcUri, OwnedUserId,
    api::client::{
        filter::{FilterDefinition, RoomFilter},
        profile::{AvatarUrl, DisplayName},
        search::search_events::v3::UserProfile,
    },
    assign,
};
use tokio::{net::TcpStream, sync::mpsc, task::AbortHandle, time::sleep};
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, error, info};
use url::Url;

pub use self::{
    ignored_users::{IgnoredUsers, IgnoredUsersError},
    room::{
        Member, MemberList, MemberRole, Membership, ReceiptPosition, Room, RoomCategory,
        RoomDisplayName, RoomHighlight, TargetRoomCategory, Timeline, TimelineFocusKind,
    },
    room_list::{RoomList, RoomMetainfo},
    sidebar::SidebarSectionName,
    user_sessions::{Device, DeviceError, UserSessions},
};
use crate::{
    RUNTIME,
    matrix::{self, ClientSetupError},
    secret::StoredSession,
    settings::{SessionListSettings, SessionSettings},
    spawn_tokio,
    utils::TokioDrop,
};

/// The database key for persisting the session's profile.
const SESSION_PROFILE_KEY: &str = "session_profile";
/// The number of consecutive missed synchronizations before the session is
/// marked as offline.
///
/// Note that this is set to `2`, but the count begins at `0` so this would
/// match the third missed synchronization.
const MISSED_SYNC_OFFLINE_COUNT: usize = 2;
/// The delays in seconds to wait for when a sync fails, depending on the number
/// of missed attempts.
const MISSED_SYNC_DELAYS: &[u64] = &[1, 5, 10, 20, 30];
/// How long a reachability probe waits before calling the homeserver
/// unreachable.
const REACHABILITY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait before probing reachability again after a failure.
const REACHABILITY_RETRY_DELAY: Duration = Duration::from_secs(10);

/// The state of the session.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SessionState {
    LoggedOut,
    #[default]
    Init,
    InitialSync,
    Ready,
}

/// The profile of the session's own user.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionProfile {
    /// The display name of the user, if any.
    pub display_name: Option<String>,
    /// The avatar of the user, if any.
    pub avatar_url: Option<OwnedMxcUri>,
}

/// What can go wrong while changing the account's own profile.
#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    /// The file could not be read.
    #[error("the image could not be read")]
    UnreadableImage,
    /// The homeserver refused.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` here expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl crate::UserFacingError for AccountError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::UnreadableImage => "Could not read the image.".to_owned(),
            // The embedder renders an SDK error itself — the GTK
            // application's rendering is translated — so this is the
            // fallback the Kotlin side gets.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// A Matrix user session.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct Session {
    inner: Arc<SessionInner>,
}

/// A weak reference to a [`Session`].
///
/// What everything owned by the session holds, so that dropping the session
/// actually drops it.
#[derive(Debug, Clone)]
pub struct WeakSession(std::sync::Weak<SessionInner>);

impl WeakSession {
    /// The session, if it is still alive.
    #[must_use]
    pub fn upgrade(&self) -> Option<Session> {
        self.0.upgrade().map(|inner| Session { inner })
    }
}

#[derive(Debug)]
struct SessionInner {
    /// The stored session this was restored from.
    info: StoredSession,
    /// The current settings for this session.
    settings: SessionSettings,
    /// The Matrix client for this session.
    client: TokioDrop<Client>,
    /// The current state of the session.
    state: SharedObservable<SessionState>,
    /// Whether this session has a connection to the homeserver.
    is_homeserver_reachable: SharedObservable<bool>,
    /// Whether this session is synchronized with the homeserver.
    is_offline: SharedObservable<bool>,
    /// The profile of the session's own user.
    profile: SharedObservable<SessionProfile>,
    /// The number of missed synchronizations in a row.
    ///
    /// Capped at `MISSED_SYNC_DELAYS.len() - 1`.
    missed_sync_count: AtomicUsize,
    /// The running sync loop.
    sync_handle: Mutex<Option<AbortHandle>>,
    /// The task watching session changes.
    session_changes_handle: Mutex<Option<AbortHandle>>,
    /// The pending retry of the reachability probe.
    reachability_retry_handle: Mutex<Option<AbortHandle>>,
    /// Guard so only one reachability check runs at a time.
    reachability_lock: tokio::sync::Mutex<()>,
    /// Where the sync loop sends room updates for the room list to consume.
    room_updates_tx: mpsc::UnboundedSender<RoomUpdates>,
    /// The receiving end, until the room list takes it.
    room_updates_rx: Mutex<Option<mpsc::UnboundedReceiver<RoomUpdates>>>,
    /// The room list of this session.
    room_list: std::sync::OnceLock<RoomList>,
    /// The users this account ignores.
    ignored_users: std::sync::OnceLock<IgnoredUsers>,
    /// The account's other sessions.
    user_sessions: std::sync::OnceLock<UserSessions>,
    /// The task feeding the room list from the sync loop.
    room_updates_handle: Mutex<Option<AbortHandle>>,
}

impl Drop for SessionInner {
    fn drop(&mut self) {
        for slot in [
            &mut self.sync_handle,
            &mut self.session_changes_handle,
            &mut self.reachability_retry_handle,
            &mut self.room_updates_handle,
        ] {
            if let Ok(Some(handle)) = slot.get_mut().map(Option::take) {
                handle.abort();
            }
        }
    }
}

impl Session {
    /// Construct an existing session.
    pub async fn new(
        stored_session: StoredSession,
        settings: SessionSettings,
    ) -> Result<Self, ClientSetupError> {
        let tokens = stored_session
            .load_tokens()
            .await
            .ok_or(ClientSetupError::NoSessionTokens)?;

        let stored_session_clone = stored_session.clone();
        let client = spawn_tokio!(async move {
            let client = matrix::client_with_stored_session(stored_session_clone, tokens).await?;

            // Make sure that we use the proper retention policy.
            let media = client.media();
            let used_media_retention_policy = media.media_retention_policy().await?;
            let wanted_media_retention_policy = MediaRetentionPolicy::default();

            if used_media_retention_policy != wanted_media_retention_policy {
                media
                    .set_media_retention_policy(wanted_media_retention_policy)
                    .await?;
            }

            Ok::<_, ClientSetupError>(client)
        })
        .await
        .expect("task was not aborted")?;

        let (room_updates_tx, room_updates_rx) = mpsc::unbounded_channel();

        let inner = Arc::new(SessionInner {
            info: stored_session,
            settings,
            client: TokioDrop::new(client),
            state: SharedObservable::new(SessionState::default()),
            is_homeserver_reachable: SharedObservable::new(false),
            is_offline: SharedObservable::new(false),
            profile: SharedObservable::new(SessionProfile::default()),
            missed_sync_count: AtomicUsize::new(0),
            sync_handle: Mutex::new(None),
            session_changes_handle: Mutex::new(None),
            reachability_retry_handle: Mutex::new(None),
            reachability_lock: tokio::sync::Mutex::new(()),
            room_updates_tx,
            room_updates_rx: Mutex::new(Some(room_updates_rx)),
            room_list: std::sync::OnceLock::new(),
            ignored_users: std::sync::OnceLock::new(),
            user_sessions: std::sync::OnceLock::new(),
            room_updates_handle: Mutex::new(None),
        });

        Ok(Self { inner })
    }

    /// Create a new session from the session of the given Matrix client.
    pub async fn create(
        client: &Client,
        list_settings: &SessionListSettings,
    ) -> Result<Self, ClientSetupError> {
        let stored_session = StoredSession::new(client).await?;
        let settings = list_settings.get_or_create(&stored_session.id);

        Self::new(stored_session, settings).await
    }

    /// Finish initialization of this session.
    pub async fn prepare(&self) {
        let inner = &self.inner;

        {
            let weak = Arc::downgrade(inner);
            RUNTIME.spawn(async move {
                let Some(inner) = weak.upgrade() else { return };
                // First, load the profile from the cache, it will be quicker.
                inner.init_user_profile().await;
                // Then, check if the profile changed.
                inner.update_user_profile().await;
            });
        }

        SessionInner::watch_session_changes(inner);
        SessionInner::update_homeserver_reachable(inner).await;

        self.room_list().load().await;
        self.consume_room_updates();
        self.ignored_users().load().await;

        // The verification, calls and security subsystems attach here once
        // their chunks are extracted.

        let client = self.client();
        spawn_tokio!(async move {
            client
                .send_queue()
                .respawn_tasks_for_rooms_with_unsent_requests()
                .await;
        });

        inner.set_state(SessionState::InitialSync);
        SessionInner::sync(inner);

        debug!(session = self.session_id(), "A new session was prepared");
    }

    /// The stored session this was restored from.
    #[must_use]
    pub fn info(&self) -> &StoredSession {
        &self.inner.info
    }

    /// The local session's ID.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.inner.info.id
    }

    /// The Matrix session's user ID.
    #[must_use]
    pub fn user_id(&self) -> &OwnedUserId {
        &self.inner.info.user_id
    }

    /// The Matrix session's homeserver.
    #[must_use]
    pub fn homeserver(&self) -> &Url {
        &self.inner.info.homeserver
    }

    /// The Matrix session's device ID.
    #[must_use]
    pub fn device_id(&self) -> &OwnedDeviceId {
        &self.inner.info.device_id
    }

    /// Whether this session uses the OAuth 2.0 API.
    #[must_use]
    pub fn uses_oauth_api(&self) -> bool {
        self.inner.info.client_id.is_some()
    }

    /// The current settings for this session.
    #[must_use]
    pub fn settings(&self) -> &SessionSettings {
        &self.inner.settings
    }

    /// The Matrix client.
    #[must_use]
    pub fn client(&self) -> Client {
        (*self.inner.client).clone()
    }

    /// The current state of the session.
    #[must_use]
    pub fn state(&self) -> SessionState {
        self.inner.state.get()
    }

    /// Subscribe to the state of the session.
    pub fn subscribe_state(&self) -> Subscriber<SessionState> {
        self.inner.state.subscribe()
    }

    /// Whether this session is synchronized with the homeserver.
    #[must_use]
    pub fn is_offline(&self) -> bool {
        self.inner.is_offline.get()
    }

    /// Subscribe to whether this session is synchronized with the
    /// homeserver.
    pub fn subscribe_is_offline(&self) -> Subscriber<bool> {
        self.inner.is_offline.subscribe()
    }

    /// Whether this session has a connection to the homeserver.
    #[must_use]
    pub fn is_homeserver_reachable(&self) -> bool {
        self.inner.is_homeserver_reachable.get()
    }

    /// Subscribe to whether this session has a connection to the
    /// homeserver.
    pub fn subscribe_is_homeserver_reachable(&self) -> Subscriber<bool> {
        self.inner.is_homeserver_reachable.subscribe()
    }

    /// The profile of the session's own user.
    #[must_use]
    pub fn profile(&self) -> SessionProfile {
        self.inner.profile.get()
    }

    /// Subscribe to the profile of the session's own user.
    pub fn subscribe_profile(&self) -> Subscriber<SessionProfile> {
        self.inner.profile.subscribe()
    }

    /// Re-read the account's profile from the homeserver.
    ///
    /// Refreshes the observable and the on-disk cache, which is what
    /// `prepare()` does once at startup and nothing did again.
    pub async fn refresh_profile(&self) {
        self.inner.update_user_profile().await;
    }

    /// Change the account's display name.
    ///
    /// The observable is updated here as well as by the sync that follows,
    /// for the reason the application's general page gives: an account in
    /// no rooms is never told about its own profile change, so this is the
    /// only copy that would ever be corrected.
    pub async fn set_display_name(&self, name: &str) -> Result<(), AccountError> {
        let client = self.client();
        let name = name.to_owned();
        let stored = name.clone();

        spawn_tokio!(async move { client.account().set_display_name(Some(&name)).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| AccountError::Server(Box::new(error)))?;

        let mut profile = self.inner.profile.get();
        profile.display_name = Some(stored);
        self.inner.profile.set_if_not_eq(profile);

        Ok(())
    }

    /// Upload the given image and make it the account's avatar.
    ///
    /// Split into an upload and a set of the avatar URL exactly as the
    /// application splits it, because the URI the upload returns is what
    /// the local profile has to be corrected with.
    pub async fn set_avatar(&self, mime: &mime::Mime, data: Vec<u8>) -> Result<(), AccountError> {
        let client = self.client();
        let mime = mime.clone();

        let uri = spawn_tokio!(async move { client.media().upload(&mime, data, None).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| AccountError::Server(Box::new(error)))?
            .content_uri;

        let client = self.client();
        let stored = uri.clone();
        spawn_tokio!(async move { client.account().set_avatar_url(Some(&uri)).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| AccountError::Server(Box::new(error)))?;

        let mut profile = self.inner.profile.get();
        profile.avatar_url = Some(stored);
        self.inner.profile.set_if_not_eq(profile);

        Ok(())
    }

    /// A weak reference to this session.
    #[must_use]
    pub fn downgrade(&self) -> WeakSession {
        WeakSession(Arc::downgrade(&self.inner))
    }

    /// The room list of this session.
    #[must_use]
    pub fn room_list(&self) -> &RoomList {
        self.inner
            .room_list
            .get_or_init(|| RoomList::new(self.downgrade()))
    }

    /// The users this account ignores.
    #[must_use]
    pub fn ignored_users(&self) -> &IgnoredUsers {
        self.inner
            .ignored_users
            .get_or_init(|| IgnoredUsers::new(self.downgrade()))
    }

    /// The account's other sessions.
    ///
    /// Not loaded by `prepare()`: the list costs two requests and only the
    /// account settings ever look at it, so it loads on first use.
    #[must_use]
    pub fn user_sessions(&self) -> &UserSessions {
        self.inner
            .user_sessions
            .get_or_init(|| UserSessions::new(self.downgrade()))
    }

    /// Feed the room list from the sync loop's room updates.
    fn consume_room_updates(&self) {
        let Some(mut receiver) = self
            .inner
            .room_updates_rx
            .lock()
            .expect("mutex is not poisoned")
            .take()
        else {
            return;
        };

        let weak = self.downgrade();
        let handle = RUNTIME
            .spawn(async move {
                while let Some(updates) = receiver.recv().await {
                    let Some(session) = weak.upgrade() else {
                        break;
                    };
                    session.room_list().handle_room_updates(updates);
                }
            })
            .abort_handle();

        *self
            .inner
            .room_updates_handle
            .lock()
            .expect("mutex is not poisoned") = Some(handle);
    }

    /// Tell the session that the platform's network connectivity changed.
    ///
    /// The embedder calls this from whatever its platform offers —
    /// `NetworkMonitor` on GTK, `ConnectivityManager` on Android — and the
    /// session finds out what that means for the homeserver by probing it.
    pub fn network_changed(&self) {
        let weak = Arc::downgrade(&self.inner);
        RUNTIME.spawn(async move {
            if let Some(inner) = weak.upgrade() {
                SessionInner::update_homeserver_reachable(&inner).await;
            }
        });
    }

    /// Drop any stale offline claim and find out fresh.
    ///
    /// For the moment the application comes back to the foreground, when what
    /// this session last learned about its connection predates a background
    /// freeze. See the application's `session/mod.rs` for the full story.
    pub fn recheck_connectivity(&self) {
        let inner = &self.inner;

        if inner.state.get() < SessionState::InitialSync {
            return;
        }

        let was_struggling =
            inner.is_offline.get() || inner.missed_sync_count.load(Ordering::Relaxed) > 0;
        inner.missed_sync_count.store(0, Ordering::Relaxed);
        inner.set_offline(false);

        if !was_struggling {
            // The sync loop is healthy — its running long-poll is already
            // the fresh check.
            return;
        }

        if inner.is_homeserver_reachable.get() {
            // Restart the sync loop rather than let it sleep out a backoff
            // delay measured against a network that no longer exists.
            inner.abort_sync();
            SessionInner::sync(inner);
        } else {
            // The reachability check restarts the sync loop itself when it
            // succeeds.
            self.network_changed();
        }
    }

    /// Log out of this session.
    pub async fn log_out(&self) -> Result<(), String> {
        debug!(
            session = self.session_id(),
            "The session is about to be logged out"
        );

        // The Android pusher removal returns with the push chunk; it has to
        // go before the access token that can remove it does.

        let client = self.client();
        let handle = spawn_tokio!(async move { client.logout().await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => {
                self.inner.clean_up().await;
                Ok(())
            }
            Err(logout_error) => {
                error!(
                    session = self.session_id(),
                    "Could not log the session out: {logout_error}"
                );
                Err("Could not log the session out".to_owned())
            }
        }
    }

    /// Clean up this session after it was logged out.
    ///
    /// This should only be called if the session has been logged out without
    /// calling [`Session::log_out()`].
    pub async fn clean_up(&self) {
        self.inner.clean_up().await;
    }
}

impl SessionInner {
    /// Set the current state of the session.
    fn set_state(&self, state: SessionState) {
        let old_state = self.state.get();

        if old_state == SessionState::LoggedOut || old_state == state {
            // The session should be dismissed when it has been logged out, so
            // we do not accept anymore state changes.
            return;
        }

        self.state.set(state);
    }

    /// Abort the running sync loop, if any.
    fn abort_sync(&self) {
        if let Some(handle) = self
            .sync_handle
            .lock()
            .expect("mutex is not poisoned")
            .take()
        {
            handle.abort();
        }
    }

    /// Whether the homeserver answers a TCP dial.
    async fn probe_homeserver(&self) -> bool {
        let homeserver = &self.info.homeserver;

        let Some(host) = homeserver.host_str() else {
            error!(
                session = self.info.id,
                "Homeserver URL has no host to probe"
            );
            return false;
        };
        let port = homeserver.port_or_known_default().unwrap_or(443);

        match tokio::time::timeout(
            REACHABILITY_TIMEOUT,
            TcpStream::connect((host.to_owned(), port)),
        )
        .await
        {
            Ok(Ok(_)) => true,
            Ok(Err(connect_error)) => {
                error!(
                    session = self.info.id,
                    "Homeserver is not reachable: {connect_error}"
                );
                false
            }
            Err(_) => {
                error!(
                    session = self.info.id,
                    "Homeserver is not reachable: probe timed out"
                );
                false
            }
        }
    }

    /// Check whether the homeserver is reachable.
    async fn update_homeserver_reachable(self: &Arc<Self>) {
        // If there is a retry pending, cancel it, we will add a new one later
        // if needed.
        if let Some(handle) = self
            .reachability_retry_handle
            .lock()
            .expect("mutex is not poisoned")
            .take()
        {
            handle.abort();
        }

        let Ok(_guard) = self.reachability_lock.try_lock() else {
            // There is an ongoing check.
            return;
        };

        let is_homeserver_reachable = self.probe_homeserver().await;

        self.set_is_homeserver_reachable(is_homeserver_reachable);

        if !is_homeserver_reachable {
            // Check again later if the homeserver is reachable.
            let weak = Arc::downgrade(self);
            let handle = RUNTIME
                .spawn(async move {
                    sleep(REACHABILITY_RETRY_DELAY).await;
                    if let Some(inner) = weak.upgrade() {
                        update_homeserver_reachable_boxed(inner).await;
                    }
                })
                .abort_handle();
            *self
                .reachability_retry_handle
                .lock()
                .expect("mutex is not poisoned") = Some(handle);
        }
    }

    /// Set whether the homeserver is reachable.
    fn set_is_homeserver_reachable(self: &Arc<Self>, is_reachable: bool) {
        if self.is_homeserver_reachable.get() == is_reachable {
            return;
        }

        self.is_homeserver_reachable.set(is_reachable);
        self.abort_sync();

        if is_reachable {
            info!(session = self.info.id, "Homeserver is reachable");

            // Restart the sync loop.
            Self::sync(self);
        } else {
            self.set_offline(true);
        }
    }

    /// Set whether this session is synchronized with the homeserver.
    fn set_offline(&self, is_offline: bool) {
        if self.is_offline.get() == is_offline {
            return;
        }

        if !is_offline {
            // Restart the send queues, in case they were stopped.
            let client = (*self.client).clone();
            spawn_tokio!(async move {
                client.send_queue().set_enabled(true).await;
            });
        }

        self.is_offline.set(is_offline);
    }

    /// Watch the changes of the session, like being logged out or the
    /// tokens being refreshed.
    fn watch_session_changes(self: &Arc<Self>) {
        let receiver = self.client.subscribe_to_session_changes();
        let weak = Arc::downgrade(self);

        let handle = RUNTIME
            .spawn(async move {
                let mut stream = BroadcastStream::new(receiver);

                while let Some(change) = stream.next().await {
                    let Ok(change) = change else {
                        continue;
                    };
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };

                    match change {
                        SessionChange::UnknownToken { .. } => {
                            info!(
                                session = inner.info.id,
                                "The access token is invalid, cleaning up the session…"
                            );
                            inner.clean_up().await;
                        }
                        SessionChange::TokensRefreshed => {
                            inner.store_tokens().await;
                        }
                    }
                }
            })
            .abort_handle();

        *self
            .session_changes_handle
            .lock()
            .expect("mutex is not poisoned") = Some(handle);
    }

    /// Start syncing the Matrix client.
    fn sync(self: &Arc<Self>) {
        if self.state.get() < SessionState::InitialSync || !self.is_homeserver_reachable.get() {
            return;
        }

        let client = (*self.client).clone();
        let weak = Arc::downgrade(self);

        let handle = RUNTIME
            .spawn(async move {
                // Make sure that the event cache is subscribed to sync responses to benefit
                // from it.
                if let Err(subscribe_error) = client.event_cache().subscribe() {
                    error!("Could not subscribe event cache to sync responses: {subscribe_error}");
                }

                // TODO: only create the filter once and reuse it in the future
                let filter = assign!(FilterDefinition::default(), {
                    room: assign!(RoomFilter::with_lazy_loading(), {
                        include_leave: true,
                    }),
                });

                let sync_settings = SyncSettings::new()
                    .timeout(Duration::from_secs(30))
                    .ignore_timeout_on_first_sync(true)
                    .filter(filter.into());

                let mut sync_stream = Box::pin(client.sync_stream(sync_settings).await);
                while let Some(response) = sync_stream.next().await {
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };
                    let delay = inner.handle_sync_response(response);
                    drop(inner);

                    if let Some(delay) = delay {
                        sleep(delay).await;
                    }
                }
            })
            .abort_handle();

        *self.sync_handle.lock().expect("mutex is not poisoned") = Some(handle);
    }

    /// Handle the response received via sync.
    ///
    /// Returns the delay to wait for before making the next sync, if
    /// necessary.
    fn handle_sync_response(
        &self,
        response: Result<SyncResponse, matrix_sdk::Error>,
    ) -> Option<Duration> {
        debug!(session = self.info.id, "Received sync response");

        match response {
            Ok(response) => {
                // Buffered until the room list takes the receiving end; a
                // dropped receiver is fine too.
                let _ = self.room_updates_tx.send(response.rooms);

                if self.state.get() < SessionState::Ready {
                    self.set_state(SessionState::Ready);
                    // The notification handler and the Android pusher
                    // registration attach here with their chunks.
                }

                self.set_offline(false);
                self.missed_sync_count.store(0, Ordering::Relaxed);

                None
            }
            Err(sync_error) => {
                let missed_sync_count = self.missed_sync_count.load(Ordering::Relaxed);

                // If there are too many failed attempts, mark the session as offline.
                if missed_sync_count == MISSED_SYNC_OFFLINE_COUNT {
                    self.set_offline(true);
                }

                // Increase the count of missed syncs, if we have not reached the maximum value.
                if missed_sync_count < MISSED_SYNC_DELAYS.len() - 1 {
                    self.missed_sync_count
                        .store(missed_sync_count + 1, Ordering::Relaxed);
                }

                error!(
                    session = self.info.id,
                    "Could not perform sync: {sync_error}"
                );

                // Sleep a little between attempts.
                let delay = MISSED_SYNC_DELAYS[missed_sync_count];
                Some(Duration::from_secs(delay))
            }
        }
    }

    /// Load the cached profile of the user of this session.
    async fn init_user_profile(&self) {
        let profile = match self
            .client
            .state_store()
            .get_custom_value(SESSION_PROFILE_KEY.as_bytes())
            .await
        {
            Ok(Some(bytes)) => match serde_json::from_slice::<UserProfile>(&bytes) {
                Ok(profile) => profile,
                Err(deserialize_error) => {
                    error!(
                        session = self.info.id,
                        "Could not deserialize session profile: {deserialize_error}"
                    );
                    return;
                }
            },
            Ok(None) => return,
            Err(load_error) => {
                error!(
                    session = self.info.id,
                    "Could not load cached session profile: {load_error}"
                );
                return;
            }
        };

        self.profile.set_if_not_eq(SessionProfile {
            display_name: profile.displayname,
            avatar_url: profile.avatar_url,
        });
    }

    /// Update the profile of this session’s user.
    ///
    /// Fetches the updated profile and updates the local data.
    async fn update_user_profile(&self) {
        let profile = match self
            .client
            .account()
            .fetch_user_profile()
            .await
            .and_then(|response| {
                let mut profile = UserProfile::new();
                profile.displayname = response.get_static::<DisplayName>()?;
                profile.avatar_url = response.get_static::<AvatarUrl>()?;

                Ok(profile)
            }) {
            Ok(profile) => profile,
            Err(fetch_error) => {
                error!(
                    session = self.info.id,
                    "Could not fetch session profile: {fetch_error}"
                );
                return;
            }
        };

        let new_profile = SessionProfile {
            display_name: profile.displayname.clone(),
            avatar_url: profile.avatar_url.clone(),
        };

        if self.profile.get() == new_profile {
            // Nothing to update.
            return;
        }

        // Serialize first for caching to avoid a clone.
        let value = serde_json::to_vec(&profile);

        // Update the profile for the UI.
        self.profile.set(new_profile);

        // Update the cache.
        let value = match value {
            Ok(value) => value,
            Err(serialize_error) => {
                error!(
                    session = self.info.id,
                    "Could not serialize session profile: {serialize_error}"
                );
                return;
            }
        };

        if let Err(store_error) = self
            .client
            .state_store()
            .set_custom_value(SESSION_PROFILE_KEY.as_bytes(), value)
            .await
        {
            error!(
                session = self.info.id,
                "Could not cache session profile: {store_error}"
            );
        }
    }

    /// Update the stored session tokens.
    async fn store_tokens(&self) {
        let Some(session_tokens) = self.client.session_tokens() else {
            return;
        };

        debug!(session = self.info.id, "Storing updated session tokens…");
        self.info.store_tokens(session_tokens).await;
    }

    /// Clean up this session after it was logged out.
    ///
    /// This should only be called if the session has been logged out
    /// without calling `Session::log_out`.
    async fn clean_up(&self) {
        self.set_state(SessionState::LoggedOut);
        self.abort_sync();
        self.settings.delete();
        self.info.clone().delete().await;

        // The notification withdrawal attaches here with its chunk.

        debug!(
            session = self.info.id,
            "The logged out session was cleaned up"
        );
    }
}

/// [`SessionInner::update_homeserver_reachable()`] behind a boxed future.
///
/// The retry task calls the function that scheduled it; without the box the
/// future type would contain itself.
fn update_homeserver_reachable_boxed(inner: Arc<SessionInner>) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        SessionInner::update_homeserver_reachable(&inner).await;
    })
}

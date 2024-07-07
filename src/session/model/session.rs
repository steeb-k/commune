use std::time::Duration;

use futures_util::StreamExt;
use gettextrs::gettext;
use gtk::{
    gio, glib,
    glib::{clone, signal::SignalHandlerId},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::{
    config::SyncSettings,
    encryption::{
        recovery::RecoveryState as SdkRecoveryState, VerificationState as SdkVerificationState,
    },
    matrix_auth::MatrixSession,
    sync::SyncResponse,
    Client,
};
use ruma::{
    api::client::{
        error::ErrorKind,
        filter::{FilterDefinition, LazyLoadOptions, RoomEventFilter, RoomFilter},
        search::search_events::v3::UserProfile,
        session::logout,
    },
    assign,
    events::{direct::DirectEventContent, GlobalAccountDataEvent},
};
use tokio::task::AbortHandle;
use tracing::{debug, error};
use url::Url;

use super::{
    IgnoredUsers, ItemList, Notifications, RoomList, SessionSettings, SidebarListModel, User,
    UserSessionsList, VerificationList,
};
use crate::{
    components::AvatarData,
    prelude::*,
    secret::StoredSession,
    session_list::{SessionInfo, SessionInfoImpl},
    spawn, spawn_tokio,
    utils::{
        matrix::{self, ClientSetupError},
        TokioDrop,
    },
    Application,
};

/// The database key for persisting the session's profile.
const SESSION_PROFILE_KEY: &str = "session_profile";
/// The number of consecutive missed synchronizations before the session is
/// marked as offline.
const MISSED_SYNC_MAX_COUNT: u8 = 3;

/// The state of the session.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, glib::Enum)]
#[repr(i32)]
#[enum_type(name = "SessionState")]
pub enum SessionState {
    LoggedOut = -1,
    #[default]
    Init = 0,
    InitialSync = 1,
    Ready = 2,
}

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "BoxedClient")]
pub struct BoxedClient(Client);

/// The state of the crypto identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "CryptoIdentityState")]
pub enum CryptoIdentityState {
    /// The state is not known yet.
    #[default]
    Unknown,
    /// The crypto identity does not exist.
    ///
    /// It means that cross-signing is not set up.
    Missing,
    /// There are no other verified sessions.
    LastManStanding,
    /// There are other verified sessions.
    OtherSessions,
}

/// The state of the verification of the session.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "SessionVerificationState")]
pub enum SessionVerificationState {
    /// The state is not known yet.
    #[default]
    Unknown,
    /// The session is verified.
    Verified,
    /// The session is not verified.
    Unverified,
}

impl From<SdkVerificationState> for SessionVerificationState {
    fn from(value: SdkVerificationState) -> Self {
        match value {
            SdkVerificationState::Unknown => Self::Unknown,
            SdkVerificationState::Verified => Self::Verified,
            SdkVerificationState::Unverified => Self::Unverified,
        }
    }
}

/// The state of the recovery.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, glib::Enum)]
#[enum_type(name = "RecoveryState")]
pub enum RecoveryState {
    /// The state is not known yet.
    #[default]
    Unknown,
    /// Recovery is disabled.
    Disabled,
    /// Recovery is enabled and we have all the keys.
    Enabled,
    /// Recovery is enabled and we are missing some keys.
    Incomplete,
}

impl From<SdkRecoveryState> for RecoveryState {
    fn from(value: SdkRecoveryState) -> Self {
        match value {
            SdkRecoveryState::Unknown => Self::Unknown,
            SdkRecoveryState::Disabled => Self::Disabled,
            SdkRecoveryState::Enabled => Self::Enabled,
            SdkRecoveryState::Incomplete => Self::Incomplete,
        }
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Session)]
    pub struct Session {
        /// The Matrix client.
        #[property(construct_only)]
        pub client: TokioDrop<BoxedClient>,
        /// The list model of the sidebar.
        #[property(get = Self::sidebar_list_model)]
        pub sidebar_list_model: OnceCell<SidebarListModel>,
        /// The user of this session.
        #[property(get = Self::user)]
        pub user: OnceCell<User>,
        /// The current state of the session.
        #[property(get, builder(SessionState::default()))]
        pub state: Cell<SessionState>,
        /// Whether this session has a connection to the homeserver.
        #[property(get)]
        pub is_homeserver_reachable: Cell<bool>,
        /// Whether this session is synchronized with the homeserver.
        #[property(get)]
        pub is_offline: Cell<bool>,
        /// The current settings for this session.
        #[property(get, construct_only)]
        pub settings: OnceCell<SessionSettings>,
        /// The notifications API for this session.
        #[property(get)]
        pub notifications: Notifications,
        /// The ignored users API for this session.
        #[property(get)]
        pub ignored_users: IgnoredUsers,
        /// The list of sessions for this session's user.
        #[property(get)]
        pub user_sessions: UserSessionsList,
        /// The state of the crypto identity for this session.
        #[property(get, builder(CryptoIdentityState::default()))]
        pub crypto_identity_state: Cell<CryptoIdentityState>,
        /// The state of the verification for this session.
        #[property(get, builder(SessionVerificationState::default()))]
        pub verification_state: Cell<SessionVerificationState>,
        /// The state of recovery for this session.
        #[property(get, builder(RecoveryState::default()))]
        pub recovery_state: Cell<RecoveryState>,
        /// Whether all the cross-signing keys are available.
        #[property(get)]
        pub cross_signing_keys_available: Cell<bool>,
        /// Whether the room keys backup is enabled.
        #[property(get)]
        pub backup_enabled: Cell<bool>,
        pub sync_handle: RefCell<Option<AbortHandle>>,
        pub abort_handles: RefCell<Vec<AbortHandle>>,
        pub network_monitor_handler_id: RefCell<Option<SignalHandlerId>>,
        /// The number of missed synchonizations in a row.
        ///
        /// Capped at `MISSED_SYNC_MAX_COUNT - 1`.
        pub missed_sync_count: Cell<u8>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Session {
        const NAME: &'static str = "Session";
        type Type = super::Session;
        type ParentType = SessionInfo;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Session {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.ignored_users.set_session(Some(obj.clone()));
            self.notifications.set_session(Some(obj.clone()));
            self.user_sessions.init(&obj, obj.user_id().clone());

            let monitor = gio::NetworkMonitor::default();
            let handler_id = monitor.connect_network_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _| {
                    spawn!(async move {
                        imp.update_homeserver_reachable().await;
                    });
                }
            ));
            self.network_monitor_handler_id.replace(Some(handler_id));
        }

        fn dispose(&self) {
            // Needs to be disconnected or else it may restart the sync
            if let Some(handler_id) = self.network_monitor_handler_id.take() {
                gio::NetworkMonitor::default().disconnect(handler_id);
            }

            if let Some(handle) = self.sync_handle.take() {
                handle.abort();
            }

            for handle in self.abort_handles.take() {
                handle.abort();
            }
        }
    }

    impl SessionInfoImpl for Session {
        fn avatar_data(&self) -> AvatarData {
            self.obj().user().avatar_data().clone()
        }
    }

    impl Session {
        // The Matrix client.
        pub(super) fn client(&self) -> &Client {
            &self.client.get().expect("session should be restored").0
        }

        /// The list model of the sidebar.
        fn sidebar_list_model(&self) -> SidebarListModel {
            let obj = self.obj();
            self.sidebar_list_model
                .get_or_init(|| {
                    let item_list =
                        ItemList::new(&RoomList::new(&obj), &VerificationList::new(&obj));
                    SidebarListModel::new(&item_list)
                })
                .clone()
        }

        /// The user of the session.
        fn user(&self) -> User {
            let obj = self.obj();
            self.user
                .get_or_init(|| User::new(&obj, obj.info().user_id.clone()))
                .clone()
        }

        /// Set the crypto identity state of this session.
        pub(super) fn set_crypto_identity_state(&self, state: CryptoIdentityState) {
            if self.crypto_identity_state.get() == state {
                return;
            }

            self.crypto_identity_state.set(state);
            self.obj().notify_crypto_identity_state();
        }

        /// Set the verification state of this session.
        pub(super) fn set_verification_state(&self, state: SessionVerificationState) {
            if self.verification_state.get() == state {
                return;
            }

            self.verification_state.set(state);
            self.obj().notify_verification_state();
        }

        /// Set the recovery state of this session.
        pub(super) fn set_recovery_state(&self, state: RecoveryState) {
            if self.recovery_state.get() == state {
                return;
            }

            self.recovery_state.set(state);
            self.obj().notify_recovery_state();
        }

        /// Set whether all the cross-signing keys are available.
        pub(super) fn set_cross_signing_keys_available(&self, available: bool) {
            if self.cross_signing_keys_available.get() == available {
                return;
            }

            self.cross_signing_keys_available.set(available);
            self.obj().notify_cross_signing_keys_available();
        }

        /// Set whether the room keys backup is enabled.
        pub(super) fn set_backup_enabled(&self, enabled: bool) {
            if self.backup_enabled.get() == enabled {
                return;
            }

            self.backup_enabled.set(enabled);
            self.obj().notify_backup_enabled();
        }

        /// Update whether the homeserver is reachable.
        pub(super) async fn update_homeserver_reachable(&self) {
            let obj = self.obj();
            let monitor = gio::NetworkMonitor::default();

            let is_homeserver_reachable = if monitor.is_network_available() {
                let homeserver = obj.homeserver();
                let address = gio::NetworkAddress::parse_uri(homeserver.as_ref(), 80).unwrap();

                match monitor.can_reach_future(&address).await {
                    Ok(()) => true,
                    Err(error) => {
                        error!("Homeserver {homeserver} is not reachable: {error}");
                        false
                    }
                }
            } else {
                false
            };

            if self.is_homeserver_reachable.get() == is_homeserver_reachable {
                return;
            }

            self.is_homeserver_reachable.set(is_homeserver_reachable);

            if let Some(handle) = self.sync_handle.take() {
                handle.abort();
            }

            if is_homeserver_reachable {
                // Restart the sync loop.
                obj.sync();
            } else {
                self.set_offline(true);
            }

            obj.notify_is_homeserver_reachable();
        }

        /// Set whether this session is synchronized with the homeserver.
        pub(super) fn set_offline(&self, is_offline: bool) {
            if self.is_offline.get() == is_offline {
                return;
            }

            if is_offline {
                debug!("This session is now offline");
            } else {
                debug!("This session is now online");

                // Restart the send queues, in case they were stopped.
                let send_queue = self.client().send_queue();
                spawn_tokio!(async move {
                    send_queue.set_enabled(true).await;
                });
            }

            self.is_offline.set(is_offline);
            self.obj().notify_is_offline();
        }
    }
}

glib::wrapper! {
    /// A Matrix user session.
    pub struct Session(ObjectSubclass<imp::Session>)
        @extends SessionInfo;
}

impl Session {
    /// Create a new session.
    pub async fn new(homeserver: Url, data: MatrixSession) -> Result<Self, ClientSetupError> {
        let stored_session = StoredSession::with_login_data(homeserver, data);
        let settings = Application::default()
            .session_list()
            .settings()
            .get_or_create(stored_session.id());

        Self::restore(stored_session, settings).await
    }

    /// Restore a stored session.
    pub async fn restore(
        stored_session: StoredSession,
        settings: SessionSettings,
    ) -> Result<Self, ClientSetupError> {
        let stored_session_clone = stored_session.clone();
        let client =
            spawn_tokio!(
                async move { matrix::client_with_stored_session(stored_session_clone).await }
            )
            .await
            .unwrap()?;

        Ok(glib::Object::builder()
            .property("info", &stored_session)
            .property("settings", settings)
            .property("client", &BoxedClient(client))
            .build())
    }

    /// Set the current state of the session.
    fn set_state(&self, state: SessionState) {
        let old_state = self.state();

        if old_state == SessionState::LoggedOut || old_state == state {
            // The session should be dismissed when it has been logged out, so
            // we don't accept anymore state changes.
            return;
        }

        self.imp().state.set(state);
        self.notify_state();
    }

    /// Finish initialization of this session.
    pub async fn prepare(&self) {
        spawn!(
            glib::Priority::LOW,
            clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    // First, load the profile from the cache, it will be quicker.
                    obj.init_user_profile().await;
                    // Then, check if the profile changed.
                    obj.update_user_profile().await;
                }
            )
        );
        self.imp().update_homeserver_reachable().await;

        self.room_list().load().await;
        self.setup_direct_room_handler();
        self.verification_list().init();
        self.init_verification_state();
        self.init_recovery_state();

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                obj.init_crypto_identity_state().await;
            }
        ));

        let client = self.client();
        spawn_tokio!(async move {
            client
                .send_queue()
                .respawn_tasks_for_rooms_with_unsent_events()
                .await
        });

        self.set_state(SessionState::InitialSync);
        self.sync();

        debug!("A new session was prepared");
    }

    /// Start syncing the Matrix client.
    fn sync(&self) {
        if self.state() < SessionState::InitialSync || !self.is_homeserver_reachable() {
            return;
        }

        let client = self.client();
        let session_weak: glib::SendWeakRef<Session> = self.downgrade().into();

        let handle = spawn_tokio!(async move {
            // TODO: only create the filter once and reuse it in the future
            let room_event_filter = assign!(RoomEventFilter::default(), {
                lazy_load_options: LazyLoadOptions::Enabled {include_redundant_members: false},
            });
            let filter = assign!(FilterDefinition::default(), {
                room: assign!(RoomFilter::empty(), {
                    include_leave: true,
                    state: room_event_filter,
                }),
            });

            let sync_settings = SyncSettings::new()
                .timeout(Duration::from_secs(30))
                .filter(filter.into());

            let mut sync_stream = Box::pin(client.sync_stream(sync_settings).await);
            while let Some(response) = sync_stream.next().await {
                let session_weak = session_weak.clone();
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    if let Some(session) = session_weak.upgrade() {
                        session.handle_sync_response(response);
                    }
                });
            }
        })
        .abort_handle();

        self.imp().sync_handle.replace(Some(handle));
    }

    /// Listen to crypto identity changes.
    async fn init_crypto_identity_state(&self) {
        let client = self.client();

        let encryption = client.encryption();

        let encryption_clone = encryption.clone();
        let handle = spawn_tokio!(async move { encryption_clone.user_identities_stream().await });
        let identities_stream = match handle.await.unwrap() {
            Ok(stream) => stream,
            Err(error) => {
                error!("Could not get user identities stream: {error}");
                // All method calls here have the same error, so we can return early.
                return;
            }
        };

        let obj_weak: glib::SendWeakRef<Session> = self.downgrade().into();
        let fut = identities_stream.for_each(move |updates| {
            let obj_weak = obj_weak.clone();

            async move {
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    spawn!(async move {
                        if let Some(obj) = obj_weak.upgrade() {
                            let own_user_id = obj.user_id();
                            if updates.new.contains_key(own_user_id)
                                || updates.changed.contains_key(own_user_id)
                            {
                                obj.load_crypto_identity_state().await;
                            }
                        }
                    });
                });
            }
        });
        let identities_abort_handle = spawn_tokio!(fut).abort_handle();

        let handle = spawn_tokio!(async move { encryption.devices_stream().await });
        let devices_stream = match handle.await.unwrap() {
            Ok(stream) => stream,
            Err(error) => {
                error!("Could not get devices stream: {error}");
                // All method calls here have the same error, so we can return early.
                return;
            }
        };

        let obj_weak: glib::SendWeakRef<Session> = self.downgrade().into();
        let fut = devices_stream.for_each(move |updates| {
            let obj_weak = obj_weak.clone();

            async move {
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    spawn!(async move {
                        if let Some(obj) = obj_weak.upgrade() {
                            let own_user_id = obj.user_id();
                            if updates.new.contains_key(own_user_id)
                                || updates.changed.contains_key(own_user_id)
                            {
                                obj.load_crypto_identity_state().await;
                            }
                        }
                    });
                });
            }
        });
        let devices_abort_handle = spawn_tokio!(fut).abort_handle();

        self.imp()
            .abort_handles
            .borrow_mut()
            .extend([identities_abort_handle, devices_abort_handle]);

        self.load_crypto_identity_state().await;
    }

    /// Load the crypto identity state.
    async fn load_crypto_identity_state(&self) {
        let imp = self.imp();
        let client = self.client();

        let client_clone = client.clone();
        let user_identity_handle = spawn_tokio!(async move {
            let user_id = client_clone.user_id().unwrap();
            client_clone.encryption().get_user_identity(user_id).await
        });

        let has_identity = match user_identity_handle.await.unwrap() {
            Ok(Some(_)) => true,
            Ok(None) => {
                debug!("No crypto user identity found");
                false
            }
            Err(error) => {
                error!("Could not get crypto user identity: {error}");
                false
            }
        };

        if !has_identity {
            imp.set_crypto_identity_state(CryptoIdentityState::Missing);
            return;
        }

        let devices_handle = spawn_tokio!(async move {
            let user_id = client.user_id().unwrap();
            client.encryption().get_user_devices(user_id).await
        });

        let own_device = self.device_id();
        let has_other_sessions = match devices_handle.await.unwrap() {
            Ok(devices) => devices
                .devices()
                .any(|d| d.device_id() != own_device && d.is_cross_signed_by_owner()),
            Err(error) => {
                error!("Could not get user devices: {error}");
                // If there are actually no other devices, the user can still
                // reset the crypto identity.
                true
            }
        };

        let state = if has_other_sessions {
            CryptoIdentityState::OtherSessions
        } else {
            CryptoIdentityState::LastManStanding
        };

        imp.set_crypto_identity_state(state);
    }

    /// Listen to verification state changes.
    fn init_verification_state(&self) {
        let client = self.client();
        let mut stream = client.encryption().verification_state();
        // Get the current value right away.
        stream.reset();

        let obj_weak: glib::SendWeakRef<Session> = self.downgrade().into();
        let fut = stream.for_each(move |state| {
            let obj_weak = obj_weak.clone();

            async move {
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    spawn!(async move {
                        if let Some(obj) = obj_weak.upgrade() {
                            obj.imp().set_verification_state(state.into());
                        }
                    });
                });
            }
        });
        let verification_abort_handle = spawn_tokio!(fut).abort_handle();

        self.imp()
            .abort_handles
            .borrow_mut()
            .push(verification_abort_handle);
    }

    /// Listen to recovery state changes.
    fn init_recovery_state(&self) {
        let client = self.client();

        let obj_weak: glib::SendWeakRef<Session> = self.downgrade().into();
        let stream = client.encryption().recovery().state_stream();

        let fut = stream.for_each(move |state| {
            let obj_weak = obj_weak.clone();

            async move {
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    spawn!(async move {
                        if let Some(obj) = obj_weak.upgrade() {
                            obj.update_recovery_state(state.into()).await;
                        }
                    });
                });
            }
        });

        let abort_handle = spawn_tokio!(fut).abort_handle();
        self.imp().abort_handles.borrow_mut().push(abort_handle);
    }

    /// Update the session for the given recovery state.
    async fn update_recovery_state(&self, state: RecoveryState) {
        let imp = self.imp();

        match state {
            RecoveryState::Enabled => {
                imp.set_cross_signing_keys_available(true);
                imp.set_backup_enabled(true);
            }
            _ => {
                let encryption = self.client().encryption();
                let backups = encryption.backups();

                let handle = spawn_tokio!(async move { encryption.cross_signing_status().await });
                let cross_signing_keys_available =
                    handle.await.unwrap().is_some_and(|s| s.is_complete());
                imp.set_cross_signing_keys_available(cross_signing_keys_available);

                let handle = spawn_tokio!(async move { backups.are_enabled().await });
                let backup_enabled = handle.await.unwrap();
                imp.set_backup_enabled(backup_enabled);
            }
        }

        imp.set_recovery_state(state);
    }

    /// Start listening to notifications.
    pub async fn init_notifications(&self) {
        let obj_weak = glib::SendWeakRef::from(self.downgrade());
        let client = self.client();
        spawn_tokio!(async move {
            client
                .register_notification_handler(move |notification, room, _| {
                    let obj_weak = obj_weak.clone();
                    async move {
                        let ctx = glib::MainContext::default();
                        ctx.spawn(async move {
                            spawn!(async move {
                                if let Some(obj) = obj_weak.upgrade() {
                                    obj.notifications().show_push(notification, room).await;
                                }
                            });
                        });
                    }
                })
                .await;
        })
        .await
        .unwrap();
    }

    /// The room list of this session.
    pub fn room_list(&self) -> RoomList {
        self.sidebar_list_model().item_list().room_list()
    }

    /// The verification list of this session.
    pub fn verification_list(&self) -> VerificationList {
        self.sidebar_list_model().item_list().verification_list()
    }

    /// Load the cached profile of this session’s user.
    async fn init_user_profile(&self) {
        let client = self.client();
        let handle = spawn_tokio!(async move {
            client
                .store()
                .get_custom_value(SESSION_PROFILE_KEY.as_bytes())
                .await
        });

        let profile = match handle.await.unwrap() {
            Ok(Some(bytes)) => match serde_json::from_slice::<UserProfile>(&bytes) {
                Ok(profile) => profile,
                Err(error) => {
                    error!("Failed to deserialize session profile: {error}");
                    return;
                }
            },
            Ok(None) => return,
            Err(error) => {
                error!("Could not load cached session profile: {error}");
                return;
            }
        };

        let user = self.user();

        user.set_name(profile.displayname);
        user.set_avatar_url(profile.avatar_url);
    }

    /// Update the profile of this session’s user.
    ///
    /// Fetches the updated profile and updates the local data.
    async fn update_user_profile(&self) {
        let client = self.client();
        let client_clone = client.clone();
        let handle = spawn_tokio!(async move { client_clone.account().fetch_user_profile().await });

        let profile = match handle.await.unwrap() {
            Ok(res) => {
                let mut profile = UserProfile::new();
                profile.displayname = res.displayname;
                profile.avatar_url = res.avatar_url;

                profile
            }
            Err(error) => {
                error!("Could not fetch session profile: {error}");
                return;
            }
        };

        let user = self.user();

        if Some(user.display_name()) == profile.displayname
            && user.avatar_data().image().is_some_and(|i| {
                i.uri().as_deref() == profile.avatar_url.as_ref().map(|url| url.as_str())
            })
        {
            // Nothing to update.
            return;
        }

        // Serialize first for caching to avoid a clone.
        let value = serde_json::to_vec(&profile);

        // Update the profile for the UI.
        user.set_name(profile.displayname);
        user.set_avatar_url(profile.avatar_url);

        // Update the cache.
        let value = match value {
            Ok(value) => value,
            Err(error) => {
                error!("Failed to serialize session profile: {error}");
                return;
            }
        };

        let handle = spawn_tokio!(async move {
            client
                .store()
                .set_custom_value(SESSION_PROFILE_KEY.as_bytes(), value)
                .await
        });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not cache session profile: {error}");
        };
    }

    /// The Matrix client.
    pub fn client(&self) -> Client {
        self.imp().client().clone()
    }

    /// Connect to the signal emitted when this session is logged out.
    pub fn connect_logged_out<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_state_notify(move |obj| {
            if obj.state() == SessionState::LoggedOut {
                f(obj);
            }
        })
    }

    /// Connect to the signal emitted when this session is ready.
    pub fn connect_ready<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_state_notify(move |obj| {
            if obj.state() == SessionState::Ready {
                f(obj);
            }
        })
    }

    /// Handle the response received via sync.
    fn handle_sync_response(&self, response: Result<SyncResponse, matrix_sdk::Error>) {
        debug!("Received sync response");
        let imp = self.imp();

        match response {
            Ok(response) => {
                self.room_list().handle_room_updates(response.rooms);

                if self.state() < SessionState::Ready {
                    self.set_state(SessionState::Ready);
                }

                imp.set_offline(false);
                imp.missed_sync_count.set(0);
            }
            Err(error) => {
                if let Some(ErrorKind::UnknownToken { .. }) = error.client_api_error_kind() {
                    self.handle_logged_out();
                } else {
                    let missed_sync_count = imp.missed_sync_count.get() + 1;

                    if missed_sync_count >= MISSED_SYNC_MAX_COUNT {
                        imp.set_offline(true);
                    } else {
                        imp.missed_sync_count.set(missed_sync_count);
                    }
                }
                error!("Could not perform sync: {error}");
            }
        }
    }

    /// Log out of this session.
    pub async fn logout(&self) -> Result<(), String> {
        debug!("The session is about to be logged out");

        let client = self.client();
        let handle = spawn_tokio!(async move {
            let request = logout::v3::Request::new();
            client.send(request, None).await
        });

        match handle.await.unwrap() {
            Ok(_) => {
                self.clean_up().await;

                Ok(())
            }
            Err(error) => {
                error!("Could not log the session out: {error}");

                Err(gettext("Could not log the session out"))
            }
        }
    }

    /// Handle that the session has been logged out.
    ///
    /// This should only be called if the session has been logged out without
    /// `Session::logout`.
    pub fn handle_logged_out(&self) {
        // TODO: Show error screen. See: https://gitlab.gnome.org/World/fractal/-/issues/901

        spawn!(
            glib::Priority::LOW,
            clone!(
                #[strong(rename_to = obj)]
                self,
                async move {
                    obj.clean_up().await;
                }
            )
        );
    }

    /// Clean up this session after it was logged out.
    pub async fn clean_up(&self) {
        let imp = self.imp();

        self.set_state(SessionState::LoggedOut);

        if let Some(handle) = imp.sync_handle.take() {
            handle.abort();
        }

        if let Some(settings) = imp.settings.get() {
            settings.delete();
        }

        self.info().clone().delete().await;

        self.notifications().clear();

        debug!("The logged out session was cleaned up");
    }

    /// Listen to changes to the list of direct rooms.
    fn setup_direct_room_handler(&self) {
        let session_weak = glib::SendWeakRef::from(self.downgrade());
        self.client().add_event_handler(
            move |_event: GlobalAccountDataEvent<DirectEventContent>| {
                let session_weak = session_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(session) = session_weak.upgrade() {
                                // We update all rooms as we don't know which
                                // ones are no longer direct.
                                for room in session.room_list().snapshot() {
                                    room.load_is_direct().await;
                                }
                            }
                        });
                    });
                }
            },
        );
    }
}

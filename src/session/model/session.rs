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
    authentication::matrix::MatrixSession, config::SyncSettings, media::MediaRetentionPolicy,
    sync::SyncResponse, Client, SessionChange,
};
use ruma::{
    api::client::{
        filter::{FilterDefinition, RoomFilter},
        search::search_events::v3::UserProfile,
    },
    assign,
};
use tokio::task::AbortHandle;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, error, warn};
use url::Url;

use super::{
    IgnoredUsers, Notifications, RoomList, SessionSecurity, SessionSettings, SidebarItemList,
    SidebarListModel, User, UserSessionsList, VerificationList,
};
use crate::{
    components::AvatarData,
    prelude::*,
    secret::StoredSession,
    session_list::{SessionInfo, SessionInfoImpl},
    spawn, spawn_tokio,
    utils::{
        matrix::{self, ClientSetupError},
        oauth, TokioDrop,
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

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use async_once_cell::OnceCell as AsyncOnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Session)]
    pub struct Session {
        /// The Matrix client.
        #[property(construct_only)]
        client: TokioDrop<BoxedClient>,
        /// The list model of the sidebar.
        #[property(get = Self::sidebar_list_model)]
        sidebar_list_model: OnceCell<SidebarListModel>,
        /// The user of this session.
        #[property(get = Self::user)]
        user: OnceCell<User>,
        /// The current state of the session.
        #[property(get, builder(SessionState::default()))]
        state: Cell<SessionState>,
        /// Whether this session has a connection to the homeserver.
        #[property(get)]
        is_homeserver_reachable: Cell<bool>,
        /// Whether this session is synchronized with the homeserver.
        #[property(get)]
        is_offline: Cell<bool>,
        /// The current settings for this session.
        #[property(get, construct_only)]
        settings: OnceCell<SessionSettings>,
        /// The notifications API for this session.
        #[property(get)]
        notifications: Notifications,
        /// The ignored users API for this session.
        #[property(get)]
        ignored_users: IgnoredUsers,
        /// The list of sessions for this session's user.
        #[property(get)]
        user_sessions: UserSessionsList,
        /// Information about security for this session.
        #[property(get)]
        security: SessionSecurity,
        session_changes_handle: RefCell<Option<AbortHandle>>,
        sync_handle: RefCell<Option<AbortHandle>>,
        network_monitor_handler_id: RefCell<Option<SignalHandlerId>>,
        /// The number of missed synchonizations in a row.
        ///
        /// Capped at `MISSED_SYNC_MAX_COUNT - 1`.
        missed_sync_count: Cell<u8>,
        /// The OIDC authentication issuer, if any.
        auth_issuer: AsyncOnceCell<Url>,
        /// The account management URL of the OIDC authentication issuer, if
        /// any.
        account_management_url: AsyncOnceCell<Url>,
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

            if let Some(handle) = self.session_changes_handle.take() {
                handle.abort();
            }

            if let Some(handle) = self.sync_handle.take() {
                handle.abort();
            }
        }
    }

    impl SessionInfoImpl for Session {
        fn avatar_data(&self) -> AvatarData {
            self.user().avatar_data().clone()
        }
    }

    impl Session {
        // The Matrix client.
        pub(super) fn client(&self) -> &Client {
            &self.client.get().expect("session should be restored").0
        }

        /// The list model of the sidebar.
        fn sidebar_list_model(&self) -> SidebarListModel {
            self.sidebar_list_model
                .get_or_init(|| {
                    let obj = self.obj();
                    let item_list =
                        SidebarItemList::new(&RoomList::new(&obj), &VerificationList::new(&obj));
                    SidebarListModel::new(&item_list)
                })
                .clone()
        }

        /// The room list of this session.
        pub(super) fn room_list(&self) -> RoomList {
            self.sidebar_list_model().item_list().room_list()
        }

        /// The verification list of this session.
        pub(super) fn verification_list(&self) -> VerificationList {
            self.sidebar_list_model().item_list().verification_list()
        }

        /// The user of the session.
        fn user(&self) -> User {
            self.user
                .get_or_init(|| {
                    let obj = self.obj();
                    User::new(&obj, obj.info().user_id.clone())
                })
                .clone()
        }

        /// Set the current state of the session.
        fn set_state(&self, state: SessionState) {
            let old_state = self.state.get();

            if old_state == SessionState::LoggedOut || old_state == state {
                // The session should be dismissed when it has been logged out, so
                // we do not accept anymore state changes.
                return;
            }

            self.state.set(state);
            self.obj().notify_state();
        }

        /// Update whether the homeserver is reachable.
        pub(super) async fn update_homeserver_reachable(&self) {
            let obj = self.obj();
            let monitor = gio::NetworkMonitor::default();

            let is_homeserver_reachable = if monitor.is_network_available() {
                let homeserver = obj.homeserver();
                let address = gio::NetworkAddress::parse_uri(homeserver.as_ref(), 80)
                    .expect("url is parsed successfully");

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
                self.sync();
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

            if !is_offline {
                // Restart the send queues, in case they were stopped.
                let client = self.client().clone();
                spawn_tokio!(async move {
                    client.send_queue().set_enabled(true).await;
                });
            }

            self.is_offline.set(is_offline);
            self.obj().notify_is_offline();
        }

        /// Finish initialization of this session.
        pub(super) async fn prepare(&self) {
            spawn!(
                glib::Priority::LOW,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        // First, load the profile from the cache, it will be quicker.
                        imp.init_user_profile().await;
                        // Then, check if the profile changed.
                        imp.update_user_profile().await;
                    }
                )
            );
            self.watch_session_changes();
            self.update_homeserver_reachable().await;

            self.room_list().load().await;
            self.verification_list().init();
            self.security.set_session(Some(&*self.obj()));

            let client = self.client().clone();
            spawn_tokio!(async move {
                client
                    .send_queue()
                    .respawn_tasks_for_rooms_with_unsent_requests()
                    .await;
            });

            self.set_state(SessionState::InitialSync);
            self.sync();

            debug!("A new session was prepared");
        }

        /// Watch the changes of the session, like being logged out or the
        /// tokens being refreshed.
        fn watch_session_changes(&self) {
            let receiver = self.client().subscribe_to_session_changes();
            let stream = BroadcastStream::new(receiver);

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let fut = stream.for_each(move |change| {
                let obj_weak = obj_weak.clone();
                async move {
                    let Ok(change) = change else {
                        return;
                    };

                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                match change {
                                    SessionChange::UnknownToken { .. } => {
                                        obj.imp().clean_up().await;
                                    }
                                    SessionChange::TokensRefreshed => {
                                        obj.imp().store_tokens().await;
                                    }
                                }
                            }
                        });
                    });
                }
            });

            let handle = spawn_tokio!(fut).abort_handle();
            self.session_changes_handle.replace(Some(handle));
        }

        /// Start syncing the Matrix client.
        fn sync(&self) {
            if self.state.get() < SessionState::InitialSync || !self.is_homeserver_reachable.get() {
                return;
            }

            let client = self.client().clone();
            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());

            let handle = spawn_tokio!(async move {
                // TODO: only create the filter once and reuse it in the future
                let filter = assign!(FilterDefinition::default(), {
                    room: assign!(RoomFilter::with_lazy_loading(), {
                        include_leave: true,
                    }),
                });

                let sync_settings = SyncSettings::new()
                    .timeout(Duration::from_secs(30))
                    .filter(filter.into());

                let mut sync_stream = Box::pin(client.sync_stream(sync_settings).await);
                while let Some(response) = sync_stream.next().await {
                    let obj_weak = obj_weak.clone();
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.imp().handle_sync_response(response);
                            }
                        });
                    });
                }
            })
            .abort_handle();

            self.sync_handle.replace(Some(handle));
        }

        /// Handle the response received via sync.
        fn handle_sync_response(&self, response: Result<SyncResponse, matrix_sdk::Error>) {
            debug!("Received sync response");

            match response {
                Ok(response) => {
                    self.room_list().handle_room_updates(response.rooms);

                    if self.state.get() < SessionState::Ready {
                        self.set_state(SessionState::Ready);
                        self.init_notifications();
                    }

                    self.set_offline(false);
                    self.missed_sync_count.set(0);
                }
                Err(error) => {
                    let missed_sync_count = self.missed_sync_count.get() + 1;

                    if missed_sync_count >= MISSED_SYNC_MAX_COUNT {
                        self.set_offline(true);
                    } else {
                        self.missed_sync_count.set(missed_sync_count);
                    }
                    error!("Could not perform sync: {error}");
                }
            }
        }

        /// Load the cached profile of the user of this session.
        async fn init_user_profile(&self) {
            let client = self.client().clone();
            let handle = spawn_tokio!(async move {
                client
                    .store()
                    .get_custom_value(SESSION_PROFILE_KEY.as_bytes())
                    .await
            });

            let profile = match handle.await.expect("task was not aborted") {
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
            let client = self.client().clone();
            let client_clone = client.clone();
            let handle =
                spawn_tokio!(async move { client_clone.account().fetch_user_profile().await });

            let profile = match handle.await.expect("task was not aborted") {
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
                && user
                    .avatar_data()
                    .image()
                    .is_some_and(|i| i.uri() == profile.avatar_url)
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

            if let Err(error) = handle.await.expect("task was not aborted") {
                error!("Could not cache session profile: {error}");
            }
        }

        /// Start listening to notifications.
        fn init_notifications(&self) {
            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let client = self.client().clone();

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
            });
        }

        /// Update the stored session tokens.
        async fn store_tokens(&self) {
            let Some(sdk_session) = self.client().session() else {
                return;
            };

            debug!("Storing updated session tokens…");
            self.obj().info().store_tokens(sdk_session.into()).await;
        }

        /// Clean up this session after it was logged out.
        ///
        /// This should only be called if the session has been logged out
        /// without calling `Session::log_out`.
        pub(super) async fn clean_up(&self) {
            self.set_state(SessionState::LoggedOut);

            if let Some(handle) = self.sync_handle.take() {
                handle.abort();
            }

            if let Some(settings) = self.settings.get() {
                settings.delete();
            }

            self.obj().info().clone().delete().await;

            self.notifications.clear();

            debug!("The logged out session was cleaned up");
        }

        /// The OAuth 2.0 authorization provider, if any.
        async fn auth_issuer(&self) -> Option<&Url> {
            self.auth_issuer
                .get_or_try_init(clone!(
                    #[strong(rename_to = imp)]
                    self,
                    async move {
                        let client = imp.client().clone();

                        spawn_tokio!(
                            async move { oauth::fetch_auth_issuer(&client).await.ok_or(()) }
                        )
                        .await
                        .expect("task was not aborted")
                    }
                ))
                .await
                .ok()
        }

        /// The account management URL of the OAuth 2.0 authorization provider,
        /// if any.
        pub(super) async fn account_management_url(&self) -> Option<&Url> {
            self.account_management_url
                .get_or_try_init(clone!(
                    #[strong(rename_to = imp)]
                    self,
                    async move {
                        let auth_issuer = imp.auth_issuer().await.ok_or(())?.clone();

                        let client = imp.client().clone();
                        spawn_tokio!(async move {
                            oauth::discover_account_management_url(&client, auth_issuer)
                                .await
                                .map_err(|error| {
                                    warn!("Could not discover account management URL: {error}");
                                })
                        })
                        .await
                        .expect("task was not aborted")
                    }
                ))
                .await
                .ok()
        }
    }
}

glib::wrapper! {
    /// A Matrix user session.
    pub struct Session(ObjectSubclass<imp::Session>)
        @extends SessionInfo;
}

impl Session {
    /// Construct an existing session.
    pub(crate) async fn new(
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

        Ok(glib::Object::builder()
            .property("info", stored_session)
            .property("settings", settings)
            .property("client", BoxedClient(client))
            .build())
    }

    /// Create a new session after login.
    pub(crate) async fn create(
        homeserver: Url,
        data: MatrixSession,
    ) -> Result<Self, ClientSetupError> {
        let stored_session = StoredSession::new(homeserver, data.meta, data.tokens.into()).await?;
        let settings = Application::default()
            .session_list()
            .settings()
            .get_or_create(&stored_session.id);

        Self::new(stored_session, settings).await
    }

    /// Finish initialization of this session.
    pub(crate) async fn prepare(&self) {
        self.imp().prepare().await;
    }

    /// The room list of this session.
    pub(crate) fn room_list(&self) -> RoomList {
        self.imp().room_list()
    }

    /// The verification list of this session.
    pub(crate) fn verification_list(&self) -> VerificationList {
        self.imp().verification_list()
    }

    /// The Matrix client.
    pub(crate) fn client(&self) -> Client {
        self.imp().client().clone()
    }

    /// The account management URL of the OIDC authentication issuer, if any.
    pub(crate) async fn account_management_url(&self) -> Option<&Url> {
        self.imp().account_management_url().await
    }

    /// Log out of this session.
    pub(crate) async fn log_out(&self) -> Result<(), String> {
        debug!("The session is about to be logged out");

        let client = self.client();
        let handle = spawn_tokio!(async move { client.matrix_auth().logout().await });

        match handle.await.expect("task was not aborted") {
            Ok(_) => {
                self.imp().clean_up().await;
                Ok(())
            }
            Err(error) => {
                error!("Could not log the session out: {error}");
                Err(gettext("Could not log the session out"))
            }
        }
    }

    /// Clean up this session after it was logged out.
    ///
    /// This should only be called if the session has been logged out without
    /// calling `Session::log_out`.
    pub(crate) async fn clean_up(&self) {
        self.imp().clean_up().await;
    }

    /// Connect to the signal emitted when this session is logged out.
    pub(crate) fn connect_logged_out<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_state_notify(move |obj| {
            if obj.state() == SessionState::LoggedOut {
                f(obj);
            }
        })
    }

    /// Connect to the signal emitted when this session is ready.
    pub(crate) fn connect_ready<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_state_notify(move |obj| {
            if obj.state() == SessionState::Ready {
                f(obj);
            }
        })
    }
}

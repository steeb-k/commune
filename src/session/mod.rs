use commune_core::session::{
    Session as CoreSession, SessionProfile, SessionState as CoreSessionState,
};
use futures_util::lock::Mutex;
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::Client;
use tokio::task::AbortHandle;
use tracing::{debug, error};

// Calls are WebRTC over GStreamer, which is not cross-built for Android; see
// `doc/android.md`.
#[cfg(not(target_os = "android"))]
mod calls;
mod global_account_data;
mod identity_server;
mod ignored_users;
mod image_packs;
mod notifications;
mod presence;
mod remote;
mod room;
mod room_list;
mod security;
mod session_settings;
mod sidebar_data;
mod user;
mod user_sessions_list;
mod verification;

#[cfg(not(target_os = "android"))]
pub(crate) use self::calls::*;
pub(crate) use self::{
    global_account_data::*, identity_server::*, ignored_users::*, image_packs::*, notifications::*,
    presence::*, remote::*, room::*, room_list::*, security::*, session_settings::*,
    sidebar_data::*, user::*, user_sessions_list::*, verification::*,
};
use crate::{
    Application,
    components::AvatarData,
    core_bridge::ObjectWatcher,
    prelude::*,
    secret::StoredSession,
    session_list::{SessionInfo, SessionInfoImpl},
    spawn, spawn_tokio,
    utils::matrix::ClientSetupError,
};

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

impl From<CoreSessionState> for SessionState {
    fn from(state: CoreSessionState) -> Self {
        match state {
            CoreSessionState::LoggedOut => Self::LoggedOut,
            CoreSessionState::Init => Self::Init,
            CoreSessionState::InitialSync => Self::InitialSync,
            CoreSessionState::Ready => Self::Ready,
        }
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Session)]
    pub struct Session {
        /// The core's session, which owns the client and the sync loop.
        core: OnceCell<CoreSession>,
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
        /// The settings in the global account data for this session.
        #[property(get = Self::global_account_data_owned)]
        global_account_data: OnceCell<GlobalAccountData>,
        /// The image packs available to this session.
        #[property(get = Self::image_packs_owned)]
        image_packs: OnceCell<ImagePacks>,
        /// The notifications API for this session.
        #[property(get)]
        notifications: Notifications,
        /// The ignored users API for this session.
        #[property(get)]
        ignored_users: IgnoredUsers,
        /// What the homeserver has said about who is around.
        #[property(get)]
        presence_list: PresenceList,
        /// The calls of this session.
        #[cfg(not(target_os = "android"))]
        #[property(get = Self::calls_owned)]
        calls: OnceCell<Calls>,
        /// The list of sessions for this session's user.
        #[property(get)]
        user_sessions: UserSessionsList,
        /// Information about security for this session.
        #[property(get)]
        security: SessionSecurity,
        /// The cache for remote data.
        remote_cache: OnceCell<RemoteCache>,
        /// The identity server of this session, once something asked for it.
        identity_server: OnceCell<IdentityServer>,
        /// The task following the core's observables.
        watch_handle: RefCell<Option<AbortHandle>>,
        network_monitor_handler_id: RefCell<Option<glib::SignalHandlerId>>,
        homeserver_reachable_lock: Mutex<()>,
        homeserver_reachable_source: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Session {
        const NAME: &'static str = "Session";
        type Type = super::Session;
        type ParentType = SessionInfo;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Session {
        fn dispose(&self) {
            // Needs to be disconnected or else it may restart the sync
            if let Some(handler_id) = self.network_monitor_handler_id.take() {
                gio::NetworkMonitor::default().disconnect(handler_id);
            }

            if let Some(source) = self.homeserver_reachable_source.take() {
                source.remove();
            }

            if let Some(handle) = self.watch_handle.take() {
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
        /// Set the core's session.
        pub(super) fn set_core(&self, core: CoreSession) {
            self.core
                .set(core)
                .expect("core session should be uninitialized");

            let obj = self.obj();

            self.ignored_users.set_session(Some(obj.clone()));
            self.presence_list.set_session(Some(obj.clone()));
            self.notifications.set_session(Some(obj.clone()));
            self.user_sessions.init(&obj, obj.user_id());

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

        /// The core's session.
        pub(super) fn core(&self) -> &CoreSession {
            self.core.get().expect("core session should be initialized")
        }

        /// The Matrix client for this session.
        pub(super) fn client(&self) -> Client {
            self.core().client()
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
        pub(super) fn set_state(&self, state: SessionState) {
            let old_state = self.state.get();

            if old_state == SessionState::LoggedOut || old_state == state {
                // The session should be dismissed when it has been logged out, so
                // we do not accept anymore state changes.
                return;
            }

            self.state.set(state);

            if state == SessionState::Ready {
                self.init_notifications();

                // Make sure the UnifiedPush endpoint, when there is one, is
                // registered with this session's homeserver. Once per run,
                // at readiness, so a registration a previous run left
                // half-done heals here.
                #[cfg(target_os = "android")]
                {
                    let client = self.client();
                    spawn_tokio!(async move {
                        crate::utils::android_push::ensure_pusher(client).await;
                    });
                }
            }

            self.obj().notify_state();
        }

        /// Set whether the homeserver is reachable, from the core.
        fn set_is_homeserver_reachable(&self, is_reachable: bool) {
            if self.is_homeserver_reachable.get() == is_reachable {
                return;
            }

            self.is_homeserver_reachable.set(is_reachable);
            self.obj().notify_is_homeserver_reachable();
        }

        /// Set whether this session is synchronized with the homeserver, from
        /// the core.
        fn set_is_offline(&self, is_offline: bool) {
            if self.is_offline.get() == is_offline {
                return;
            }

            self.is_offline.set(is_offline);
            self.obj().notify_is_offline();
        }

        /// Set the profile of this session's user, from the core.
        fn set_profile(&self, profile: SessionProfile) {
            let user = self.user();
            user.set_name(profile.display_name);
            user.set_avatar_url(profile.avatar_url);
        }

        /// The homeserver URL as a `GNetworkAddress`.
        fn homeserver_address(&self) -> gio::NetworkAddress {
            let obj = self.obj();
            let homeserver = obj.homeserver();
            let default_port = if homeserver.scheme() == "http" {
                80
            } else {
                443
            };

            gio::NetworkAddress::parse_uri(homeserver.as_str(), default_port)
                .expect("url is parsed successfully")
        }

        /// Check whether the homeserver is reachable.
        pub(super) async fn update_homeserver_reachable(&self) {
            // If there is a timeout, remove it, we will add a new one later if needed.
            if let Some(source) = self.homeserver_reachable_source.take() {
                source.remove();
            }
            let Some(_guard) = self.homeserver_reachable_lock.try_lock() else {
                // There is an ongoing check.
                return;
            };

            let monitor = gio::NetworkMonitor::default();
            let is_network_available = monitor.is_network_available();

            let is_homeserver_reachable = if is_network_available {
                // Check if we can reach the homeserver.
                let address = self.homeserver_address();

                match monitor.can_reach_future(&address).await {
                    Ok(()) => true,
                    Err(error) => {
                        error!(
                            session = self.obj().session_id(),
                            "Homeserver is not reachable: {error}"
                        );
                        false
                    }
                }
            } else {
                false
            };

            // The monitor knows about captive portals, metered links and
            // proxies the core's own dial does not; the core takes the
            // answer and restarts or stops the sync loop.
            self.core()
                .report_homeserver_reachable(is_homeserver_reachable);

            if is_network_available && !is_homeserver_reachable {
                // Check again later if the homeserver is reachable.
                let source = glib::timeout_add_seconds_local_once(
                    10,
                    clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move || {
                            imp.homeserver_reachable_source.take();

                            spawn!(async move {
                                imp.update_homeserver_reachable().await;
                            });
                        }
                    ),
                );
                self.homeserver_reachable_source.replace(Some(source));
            }
        }

        /// Drop any stale offline claim and find out fresh.
        ///
        /// On Android, backgrounding cuts the process off the network and then
        /// freezes it, so the last thing this session learned before the
        /// freeze is usually "the syncs are failing" — and that stale claim is
        /// what an `is-offline` banner would greet the user with on every
        /// return, right up until the first sync lands. Coming back to the
        /// foreground makes the connectivity genuinely unknown, and unknown is
        /// presented as online: the banner's job is to announce known trouble,
        /// not unfinished measurement. If the trouble is real, the fresh
        /// checks this kicks off will say so within a few seconds.
        #[cfg(target_os = "android")]
        pub(super) fn recheck_connectivity(&self) {
            self.core().recheck_connectivity();
        }

        /// The settings stored in the global account data for this session.
        fn global_account_data(&self) -> &GlobalAccountData {
            self.global_account_data
                .get_or_init(|| GlobalAccountData::new(&self.obj()))
        }

        /// The owned settings stored in the global account data for this
        /// session.
        fn global_account_data_owned(&self) -> GlobalAccountData {
            self.global_account_data().clone()
        }

        /// The image packs available to this session.
        fn image_packs(&self) -> &ImagePacks {
            self.image_packs
                .get_or_init(|| ImagePacks::new(&self.obj()))
        }

        /// The owned image packs available to this session.
        fn image_packs_owned(&self) -> ImagePacks {
            self.image_packs().clone()
        }

        /// The calls of this session.
        #[cfg(not(target_os = "android"))]
        pub(super) fn calls(&self) -> &Calls {
            self.calls.get_or_init(|| Calls::new(&self.obj()))
        }

        /// The owned calls of this session.
        #[cfg(not(target_os = "android"))]
        fn calls_owned(&self) -> Calls {
            self.calls().clone()
        }

        /// The cache for remote data.
        pub(super) fn remote_cache(&self) -> &RemoteCache {
            self.remote_cache
                .get_or_init(|| RemoteCache::new(self.obj().clone()))
        }

        /// The identity server of this session.
        pub(super) fn identity_server(&self) -> &IdentityServer {
            self.identity_server.get_or_init(IdentityServer::default)
        }

        /// Finish initialization of this session.
        pub(super) async fn prepare(&self) {
            let core = self.core().clone();

            // The core loads the room list, watches the session's changes,
            // probes the homeserver, attaches verification, security and
            // calls on its side, and starts the one sync loop. Its store and
            // its probe want the runtime.
            spawn_tokio!(async move { core.prepare().await })
                .await
                .expect("task was not aborted");

            self.attach();

            debug!(
                session = self.obj().session_id(),
                "A new session was prepared"
            );
        }

        /// Attach the application's side to the prepared core.
        ///
        /// The desktop's lists, packs, verification, calls and security
        /// hang off the core once it runs, and the state, connectivity and
        /// profile mirrors start here: `watch_core` reads the core's
        /// current state after subscribing, so a core that is already
        /// `Ready` — one the list restored on the runtime — is picked up
        /// at once.
        pub(super) fn attach(&self) {
            self.global_account_data();
            self.image_packs();
            self.room_list().load();
            self.verification_list().init();
            #[cfg(not(target_os = "android"))]
            self.calls().init();
            self.security.set_session(Some(&*self.obj()));

            self.watch_core();
        }

        /// Follow the core's observables into this object's properties.
        fn watch_core(&self) {
            let obj = self.obj();
            let core = self.core();

            let handle = ObjectWatcher::new(&*obj)
                .follow(core.subscribe_state(), |obj: &super::Session, state| {
                    obj.imp().set_state(state.into());
                })
                .follow(
                    core.subscribe_is_offline(),
                    |obj: &super::Session, is_offline| {
                        obj.imp().set_is_offline(is_offline);
                    },
                )
                .follow(
                    core.subscribe_is_homeserver_reachable(),
                    |obj: &super::Session, is_reachable| {
                        obj.imp().set_is_homeserver_reachable(is_reachable);
                    },
                )
                .follow(core.subscribe_profile(), |obj: &super::Session, profile| {
                    obj.imp().set_profile(profile);
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // The values the core already has, after subscribing so that
            // nothing between the two is lost.
            self.set_state(core.state().into());
            self.set_is_offline(core.is_offline());
            self.set_is_homeserver_reachable(core.is_homeserver_reachable());
            self.set_profile(core.profile());
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

        /// Clean up this session after it was logged out.
        ///
        /// This should only be called if the session has been logged out
        /// without calling `Session::log_out`.
        pub(super) async fn clean_up(&self) {
            let obj = self.obj();

            let core = self.core().clone();
            spawn_tokio!(async move { core.clean_up().await })
                .await
                .expect("task was not aborted");

            // Now rather than when the core's change arrives: whoever
            // awaited this expects the state to say so.
            self.set_state(SessionState::LoggedOut);
            self.notifications.clear();

            debug!(
                session = obj.session_id(),
                "The logged out session was cleaned up"
            );
        }
    }
}

glib::wrapper! {
    /// A Matrix user session.
    pub struct Session(ObjectSubclass<imp::Session>)
        @extends SessionInfo;
}

impl Session {
    /// Present the given core session.
    pub(crate) fn from_core(core: CoreSession) -> Self {
        let stored_session = StoredSession::from(core.info().clone());
        let settings = SessionSettings::new(core.settings().clone());

        let obj = glib::Object::builder::<Self>()
            .property("info", stored_session)
            .property("settings", settings)
            .build();
        obj.imp().set_core(core);

        obj
    }

    /// Present the given core session, which the core has already prepared.
    ///
    /// A session the list restored: the core prepared it on the runtime,
    /// so the application's side attaches now instead of in `prepare`.
    pub(crate) fn from_prepared_core(core: CoreSession) -> Self {
        let obj = Self::from_core(core);
        obj.imp().attach();
        obj
    }

    /// Create a new session from the session of the given Matrix client.
    ///
    /// The session is not in the list yet: the login flow prepares it
    /// and the window adds it when the setup is done.
    pub(crate) async fn create(client: &Client) -> Result<Self, ClientSetupError> {
        let client = client.clone();
        let settings = Application::default().session_list().settings();
        let core = spawn_tokio!(async move { CoreSession::create(&client, &settings).await })
            .await
            .expect("task was not aborted")?;

        Ok(Self::from_core(core))
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
        self.imp().client()
    }

    /// The core's session, which this presents.
    pub(crate) fn core(&self) -> &CoreSession {
        self.imp().core()
    }

    /// The cache for remote data.
    pub(crate) fn remote_cache(&self) -> &RemoteCache {
        self.imp().remote_cache()
    }

    /// Drop any stale offline claim and find out fresh.
    ///
    /// For the moment the application comes back to the foreground, when what
    /// this session last learned about its connection predates a background
    /// freeze.
    #[cfg(target_os = "android")]
    pub(crate) fn recheck_connectivity(&self) {
        self.imp().recheck_connectivity();
    }

    /// The identity server of this session.
    pub(crate) fn identity_server(&self) -> &IdentityServer {
        self.imp().identity_server()
    }

    /// Log out of this session.
    pub(crate) async fn log_out(&self) -> Result<(), String> {
        debug!(
            session = self.session_id(),
            "The session is about to be logged out"
        );

        // The pusher belongs to the account, so nothing removes it with the
        // device — and it has to go before the access token that can remove
        // it does. Best-effort: an account that never had one refuses the
        // delete, quietly.
        #[cfg(target_os = "android")]
        {
            let client = self.client();
            let handle =
                spawn_tokio!(
                    async move { crate::utils::android_push::remove_pusher(client).await }
                );
            handle.await.expect("task was not aborted");
        }

        let core = self.core().clone();
        let handle = spawn_tokio!(async move { core.log_out().await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => {
                // The core cleaned itself up; the rest is the application's.
                self.imp().set_state(SessionState::LoggedOut);
                self.notifications().clear();
                Ok(())
            }
            Err(error) => {
                error!(
                    session = self.session_id(),
                    "Could not log the session out: {error}"
                );
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

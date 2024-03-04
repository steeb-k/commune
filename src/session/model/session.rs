use std::time::Duration;

use futures_util::StreamExt;
use gettextrs::gettext;
use gtk::{
    gio, glib,
    glib::{clone, signal::SignalHandlerId},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::{config::SyncSettings, matrix_auth::MatrixSession, sync::SyncResponse, Client};
use ruma::{
    api::client::{
        error::ErrorKind,
        filter::{FilterDefinition, LazyLoadOptions, RoomEventFilter, RoomFilter},
        session::logout,
    },
    assign,
    events::{direct::DirectEventContent, GlobalAccountDataEvent},
};
use tokio::task::JoinHandle;
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
        check_if_reachable,
        matrix::{self, ClientSetupError},
        TokioDrop,
    },
    Application,
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

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "BoxedClient")]
pub struct BoxedClient(Client);

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
        pub sync_tokio_handle: RefCell<Option<JoinHandle<()>>>,
        pub offline_handler_id: RefCell<Option<SignalHandlerId>>,
        /// Whether this session has a connection to the homeserver.
        #[property(get)]
        pub offline: Cell<bool>,
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
            let handler_id = monitor.connect_network_changed(clone!(@weak obj => move |_, _| {
                spawn!(clone!(@weak obj => async move {
                    obj.update_offline().await;
                }));
            }));

            self.offline_handler_id.replace(Some(handler_id));
        }

        fn dispose(&self) {
            // Needs to be disconnected or else it may restart the sync
            if let Some(handler_id) = self.offline_handler_id.take() {
                gio::NetworkMonitor::default().disconnect(handler_id);
            }

            if let Some(handle) = self.sync_tokio_handle.take() {
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
        self.update_user_profile();
        self.update_offline().await;

        self.room_list().load().await;
        self.setup_direct_room_handler();
        self.verification_list().init();

        self.set_state(SessionState::InitialSync);
        self.sync();

        debug!("A new session was prepared");
    }

    /// Start syncing the Matrix client.
    fn sync(&self) {
        if self.state() < SessionState::InitialSync || self.offline() {
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
        });

        self.imp().sync_tokio_handle.replace(Some(handle));
    }

    /// Whether this session is verified with cross-signing.
    pub async fn is_verified(&self) -> bool {
        let client = self.client();
        let e2ee_device_handle = spawn_tokio!(async move {
            let user_id = client.user_id().unwrap();
            let device_id = client.device_id().unwrap();
            client.encryption().get_device(user_id, device_id).await
        });

        match e2ee_device_handle.await.unwrap() {
            Ok(Some(device)) => device.is_verified_with_cross_signing(),
            Ok(None) => {
                error!("Could not find this session’s encryption profile");
                false
            }
            Err(error) => {
                error!("Could not get session’s encryption profile: {error}");
                false
            }
        }
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
                                    obj.notifications().show(notification, room).await;
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

    /// Update the profile of this session’s user.
    ///
    /// Fetches the updated profile and updates the local data.
    pub fn update_user_profile(&self) {
        let client = self.client();
        let user = self.user();

        let handle = spawn_tokio!(async move { client.account().get_profile().await });

        spawn!(glib::Priority::LOW, async move {
            match handle.await.unwrap() {
                Ok(res) => {
                    user.set_name(res.displayname);
                    user.set_avatar_url(res.avatar_url);
                }
                Err(error) => error!("Could not fetch account metadata: {error}"),
            }
        });
    }

    /// The Matrix client.
    pub fn client(&self) -> Client {
        self.imp()
            .client
            .get()
            .expect("The session wasn't prepared")
            .0
            .clone()
    }

    /// Update whether this session is offline.
    async fn update_offline(&self) {
        let imp = self.imp();
        let monitor = gio::NetworkMonitor::default();

        let offline = if monitor.is_network_available() {
            !check_if_reachable(&self.homeserver()).await
        } else {
            true
        };

        if self.offline() == offline {
            return;
        }

        if offline {
            debug!("This session is now offline");
        } else {
            debug!("This session is now online");
        }

        imp.offline.set(offline);

        if let Some(handle) = imp.sync_tokio_handle.take() {
            handle.abort();
        }

        // Restart the sync loop when online
        self.sync();

        self.notify_offline();
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
        match response {
            Ok(response) => {
                self.room_list().handle_room_updates(response.rooms);

                if self.state() < SessionState::Ready {
                    self.set_state(SessionState::Ready);
                }
            }
            Err(error) => {
                if let Some(kind) = error.client_api_error_kind() {
                    if matches!(kind, ErrorKind::UnknownToken { .. }) {
                        self.handle_logged_out();
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
                self.cleanup_session().await;

                Ok(())
            }
            Err(error) => {
                error!("Could not log the session out: {error}");

                Err(gettext("Could not log the session out."))
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
            clone!(@strong self as obj => async move {
                obj.cleanup_session().await;
            })
        );
    }

    /// Clean up this session after it was logged out.
    async fn cleanup_session(&self) {
        let imp = self.imp();

        self.set_state(SessionState::LoggedOut);

        if let Some(handle) = imp.sync_tokio_handle.take() {
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

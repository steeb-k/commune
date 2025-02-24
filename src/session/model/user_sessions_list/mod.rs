use std::cmp;

use futures_util::StreamExt;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use indexmap::{IndexMap, IndexSet};
use matrix_sdk::encryption::identities::UserDevices;
use ruma::OwnedUserId;
use tokio::task::AbortHandle;
use tracing::error;

mod user_session;

pub use self::user_session::UserSession;
use self::user_session::UserSessionData;
use super::Session;
use crate::{prelude::*, spawn, spawn_tokio, utils::LoadingState};

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::UserSessionsList)]
    pub struct UserSessionsList {
        /// The current session.
        #[property(get)]
        session: glib::WeakRef<Session>,
        /// The ID of the user the sessions belong to.
        user_id: OnceCell<OwnedUserId>,
        /// The other user sessions.
        #[property(get)]
        other_sessions: gio::ListStore,
        /// The current user session.
        #[property(get)]
        current_session: RefCell<Option<UserSession>>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// Whether the list is empty.
        #[property(get = Self::is_empty)]
        is_empty: PhantomData<bool>,
        sessions_watch_abort_handle: RefCell<Option<AbortHandle>>,
    }

    impl Default for UserSessionsList {
        fn default() -> Self {
            Self {
                session: Default::default(),
                user_id: Default::default(),
                other_sessions: gio::ListStore::new::<UserSession>(),
                current_session: Default::default(),
                loading_state: Default::default(),
                is_empty: Default::default(),
                sessions_watch_abort_handle: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserSessionsList {
        const NAME: &'static str = "UserSessionsList";
        type Type = super::UserSessionsList;
    }

    #[glib::derived_properties]
    impl ObjectImpl for UserSessionsList {
        fn dispose(&self) {
            if let Some(abort_handle) = self.sessions_watch_abort_handle.take() {
                abort_handle.abort();
            }
        }
    }

    impl UserSessionsList {
        /// Initialize this list with the given session and user ID.
        pub(super) fn init(&self, session: &Session, user_id: OwnedUserId) {
            self.session.set(Some(session));
            let user_id = self.user_id.get_or_init(|| user_id);

            // We know that we have at least this session for our own user.
            if session.user_id() == user_id {
                let current_session = UserSession::new(
                    session,
                    UserSessionData::DeviceId(session.device_id().clone()),
                );
                self.set_current_session(Some(current_session));
            }

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.watch_sessions().await;
                }
            ));
        }

        /// The ID of the user the sessions belong to.
        fn user_id(&self) -> &OwnedUserId {
            self.user_id.get().expect("user ID is initialized")
        }

        /// Listen to changes in the user sessions.
        async fn watch_sessions(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let client = session.client();
            let stream = match client.encryption().devices_stream().await {
                Ok(stream) => stream,
                Err(error) => {
                    error!("Could not access the user sessions stream: {error}");
                    return;
                }
            };

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let user_id = self.user_id().clone();
            let fut = stream.for_each(move |updates| {
                let user_id = user_id.clone();
                let obj_weak = obj_weak.clone();

                async move {
                    // If a device update is received for an account different than the one
                    // for which the settings are currently opened, we don't want to reload the user
                    // sessions, to save bandwidth.
                    // However, when a device is disconnected, an empty device update is received.
                    // In this case, we do not know which account had a device disconnection, so we
                    // want to reload the sessions just in case.
                    if !updates.new.contains_key(&user_id)
                        && !updates.changed.contains_key(&user_id)
                        && (!updates.new.is_empty() || !updates.changed.is_empty())
                    {
                        return;
                    }

                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.load().await;
                            }
                        });
                    });
                }
            });

            let abort_handle = spawn_tokio!(fut).abort_handle();
            self.sessions_watch_abort_handle.replace(Some(abort_handle));
        }

        /// Load the list of user sessions.
        pub(super) async fn load(&self) {
            if self.loading_state.get() == LoadingState::Loading {
                // Do not load the list twice at the same time.
                return;
            }

            let Some(session) = self.session.upgrade() else {
                return;
            };

            self.set_loading_state(LoadingState::Loading);

            let user_id = self.user_id().clone();
            let client = session.client();
            let handle = spawn_tokio!(async move {
                let crypto_sessions = match client.encryption().get_user_devices(&user_id).await {
                    Ok(crypto_sessions) => Some(crypto_sessions),
                    Err(error) => {
                        error!("Could not get crypto sessions for user {user_id}: {error}");
                        None
                    }
                };

                let is_own_user = client.user_id().unwrap() == user_id;

                let mut api_sessions = None;
                if is_own_user {
                    match client.devices().await {
                        Ok(response) => {
                            api_sessions = Some(response.devices);
                        }
                        Err(error) => {
                            error!("Could not get sessions list for user {user_id}: {error}");
                        }
                    }
                }

                (api_sessions, crypto_sessions)
            });

            let (api_sessions, crypto_sessions) = handle.await.unwrap();

            if api_sessions.is_none() && crypto_sessions.is_none() {
                self.set_loading_state(LoadingState::Error);
                return;
            };

            // Convert API sessions to a map.
            let mut api_sessions = api_sessions
                .into_iter()
                .flatten()
                .map(|d| (d.device_id.clone(), d))
                .collect::<IndexMap<_, _>>();

            // Sort the API sessions, last seen first, then sort by device ID.
            api_sessions.sort_by(|_key_a, val_a, _key_b, val_b| {
                match val_b.last_seen_ts.cmp(&val_a.last_seen_ts) {
                    cmp::Ordering::Equal => val_a.device_id.cmp(&val_b.device_id),
                    cmp => cmp,
                }
            });

            // Build the full list of IDs while preserving the sorting order.
            let ids = api_sessions
                .keys()
                .cloned()
                .chain(
                    crypto_sessions
                        .iter()
                        .flat_map(UserDevices::keys)
                        .map(ToOwned::to_owned),
                )
                .collect::<IndexSet<_>>();

            let (current, others) = ids
                .into_iter()
                .filter_map(|id| {
                    let data = match (
                        api_sessions.shift_remove(&id),
                        crypto_sessions.as_ref().and_then(|s| s.get(&id)),
                    ) {
                        (Some(api), Some(crypto)) => UserSessionData::Both { api, crypto },
                        (Some(api), None) => UserSessionData::DevicesApi(api),
                        (None, Some(crypto)) => UserSessionData::Crypto(crypto),
                        _ => return None,
                    };

                    Some(UserSession::new(&session, data))
                })
                .partition::<Vec<_>, _>(UserSession::is_current);

            if let Some(current) = current.into_iter().next() {
                self.set_current_session(Some(current));
            }

            let was_empty = self.is_empty();

            let removed = self.other_sessions.n_items();
            self.other_sessions.splice(0, removed, &others);

            if self.is_empty() != was_empty {
                self.obj().notify_is_empty();
            }

            self.set_loading_state(LoadingState::Ready);
        }

        /// Set the current user session.
        fn set_current_session(&self, user_session: Option<UserSession>) {
            if *self.current_session.borrow() == user_session {
                return;
            }

            let was_empty = self.is_empty();

            self.current_session.replace(user_session);

            let obj = self.obj();
            obj.notify_current_session();

            if self.is_empty() != was_empty {
                obj.notify_is_empty();
            }
        }

        /// Set the loading state of the list.
        fn set_loading_state(&self, loading_state: LoadingState) {
            if self.loading_state.get() == loading_state {
                return;
            }

            self.loading_state.set(loading_state);
            self.obj().notify_loading_state();
        }

        /// Whether the list is empty.
        fn is_empty(&self) -> bool {
            self.current_session.borrow().is_none() && self.other_sessions.n_items() == 0
        }
    }
}

glib::wrapper! {
    /// List of active user sessions for a user.
    pub struct UserSessionsList(ObjectSubclass<imp::UserSessionsList>);
}

impl UserSessionsList {
    /// Construct a new empty `UserSessionsList`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Initialize this list with the given session and user ID.
    pub fn init(&self, session: &Session, user_id: OwnedUserId) {
        self.imp().init(session, user_id);
    }

    /// Load the list of user sessions.
    pub async fn load(&self) {
        self.imp().load().await;
    }
}

impl Default for UserSessionsList {
    fn default() -> Self {
        Self::new()
    }
}

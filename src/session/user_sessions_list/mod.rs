use commune_core::session::Device;
use gtk::{glib, prelude::*, subclass::prelude::*};
use ruma::{OwnedDeviceId, UserId};
use tokio::task::AbortHandle;
use tracing::warn;

mod other_sessions_list;
mod user_session;

pub use self::{other_sessions_list::OtherSessionsList, user_session::UserSession};
use super::Session;
use crate::{core_bridge::ObjectWatcher, prelude::*, spawn_tokio, utils::LoadingState};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::UserSessionsList)]
    pub struct UserSessionsList {
        /// The current session.
        #[property(get)]
        session: glib::WeakRef<Session>,
        /// The other user sessions.
        #[property(get)]
        other_sessions: OtherSessionsList,
        /// The current user session.
        #[property(get)]
        current_session: RefCell<Option<UserSession>>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// Whether the list is empty.
        #[property(get = Self::is_empty)]
        is_empty: PhantomData<bool>,
        /// The task following the core's list.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserSessionsList {
        const NAME: &'static str = "UserSessionsList";
        type Type = super::UserSessionsList;
    }

    #[glib::derived_properties]
    impl ObjectImpl for UserSessionsList {
        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl UserSessionsList {
        /// Initialize this list with the given session and user ID, and
        /// follow the core's list.
        ///
        /// The core keeps the sessions of the account itself; a list for
        /// another user has nothing to follow.
        pub(super) fn init(&self, session: &Session, user_id: &UserId) {
            type L = super::UserSessionsList;

            self.session.set(Some(session));

            if **session.user_id() != *user_id {
                warn!("The sessions of another user are not listed");
                return;
            }

            // We know that we have at least this session for our own user.
            let current_session = UserSession::new(session, session.device_id().clone());
            self.current_session.replace(Some(current_session));

            let core = session.core().user_sessions().clone();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe(), |obj: &L, devices| {
                    obj.imp().set_devices(devices);
                })
                .follow(core.subscribe_state(), |obj: &L, state| {
                    obj.imp().set_loading_state(state.into());
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.set_devices(core.snapshot());
            self.set_loading_state(core.state().into());

            // The application has always read the list when the session is
            // set up; the core reads it on first use, which is now.
            spawn_tokio!(async move { core.ensure_loaded().await });
        }

        /// Mirror the core's devices: the current one onto its object, the
        /// rest onto the list of other sessions.
        fn set_devices(&self, devices: Vec<Device>) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let was_empty = self.is_empty();

            let (current, others): (Vec<_>, Vec<_>) =
                devices.into_iter().partition(|device| device.is_current);

            if let Some((current_session, device)) = self
                .current_session
                .borrow()
                .clone()
                .zip(current.into_iter().next())
            {
                current_session.set_device(&device);
            }

            self.other_sessions.update(&session, others);

            if self.is_empty() != was_empty {
                self.obj().notify_is_empty();
            }
        }

        /// Find the user session with the given device ID, if any.
        pub(super) fn get(&self, device_id: &OwnedDeviceId) -> Option<UserSession> {
            if let Some(current_session) = self.current_session.borrow().as_ref()
                && current_session.device_id() == device_id
            {
                return Some(current_session.clone());
            }

            self.other_sessions.get(device_id)
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
    ///
    /// The list itself is the core's; this presents it.
    pub struct UserSessionsList(ObjectSubclass<imp::UserSessionsList>);
}

impl UserSessionsList {
    /// Construct a new empty `UserSessionsList`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Initialize this list with the given session and user ID.
    pub(crate) fn init(&self, session: &Session, user_id: &UserId) {
        self.imp().init(session, user_id);
    }

    /// Load the list of user sessions again.
    pub(crate) async fn load(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let core = session.core().user_sessions().clone();
        spawn_tokio!(async move { core.load().await })
            .await
            .expect("task was not aborted");
    }

    /// Find the user session with the given device ID, if any.
    pub(crate) fn get(&self, device_id: &OwnedDeviceId) -> Option<UserSession> {
        self.imp().get(device_id)
    }
}

impl Default for UserSessionsList {
    fn default() -> Self {
        Self::new()
    }
}

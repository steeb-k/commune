use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use indexmap::map::IndexMap;
use tracing::{error, info, warn};

mod failed_session;
mod new_session;
mod session_info;

pub use self::{failed_session::*, new_session::*, session_info::*};
use crate::{
    prelude::*,
    secret::{self, StoredSession},
    session::model::{Session, SessionState},
    spawn, spawn_tokio,
    utils::LoadingState,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default)]
    pub struct SessionList {
        /// The loading state of the list.
        pub state: Cell<LoadingState>,
        /// The error message, if state is set to `LoadingState::Error`.
        pub error: RefCell<Option<String>>,
        /// A map of session ID to session.
        pub list: RefCell<IndexMap<String, SessionInfo>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionList {
        const NAME: &'static str = "SessionList";
        type Type = super::SessionList;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for SessionList {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecEnum::builder::<LoadingState>("state")
                        .read_only()
                        .build(),
                    glib::ParamSpecString::builder("error").read_only().build(),
                    glib::ParamSpecBoolean::builder("is-empty")
                        .read_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "state" => obj.state().to_value(),
                "error" => obj.error().to_value(),
                "is-empty" => obj.is_empty().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl ListModelImpl for SessionList {
        fn item_type(&self) -> glib::Type {
            SessionInfo::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get_index(position as usize)
                .map(|(_, v)| v.upcast_ref::<glib::Object>())
                .cloned()
        }
    }
}

glib::wrapper! {
    /// List of all logged in sessions.
    pub struct SessionList(ObjectSubclass<imp::SessionList>)
        @implements gio::ListModel;
}

impl SessionList {
    /// Create a new empty `SessionList`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The loading state of this list.
    pub fn state(&self) -> LoadingState {
        self.imp().state.get()
    }

    /// Set the loading state of this list.
    fn set_state(&self, state: LoadingState) {
        if self.state() == state {
            return;
        }

        self.imp().state.set(state);
        self.notify("state");
    }

    /// The error message, if `state` is set to `LoadingState::Error`.
    pub fn error(&self) -> Option<String> {
        self.imp().error.borrow().clone()
    }

    /// Set the error message.
    fn set_error(&self, message: String) {
        self.imp().error.replace(Some(message));
        self.notify("error");
    }

    /// Whether this list is empty.
    pub fn is_empty(&self) -> bool {
        self.imp().list.borrow().is_empty()
    }

    /// Whether any of the sessions are new.
    pub fn has_new_sessions(&self) -> bool {
        self.imp()
            .list
            .borrow()
            .values()
            .any(|s| s.is::<NewSession>())
    }

    /// The session with the given ID, if any.
    pub fn get(&self, session_id: &str) -> Option<SessionInfo> {
        self.imp().list.borrow().get(session_id).cloned()
    }

    /// The index of the session with the given ID, if any.
    pub fn index(&self, session_id: &str) -> Option<usize> {
        self.imp().list.borrow().get_index_of(session_id)
    }

    /// Insert the given session to the list.
    ///
    /// If a session with the same ID already exists, it is replaced.
    ///
    /// Returns the index of the session.
    pub fn insert(&self, session: impl IsA<SessionInfo>) -> usize {
        let session = session.upcast();

        if let Some(session) = session.downcast_ref::<Session>() {
            // Start listening to notifications when the session is ready.
            if session.state() == SessionState::Ready {
                spawn!(clone!(@weak session => async move {
                    session.init_notifications().await
                }));
            } else {
                session.connect_ready(|session| {
                    spawn!(clone!(@weak session => async move {
                        session.init_notifications().await
                    }));
                });
            }

            session.connect_logged_out(clone!(@weak self as obj => move |session| {
                obj.remove(session.session_id())
            }));
        }

        let was_empty = self.is_empty();

        let (index, replaced) = self
            .imp()
            .list
            .borrow_mut()
            .insert_full(session.session_id().to_owned(), session);

        let removed = if replaced.is_some() { 1 } else { 0 };

        self.items_changed(index as u32, removed, 1);

        if was_empty {
            self.notify("is-empty");
        }

        index
    }

    /// Remove the session with the given ID from the list.
    pub fn remove(&self, session_id: &str) {
        let removed = self.imp().list.borrow_mut().shift_remove_full(session_id);

        if let Some((position, ..)) = removed {
            self.items_changed(position as u32, 1, 0);

            if self.is_empty() {
                self.notify("is-empty");
            }
        }
    }

    pub fn connect_is_empty_notify<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_notify_local(Some("is-empty"), move |obj, _| {
            f(obj);
        })
    }

    /// Restore the logged-in sessions.
    pub async fn restore_sessions(&self) {
        if self.state() >= LoadingState::Loading {
            return;
        }

        self.set_state(LoadingState::Loading);

        let handle = spawn_tokio!(secret::restore_sessions());
        match handle.await.unwrap() {
            Ok(sessions) => {
                for stored_session in sessions {
                    info!(
                        "Restoring previous session for user: {}",
                        stored_session.user_id
                    );
                    if let Some(path) = stored_session.path.to_str() {
                        info!("Database path: {path}");
                    }
                    self.insert(NewSession::new(stored_session.clone()));

                    spawn!(
                        glib::Priority::DEFAULT_IDLE,
                        clone!(@weak self as obj => async move {
                            obj.restore_stored_session(stored_session).await;
                        })
                    );
                }

                self.set_state(LoadingState::Ready)
            }
            Err(error) => {
                error!("Failed to restore previous sessions: {error}");

                let message = format!(
                    "{}\n\n{}",
                    gettext("Failed to restore previous sessions"),
                    error.to_user_facing(),
                );

                self.set_error(message);
                self.set_state(LoadingState::Error);
            }
        }
    }

    /// Restore a stored session.
    async fn restore_stored_session(&self, session_info: StoredSession) {
        match Session::restore(session_info.clone()).await {
            Ok(session) => {
                session.prepare().await;
                self.insert(session);
            }
            Err(error) => {
                warn!("Failed to restore previous session: {error}");
                self.insert(FailedSession::new(session_info, error));
            }
        }
    }
}

impl Default for SessionList {
    fn default() -> Self {
        Self::new()
    }
}

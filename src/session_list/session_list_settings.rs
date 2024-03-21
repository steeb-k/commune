use gtk::{glib, prelude::*, subclass::prelude::*};
use indexmap::{IndexMap, IndexSet};
use tracing::error;

use crate::{
    session::model::{SessionSettings, StoredSessionSettings},
    Application,
};

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct SessionListSettings {
        /// The settings of the sessions.
        pub sessions: RefCell<IndexMap<String, SessionSettings>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionListSettings {
        const NAME: &'static str = "SessionListSettings";
        type Type = super::SessionListSettings;
    }

    impl ObjectImpl for SessionListSettings {}
}

glib::wrapper! {
    /// The settings of the list of sessions.
    pub struct SessionListSettings(ObjectSubclass<imp::SessionListSettings>);
}

impl SessionListSettings {
    /// Create a new `SessionListSettings`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Load these settings from the GSettings.
    pub fn load(&self) {
        let serialized = Application::default().settings().string("sessions");

        let stored_sessions =
            match serde_json::from_str::<Vec<(String, StoredSessionSettings)>>(&serialized) {
                Ok(stored_sessions) => stored_sessions,
                Err(error) => {
                    error!(
                        "Could not load sessions settings, fallback to default settings: {error}"
                    );
                    Default::default()
                }
            };

        let sessions = stored_sessions
            .into_iter()
            .map(|(session_id, stored_session)| {
                let session = SessionSettings::restore(&session_id, stored_session);
                (session_id, session)
            })
            .collect();

        self.imp().sessions.replace(sessions);
    }

    /// Save the settings in the GSettings.
    pub fn save(&self) {
        let stored_sessions = self
            .imp()
            .sessions
            .borrow()
            .iter()
            .map(|(session_id, session)| (session_id.clone(), session.stored_settings()))
            .collect::<Vec<_>>();

        if let Err(error) = Application::default().settings().set_string(
            "sessions",
            &serde_json::to_string(&stored_sessions).unwrap(),
        ) {
            error!("Could not save sessions settings: {error}");
        }
    }

    /// Get or create the settings for the session with the given ID.
    pub fn get_or_create(&self, session_id: &str) -> SessionSettings {
        let sessions = &self.imp().sessions;

        if let Some(session) = sessions.borrow().get(session_id) {
            return session.clone();
        };

        let session = SessionSettings::new(session_id);
        sessions
            .borrow_mut()
            .insert(session_id.to_owned(), session.clone());
        self.save();

        session
    }

    /// Remove the settings of the session with the given ID.
    pub fn remove(&self, session_id: &str) {
        self.imp().sessions.borrow_mut().shift_remove(session_id);
        self.save();
    }

    /// Get the list of session IDs stored in these settings.
    pub fn session_ids(&self) -> IndexSet<String> {
        self.imp().sessions.borrow().keys().cloned().collect()
    }
}

impl Default for SessionListSettings {
    fn default() -> Self {
        Self::new()
    }
}

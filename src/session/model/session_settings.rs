use gtk::{glib, prelude::*, subclass::prelude::*};
use serde::{Deserialize, Serialize};

use crate::Application;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSessionSettings {
    /// Custom servers to explore.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    explore_custom_servers: Vec<String>,

    /// Whether notifications are enabled for this session.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    notifications_enabled: bool,
}

impl Default for StoredSessionSettings {
    fn default() -> Self {
        Self {
            explore_custom_servers: Default::default(),
            notifications_enabled: true,
        }
    }
}

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "BoxedStoredSessionSettings")]
pub struct BoxedStoredSessionSettings(StoredSessionSettings);

mod imp {
    use std::cell::RefCell;

    use once_cell::sync::{Lazy, OnceCell};

    use super::*;

    #[derive(Debug, Default)]
    pub struct SessionSettings {
        /// The ID of the session these settings are for.
        pub session_id: OnceCell<String>,
        /// The stored settings.
        pub stored_settings: RefCell<StoredSessionSettings>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionSettings {
        const NAME: &'static str = "SessionSettings";
        type Type = super::SessionSettings;
    }

    impl ObjectImpl for SessionSettings {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecString::builder("session-id")
                        .construct_only()
                        .build(),
                    glib::ParamSpecBoxed::builder::<BoxedStoredSessionSettings>("stored-settings")
                        .write_only()
                        .construct_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("notifications-enabled")
                        .default_value(true)
                        .explicit_notify()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "session-id" => self.session_id.set(value.get().unwrap()).unwrap(),
                "stored-settings" => {
                    self.stored_settings
                        .replace(value.get::<BoxedStoredSessionSettings>().unwrap().0);
                }
                "notifications-enabled" => obj.set_notifications_enabled(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "session-id" => obj.session_id().to_value(),
                "notifications-enabled" => obj.notifications_enabled().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    /// The settings of a `Session`.
    pub struct SessionSettings(ObjectSubclass<imp::SessionSettings>);
}

impl SessionSettings {
    /// Create a new `SessionSettings` for the given session ID.
    pub fn new(session_id: &str) -> Self {
        glib::Object::builder()
            .property("session-id", session_id)
            .property(
                "stored-settings",
                &BoxedStoredSessionSettings(StoredSessionSettings::default()),
            )
            .build()
    }

    /// Restore existing `SessionSettings` with the given session ID and stored
    /// settings.
    pub fn restore(session_id: &str, stored_settings: StoredSessionSettings) -> Self {
        glib::Object::builder()
            .property("session-id", session_id)
            .property(
                "stored-settings",
                &BoxedStoredSessionSettings(stored_settings),
            )
            .build()
    }

    /// The stored settings.
    pub fn stored_settings(&self) -> StoredSessionSettings {
        self.imp().stored_settings.borrow().clone()
    }

    /// Save the settings in the GSettings.
    fn save(&self) {
        Application::default().session_list().settings().save();
    }

    /// Delete the settings from the GSettings.
    pub fn delete(&self) {
        Application::default()
            .session_list()
            .settings()
            .remove(self.session_id());
    }

    /// The ID of the session these settings are for.
    pub fn session_id(&self) -> &str {
        self.imp().session_id.get().unwrap()
    }

    pub fn explore_custom_servers(&self) -> Vec<String> {
        self.imp()
            .stored_settings
            .borrow()
            .explore_custom_servers
            .clone()
    }

    pub fn set_explore_custom_servers(&self, servers: Vec<String>) {
        if self.explore_custom_servers() == servers {
            return;
        }

        self.imp()
            .stored_settings
            .borrow_mut()
            .explore_custom_servers = servers;
        self.save();
    }

    /// Whether notifications are enabled for this session.
    pub fn notifications_enabled(&self) -> bool {
        self.imp().stored_settings.borrow().notifications_enabled
    }

    /// Set whether notifications are enabled for this session.
    pub fn set_notifications_enabled(&self, enabled: bool) {
        if self.notifications_enabled() == enabled {
            return;
        }

        self.imp()
            .stored_settings
            .borrow_mut()
            .notifications_enabled = enabled;
        self.save();
        self.notify("notifications-enabled");
    }
}

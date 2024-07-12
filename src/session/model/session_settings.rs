use gtk::{glib, prelude::*, subclass::prelude::*};
use serde::{Deserialize, Serialize};

use super::CategoryType;
use crate::Application;

#[derive(Debug, Clone, Serialize, Deserialize, glib::Boxed)]
#[boxed_type(name = "StoredSessionSettings")]
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

    /// Whether public read receipts are enabled for this session.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    public_read_receipts_enabled: bool,

    /// Whether typing notifications are enabled for this session.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    typing_enabled: bool,

    /// Which categories are expanded.
    #[serde(default)]
    categories_expanded: CategoriesExpanded,
}

impl Default for StoredSessionSettings {
    fn default() -> Self {
        Self {
            explore_custom_servers: Default::default(),
            notifications_enabled: true,
            public_read_receipts_enabled: true,
            typing_enabled: true,
            categories_expanded: Default::default(),
        }
    }
}

mod imp {
    use std::{
        cell::{OnceCell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SessionSettings)]
    pub struct SessionSettings {
        /// The ID of the session these settings are for.
        #[property(get, construct_only)]
        pub session_id: OnceCell<String>,
        /// The stored settings.
        #[property(get, construct_only)]
        pub stored_settings: RefCell<StoredSessionSettings>,
        /// Whether notifications are enabled for this session.
        #[property(get = Self::notifications_enabled, set = Self::set_notifications_enabled, explicit_notify, default = true)]
        pub notifications_enabled: PhantomData<bool>,
        /// Whether public read receipts are enabled for this session.
        #[property(get = Self::public_read_receipts_enabled, set = Self::set_public_read_receipts_enabled, explicit_notify, default = true)]
        pub public_read_receipts_enabled: PhantomData<bool>,
        /// Whether typing notifications are enabled for this session.
        #[property(get = Self::typing_enabled, set = Self::set_typing_enabled, explicit_notify, default = true)]
        pub typing_enabled: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionSettings {
        const NAME: &'static str = "SessionSettings";
        type Type = super::SessionSettings;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SessionSettings {}

    impl SessionSettings {
        /// Whether notifications are enabled for this session.
        fn notifications_enabled(&self) -> bool {
            self.stored_settings.borrow().notifications_enabled
        }

        /// Set whether notifications are enabled for this session.
        fn set_notifications_enabled(&self, enabled: bool) {
            if self.notifications_enabled() == enabled {
                return;
            }
            let obj = self.obj();

            self.stored_settings.borrow_mut().notifications_enabled = enabled;
            obj.save();
            obj.notify_notifications_enabled();
        }

        /// Whether public read receipts are enabled for this session.
        fn public_read_receipts_enabled(&self) -> bool {
            self.stored_settings.borrow().public_read_receipts_enabled
        }

        /// Set whether public read receipts are enabled for this session.
        fn set_public_read_receipts_enabled(&self, enabled: bool) {
            if self.public_read_receipts_enabled() == enabled {
                return;
            }
            let obj = self.obj();

            self.stored_settings
                .borrow_mut()
                .public_read_receipts_enabled = enabled;
            obj.save();
            obj.notify_public_read_receipts_enabled();
        }

        /// Whether typing notifications are enabled for this session.
        fn typing_enabled(&self) -> bool {
            self.stored_settings.borrow().typing_enabled
        }

        /// Set whether typing notifications are enabled for this session.
        fn set_typing_enabled(&self, enabled: bool) {
            if self.typing_enabled() == enabled {
                return;
            }
            let obj = self.obj();

            self.stored_settings.borrow_mut().typing_enabled = enabled;
            obj.save();
            obj.notify_typing_enabled();
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
            .property("stored-settings", &StoredSessionSettings::default())
            .build()
    }

    /// Restore existing `SessionSettings` with the given session ID and stored
    /// settings.
    pub fn restore(session_id: &str, stored_settings: StoredSessionSettings) -> Self {
        glib::Object::builder()
            .property("session-id", session_id)
            .property("stored-settings", &stored_settings)
            .build()
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
            .remove(&self.session_id());
    }

    /// Custom servers to explore.
    pub fn explore_custom_servers(&self) -> Vec<String> {
        self.imp()
            .stored_settings
            .borrow()
            .explore_custom_servers
            .clone()
    }

    /// Set the custom servers to explore.
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

    /// Whether the given category is expanded.
    pub fn is_category_expanded(&self, category: CategoryType) -> bool {
        self.imp()
            .stored_settings
            .borrow()
            .categories_expanded
            .is_category_expanded(category)
    }

    /// Set whether the given category is expanded.
    pub fn set_category_expanded(&self, category: CategoryType, expanded: bool) {
        self.imp()
            .stored_settings
            .borrow_mut()
            .categories_expanded
            .set_category_expanded(category, expanded);
        self.save();
    }
}

/// Whether the categories are expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct CategoriesExpanded {
    verification_request: bool,
    invited: bool,
    favorite: bool,
    normal: bool,
    low_priority: bool,
    left: bool,
}

impl CategoriesExpanded {
    /// Whether the given category is expanded.
    pub fn is_category_expanded(&self, category: CategoryType) -> bool {
        match category {
            CategoryType::VerificationRequest => self.verification_request,
            CategoryType::Invited => self.invited,
            CategoryType::Favorite => self.favorite,
            CategoryType::Normal => self.normal,
            CategoryType::LowPriority => self.low_priority,
            CategoryType::Left => self.left,
            _ => false,
        }
    }

    /// Set whether the given category is expanded.
    pub fn set_category_expanded(&mut self, category: CategoryType, expanded: bool) {
        let field = match category {
            CategoryType::VerificationRequest => &mut self.verification_request,
            CategoryType::Invited => &mut self.invited,
            CategoryType::Favorite => &mut self.favorite,
            CategoryType::Normal => &mut self.normal,
            CategoryType::LowPriority => &mut self.low_priority,
            CategoryType::Left => &mut self.left,
            _ => return,
        };

        *field = expanded;
    }
}

impl Default for CategoriesExpanded {
    fn default() -> Self {
        Self {
            verification_request: true,
            invited: true,
            favorite: true,
            normal: true,
            low_priority: true,
            left: false,
        }
    }
}

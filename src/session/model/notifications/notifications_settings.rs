use futures_util::StreamExt;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    notification_settings::{
        IsEncrypted, NotificationSettings as MatrixNotificationSettings, RoomNotificationMode,
    },
    NotificationSettingsError,
};
use ruma::push::{PredefinedOverrideRuleId, RuleKind};
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, warn};

use crate::{
    session::model::{Session, SessionState},
    spawn, spawn_tokio,
};

/// The possible values for the global notifications setting.
#[derive(
    Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum, strum::Display, strum::EnumString,
)]
#[repr(u32)]
#[enum_type(name = "NotificationsGlobalSetting")]
#[strum(serialize_all = "kebab-case")]
pub enum NotificationsGlobalSetting {
    /// Every message in every room.
    #[default]
    All = 0,
    /// Every message in 1-to-1 rooms, and mentions and keywords in every room.
    DirectAndMentions = 1,
    /// Only mentions and keywords in every room.
    MentionsOnly = 2,
}

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::NotificationsSettings)]
    pub struct NotificationsSettings {
        /// The parent `Session`.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        pub session: glib::WeakRef<Session>,
        /// The SDK notification settings API.
        pub api: RefCell<Option<MatrixNotificationSettings>>,
        /// Whether notifications are enabled for this Matrix account.
        #[property(get)]
        pub account_enabled: Cell<bool>,
        /// Whether notifications are enabled for this session.
        #[property(get, set = Self::set_session_enabled, explicit_notify)]
        pub session_enabled: Cell<bool>,
        /// The global setting about which messages trigger notifications.
        #[property(get, builder(NotificationsGlobalSetting::default()))]
        pub global_setting: Cell<NotificationsGlobalSetting>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NotificationsSettings {
        const NAME: &'static str = "NotificationsSettings";
        type Type = super::NotificationsSettings;
    }

    #[glib::derived_properties]
    impl ObjectImpl for NotificationsSettings {}

    impl NotificationsSettings {
        /// Set the parent `Session`.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            let obj = self.obj();

            if let Some(session) = session {
                session
                    .settings()
                    .bind_property("notifications-enabled", &*obj, "session-enabled")
                    .sync_create()
                    .bidirectional()
                    .build();
            }

            self.session.set(session);
            obj.notify_session();

            spawn!(clone!(@weak obj => async move {
                obj.init_api().await;
            }));
        }

        /// Set whether notifications are enabled for this session.
        fn set_session_enabled(&self, enabled: bool) {
            if self.session_enabled.get() == enabled {
                return;
            }

            if !enabled {
                if let Some(session) = self.session.upgrade() {
                    session.notifications().clear();
                }
            }

            self.session_enabled.set(enabled);
            self.obj().notify_session_enabled();
        }
    }
}

glib::wrapper! {
    /// The notifications settings of a `Session`.
    pub struct NotificationsSettings(ObjectSubclass<imp::NotificationsSettings>);
}

impl NotificationsSettings {
    /// Create a new `NotificationsSettings`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The SDK notification settings API.
    fn api(&self) -> Option<MatrixNotificationSettings> {
        self.imp().api.borrow().clone()
    }

    /// Initialize the SDK notification settings API.
    async fn init_api(&self) {
        let Some(session) = self.session() else {
            self.imp().api.take();
            return;
        };

        // If the session is not ready, there is no client so let's wait to initialize
        // the API.
        if session.state() != SessionState::Ready {
            self.imp().api.take();

            session.connect_ready(clone!(@weak self as obj => move |_| {
                spawn!(clone!(@weak obj => async move {
                    obj.init_api().await;
                }));
            }));

            return;
        }

        let client = session.client();
        let api = spawn_tokio!(async move { client.notification_settings().await })
            .await
            .unwrap();
        let mut api_receiver = api.subscribe_to_changes();

        self.imp().api.replace(Some(api.clone()));

        let (mut sender, mut receiver) = futures_channel::mpsc::channel(10);
        spawn_tokio!(async move {
            loop {
                match api_receiver.recv().await {
                    Ok(()) => {
                        if let Err(error) = sender.try_send(()) {
                            error!("Error sending notifications settings change: {error}");
                            panic!();
                        }
                    }
                    Err(RecvError::Closed) => {
                        break;
                    }
                    Err(RecvError::Lagged(_)) => {
                        warn!("Some notifications settings changes were dropped");
                    }
                }
            }
        });

        spawn!(clone!(@weak self as obj => async move { obj.update().await; }));

        while let Some(()) = receiver.next().await {
            spawn!(clone!(@weak self as obj => async move { obj.update().await; }));
        }
    }

    /// Update the notification settings from the SDK API.
    async fn update(&self) {
        let Some(api) = self.api() else {
            return;
        };

        let api_clone = api.clone();
        let handle = spawn_tokio!(async move {
            api_clone
                .is_push_rule_enabled(RuleKind::Override, PredefinedOverrideRuleId::Master)
                .await
        });

        let account_enabled = match handle.await.unwrap() {
            // The rule disables notifications, so we need to invert the boolean.
            Ok(enabled) => !enabled,
            Err(error) => {
                error!("Failed to get account notifications setting: {error}");
                true
            }
        };
        self.set_account_enabled_inner(account_enabled);

        if default_rooms_notifications_is_all(api.clone(), false).await {
            self.set_global_setting_inner(NotificationsGlobalSetting::All);
        } else if default_rooms_notifications_is_all(api.clone(), true).await {
            self.set_global_setting_inner(NotificationsGlobalSetting::DirectAndMentions);
        } else {
            self.set_global_setting_inner(NotificationsGlobalSetting::MentionsOnly);
        }
    }

    /// Set whether notifications are enabled for this session.
    pub async fn set_account_enabled(
        &self,
        enabled: bool,
    ) -> Result<(), NotificationSettingsError> {
        let Some(api) = self.api() else {
            error!("Cannot update notifications settings when API is not initialized");
            return Err(NotificationSettingsError::UnableToUpdatePushRule);
        };

        let handle = spawn_tokio!(async move {
            api.set_push_rule_enabled(
                RuleKind::Override,
                PredefinedOverrideRuleId::Master,
                // The rule disables notifications, so we need to invert the boolean.
                !enabled,
            )
            .await
        });

        match handle.await.unwrap() {
            Ok(()) => {
                self.set_account_enabled_inner(enabled);
                Ok(())
            }
            Err(error) => {
                error!("Failed to change account notifications setting: {error}");
                Err(error)
            }
        }
    }

    fn set_account_enabled_inner(&self, enabled: bool) {
        if self.account_enabled() == enabled {
            return;
        }

        self.imp().account_enabled.set(enabled);
        self.notify_account_enabled();
    }

    /// Set the global setting about which messages trigger notifications.
    pub async fn set_global_setting(
        &self,
        setting: NotificationsGlobalSetting,
    ) -> Result<(), NotificationSettingsError> {
        let Some(api) = self.api() else {
            error!("Cannot update notifications settings when API is not initialized");
            return Err(NotificationSettingsError::UnableToUpdatePushRule);
        };

        let (group_all, one_to_one_all) = match setting {
            NotificationsGlobalSetting::All => (true, true),
            NotificationsGlobalSetting::DirectAndMentions => (false, true),
            NotificationsGlobalSetting::MentionsOnly => (false, false),
        };

        if let Err(error) = set_default_rooms_notifications_all(api.clone(), false, group_all).await
        {
            error!("Failed to change global group chats notifications setting: {error}");
            return Err(error);
        }
        if let Err(error) = set_default_rooms_notifications_all(api, true, one_to_one_all).await {
            error!("Failed to change global 1-to-1 chats notifications setting: {error}");
            return Err(error);
        }

        self.set_global_setting_inner(setting);

        Ok(())
    }

    fn set_global_setting_inner(&self, setting: NotificationsGlobalSetting) {
        if self.global_setting() == setting {
            return;
        }

        self.imp().global_setting.set(setting);
        self.notify_global_setting();
    }
}

impl Default for NotificationsSettings {
    fn default() -> Self {
        Self::new()
    }
}

async fn default_rooms_notifications_is_all(
    api: MatrixNotificationSettings,
    is_one_to_one: bool,
) -> bool {
    let mode = spawn_tokio!(async move {
        api.get_default_room_notification_mode(IsEncrypted::No, is_one_to_one.into())
            .await
    })
    .await
    .unwrap();

    mode == RoomNotificationMode::AllMessages
}

async fn set_default_rooms_notifications_all(
    api: MatrixNotificationSettings,
    is_one_to_one: bool,
    all: bool,
) -> Result<(), NotificationSettingsError> {
    let mode = if all {
        RoomNotificationMode::AllMessages
    } else {
        RoomNotificationMode::MentionsAndKeywordsOnly
    };

    spawn_tokio!(async move {
        api.set_default_room_notification_mode(IsEncrypted::No, is_one_to_one.into(), mode)
            .await
    })
    .await
    .unwrap()
}

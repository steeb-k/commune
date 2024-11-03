use std::collections::HashMap;

use futures_util::StreamExt;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    notification_settings::{
        IsEncrypted, NotificationSettings as MatrixNotificationSettings, RoomNotificationMode,
    },
    NotificationSettingsError,
};
use ruma::{
    push::{PredefinedOverrideRuleId, RuleKind},
    OwnedRoomId, RoomId,
};
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, warn};

use crate::{
    session::model::{Room, Session, SessionState},
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

/// The possible values for a room notifications setting.
#[derive(
    Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum, strum::Display, strum::EnumString,
)]
#[repr(u32)]
#[enum_type(name = "NotificationsRoomSetting")]
#[strum(serialize_all = "kebab-case")]
pub enum NotificationsRoomSetting {
    /// Use the global setting.
    #[default]
    Global = 0,
    /// All messages.
    All = 1,
    /// Only mentions and keywords.
    MentionsOnly = 2,
    /// No notifications.
    Mute = 3,
}

impl NotificationsRoomSetting {
    /// Convert to a [`RoomNotificationMode`].
    fn to_notification_mode(self) -> Option<RoomNotificationMode> {
        match self {
            Self::Global => None,
            Self::All => Some(RoomNotificationMode::AllMessages),
            Self::MentionsOnly => Some(RoomNotificationMode::MentionsAndKeywordsOnly),
            Self::Mute => Some(RoomNotificationMode::Mute),
        }
    }
}

impl From<RoomNotificationMode> for NotificationsRoomSetting {
    fn from(value: RoomNotificationMode) -> Self {
        match value {
            RoomNotificationMode::AllMessages => Self::All,
            RoomNotificationMode::MentionsAndKeywordsOnly => Self::MentionsOnly,
            RoomNotificationMode::Mute => Self::Mute,
        }
    }
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
        /// The list of keywords that trigger notifications.
        #[property(get)]
        pub keywords_list: gtk::StringList,
        /// The map of room ID to per-room notification setting.
        ///
        /// Any room not in this map uses the global setting.
        pub per_room_settings: RefCell<HashMap<OwnedRoomId, NotificationsRoomSetting>>,
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

            spawn!(clone!(
                #[weak]
                obj,
                async move {
                    obj.init_api().await;
                }
            ));
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

            session.connect_ready(clone!(
                #[weak(rename_to = obj)]
                self,
                move |_| {
                    spawn!(async move {
                        obj.init_api().await;
                    });
                }
            ));

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

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                obj.update().await;
            }
        ));

        while let Some(()) = receiver.next().await {
            spawn!(clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    obj.update().await;
                }
            ));
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
                error!("Could not get account notifications setting: {error}");
                true
            }
        };
        self.set_account_enabled_inner(account_enabled);

        if default_rooms_notifications_is_all(api.clone(), false).await {
            self.set_global_setting_inner(NotificationsGlobalSetting::All);
        } else if default_rooms_notifications_is_all(api, true).await {
            self.set_global_setting_inner(NotificationsGlobalSetting::DirectAndMentions);
        } else {
            self.set_global_setting_inner(NotificationsGlobalSetting::MentionsOnly);
        }

        self.update_keywords_list().await;
        self.update_per_room_settings().await;
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
                error!("Could not change account notifications setting: {error}");
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
            error!("Could not change global group chats notifications setting: {error}");
            return Err(error);
        }
        if let Err(error) = set_default_rooms_notifications_all(api, true, one_to_one_all).await {
            error!("Could not change global 1-to-1 chats notifications setting: {error}");
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

    /// Update the local list of keywords with the remote one.
    async fn update_keywords_list(&self) {
        let Some(api) = self.api() else {
            return;
        };

        let keywords = spawn_tokio!(async move { api.enabled_keywords().await })
            .await
            .unwrap();

        let list = &self.imp().keywords_list;
        let mut diverges_at = None;

        let keywords = keywords.iter().map(String::as_str).collect::<Vec<_>>();
        let new_len = keywords.len() as u32;
        let old_len = list.n_items();

        // Check if there is any keyword that changed, was moved or was added.
        for (pos, keyword) in keywords.iter().enumerate() {
            if Some(*keyword)
                != list
                    .item(pos as u32)
                    .and_downcast::<gtk::StringObject>()
                    .map(|o| o.string())
                    .as_deref()
            {
                diverges_at = Some(pos as u32);
                break;
            }
        }

        // Check if keywords were removed.
        if diverges_at.is_none() && old_len > new_len {
            diverges_at = Some(new_len);
        }

        let Some(pos) = diverges_at else {
            // Nothing to do.
            return;
        };

        let additions = &keywords[pos as usize..];
        list.splice(pos, old_len.saturating_sub(pos), additions);
    }

    /// Remove a keyword from the list.
    pub async fn remove_keyword(&self, keyword: String) -> Result<(), NotificationSettingsError> {
        let Some(api) = self.api() else {
            error!("Cannot update notifications settings when API is not initialized");
            return Err(NotificationSettingsError::UnableToUpdatePushRule);
        };

        let keyword_clone = keyword.clone();
        let handle = spawn_tokio!(async move { api.remove_keyword(&keyword_clone).await });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not remove notification keyword `{keyword}`: {error}");
            return Err(error);
        }

        self.update_keywords_list().await;

        Ok(())
    }

    /// Add a keyword to the list.
    pub async fn add_keyword(&self, keyword: String) -> Result<(), NotificationSettingsError> {
        let Some(api) = self.api() else {
            error!("Cannot update notifications settings when API is not initialized");
            return Err(NotificationSettingsError::UnableToUpdatePushRule);
        };

        let keyword_clone = keyword.clone();
        let handle = spawn_tokio!(async move { api.add_keyword(keyword_clone).await });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not add notification keyword `{keyword}`: {error}");
            return Err(error);
        }

        self.update_keywords_list().await;

        Ok(())
    }

    /// Update the local list of per-room settings with the remote one.
    async fn update_per_room_settings(&self) {
        let Some(api) = self.api() else {
            return;
        };

        let api_clone = api.clone();
        let room_ids = spawn_tokio!(async move {
            api_clone
                .get_rooms_with_user_defined_rules(Some(true))
                .await
        })
        .await
        .unwrap();

        // Update the local map.
        let mut per_room_settings = HashMap::with_capacity(room_ids.len());
        for room_id in room_ids {
            let Ok(room_id) = RoomId::parse(room_id) else {
                continue;
            };

            let room_id_clone = room_id.clone();
            let api_clone = api.clone();
            let handle = spawn_tokio!(async move {
                api_clone
                    .get_user_defined_room_notification_mode(&room_id_clone)
                    .await
            });

            if let Some(setting) = handle.await.unwrap() {
                per_room_settings.insert(room_id, setting.into());
            }
        }

        self.imp()
            .per_room_settings
            .replace(per_room_settings.clone());

        // Update the setting in the rooms.
        // Since we don't know when a room was added or removed, we have to update every
        // room.
        let Some(session) = self.session() else {
            return;
        };
        let room_list = session.room_list();

        for room in room_list.iter::<Room>() {
            let Ok(room) = room else {
                // Returns an error when the list changed, just stop.
                break;
            };

            if let Some(setting) = per_room_settings.get(room.room_id()) {
                room.set_notifications_setting(*setting);
            } else {
                room.set_notifications_setting(NotificationsRoomSetting::Global);
            }
        }
    }

    /// Set the notification setting for the room with the given ID.
    pub async fn set_per_room_setting(
        &self,
        room_id: OwnedRoomId,
        setting: NotificationsRoomSetting,
    ) -> Result<(), NotificationSettingsError> {
        let Some(api) = self.api() else {
            error!("Cannot update notifications settings when API is not initialized");
            return Err(NotificationSettingsError::UnableToUpdatePushRule);
        };

        let room_id_clone = room_id.clone();
        let handle = if let Some(mode) = setting.to_notification_mode() {
            spawn_tokio!(async move { api.set_room_notification_mode(&room_id_clone, mode).await })
        } else {
            spawn_tokio!(async move { api.delete_user_defined_room_rules(&room_id_clone).await })
        };

        if let Err(error) = handle.await.unwrap() {
            error!("Could not update notifications setting for room `{room_id}`: {error}");
            return Err(error);
        }

        self.update_per_room_settings().await;

        Ok(())
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

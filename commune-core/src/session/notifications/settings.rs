//! The notifications settings of an account, headless.
//!
//! The application's `NotificationsSettings`
//! (`src/session/notifications/notifications_settings.rs`) with the
//! `GObject` removed: the account-level switch, the global setting made of
//! the two default room modes, the keywords, the four special rules and
//! the per-room settings, all read from the SDK's push rules and read
//! again whenever they change. What stayed with the application is the
//! per-session switch, which is a `GSettings` key, and the `GtkStringList`
//! of keywords.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use eyeball::{SharedObservable, Subscriber};
use futures_util::StreamExt;
use matrix_sdk::{
    NotificationSettingsError,
    notification_settings::{
        IsEncrypted, NotificationSettings as MatrixNotificationSettings, RoomNotificationMode,
    },
};
use ruma::{
    OwnedRoomId, RoomId,
    push::{PredefinedOverrideRuleId, PredefinedUnderrideRuleId, RuleKind},
};
use tokio::task::AbortHandle;
use tokio_stream::wrappers::BroadcastStream;
use tracing::error;

use crate::{RUNTIME, UserFacingError, session::WeakSession, spawn_tokio};

/// The possible values for the global notifications setting.
#[derive(Debug, Default, Eq, PartialEq, Clone, Copy)]
pub enum NotificationsGlobalSetting {
    /// Every message in every room.
    #[default]
    All,
    /// Every message in 1-to-1 rooms, and mentions and keywords in every room.
    DirectAndMentions,
    /// Only mentions and keywords in every room.
    MentionsOnly,
}

impl NotificationsGlobalSetting {
    /// Get the string representation of this value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::DirectAndMentions => "direct-and-mentions",
            Self::MentionsOnly => "mentions-only",
        }
    }
}

impl std::str::FromStr for NotificationsGlobalSetting {
    type Err = ();

    /// Construct a `NotificationsGlobalSetting` from its string
    /// representation, if it is one.
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "all" => Ok(Self::All),
            "direct-and-mentions" => Ok(Self::DirectAndMentions),
            "mentions-only" => Ok(Self::MentionsOnly),
            _ => Err(()),
        }
    }
}

/// The predefined push rules that apply across all rooms and can be toggled
/// on their own.
///
/// Each maps to a well-known rule of the push module. The user-mention and
/// room-mention rules are set through the SDK, which keeps the deprecated
/// rules they replaced (`.m.rule.contains_display_name`,
/// `.m.rule.contains_user_name` and `.m.rule.roomnotif`) in step for older
/// clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationsSpecialRule {
    /// Mentions of the user, `.m.rule.is_user_mention`.
    UserMention,
    /// Mentions of the whole room, `.m.rule.is_room_mention`.
    RoomMention,
    /// Invites to a room, `.m.rule.invite_for_me`.
    Invite,
    /// Incoming calls, `.m.rule.call`.
    Call,
}

impl NotificationsSpecialRule {
    /// The four rules, in the order the settings page lists them.
    pub const ALL: [Self; 4] = [
        Self::UserMention,
        Self::RoomMention,
        Self::Invite,
        Self::Call,
    ];

    /// The kind and ID of the push rule.
    fn kind_and_id(self) -> (RuleKind, String) {
        match self {
            Self::UserMention => (
                RuleKind::Override,
                PredefinedOverrideRuleId::IsUserMention.to_string(),
            ),
            Self::RoomMention => (
                RuleKind::Override,
                PredefinedOverrideRuleId::IsRoomMention.to_string(),
            ),
            Self::Invite => (
                RuleKind::Override,
                PredefinedOverrideRuleId::InviteForMe.to_string(),
            ),
            Self::Call => (
                RuleKind::Underride,
                PredefinedUnderrideRuleId::Call.to_string(),
            ),
        }
    }
}

/// The possible values for a room notifications setting.
#[derive(Debug, Default, Eq, PartialEq, Clone, Copy)]
pub enum NotificationsRoomSetting {
    /// Use the global setting.
    #[default]
    Global,
    /// All messages.
    All,
    /// Only mentions and keywords.
    MentionsOnly,
    /// No notifications.
    Mute,
}

impl NotificationsRoomSetting {
    /// Convert to a [`RoomNotificationMode`].
    #[must_use]
    pub fn to_notification_mode(self) -> Option<RoomNotificationMode> {
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

/// What can go wrong while changing the notifications settings.
#[derive(Debug, thiserror::Error)]
pub enum NotificationsError {
    /// The settings have not been read yet, so there is nothing to change.
    #[error("the notifications settings are not loaded yet")]
    NotLoaded,
    /// The homeserver refused, or the push rules could not be changed.
    ///
    /// Boxed because the SDK's error is large enough that carrying it by
    /// value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<NotificationSettingsError>),
}

impl UserFacingError for NotificationsError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NotLoaded => "The notifications settings are not loaded yet.".to_owned(),
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// The notifications settings of an account.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct NotificationsSettings {
    inner: Arc<NotificationsSettingsInner>,
}

#[derive(Debug)]
struct NotificationsSettingsInner {
    /// The session these settings belong to.
    session: WeakSession,
    /// The SDK notification settings API, once the client exists.
    api: Mutex<Option<MatrixNotificationSettings>>,
    /// Whether notifications are enabled for this Matrix account.
    account_enabled: SharedObservable<bool>,
    /// The global setting about which messages trigger notifications.
    global_setting: SharedObservable<NotificationsGlobalSetting>,
    /// The list of keywords that trigger notifications.
    keywords: SharedObservable<Vec<String>>,
    /// Whether mentions of the user trigger notifications.
    mention_rule_enabled: SharedObservable<bool>,
    /// Whether mentions of the whole room trigger notifications.
    room_mention_rule_enabled: SharedObservable<bool>,
    /// Whether invites trigger notifications.
    invite_rule_enabled: SharedObservable<bool>,
    /// Whether incoming calls trigger notifications.
    call_rule_enabled: SharedObservable<bool>,
    /// The map of room ID to per-room notification setting.
    ///
    /// Any room not in this map uses the global setting.
    per_room_settings: SharedObservable<HashMap<OwnedRoomId, NotificationsRoomSetting>>,
    /// The task following the SDK's changes.
    watch_handle: Mutex<Option<AbortHandle>>,
}

impl Drop for NotificationsSettingsInner {
    fn drop(&mut self) {
        if let Ok(Some(handle)) = self.watch_handle.get_mut().map(Option::take) {
            handle.abort();
        }
    }
}

impl NotificationsSettings {
    /// Create the notifications settings of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        Self {
            inner: Arc::new(NotificationsSettingsInner {
                session,
                api: Mutex::new(None),
                account_enabled: SharedObservable::new(false),
                global_setting: SharedObservable::new(NotificationsGlobalSetting::default()),
                keywords: SharedObservable::new(Vec::new()),
                mention_rule_enabled: SharedObservable::new(false),
                room_mention_rule_enabled: SharedObservable::new(false),
                invite_rule_enabled: SharedObservable::new(false),
                call_rule_enabled: SharedObservable::new(false),
                per_room_settings: SharedObservable::new(HashMap::new()),
                watch_handle: Mutex::new(None),
            }),
        }
    }

    /// Read the settings from the SDK, and follow them from there.
    ///
    /// Needs the client, so this runs once the session is ready.
    pub async fn load(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        let client = session.client();
        let api = spawn_tokio!(async move { client.notification_settings().await })
            .await
            .expect("task was not aborted");
        let stream = BroadcastStream::new(api.subscribe_to_changes());

        *self.inner.api.lock().expect("mutex is not poisoned") = Some(api);

        let weak = Arc::downgrade(&self.inner);
        let fut = stream.for_each(move |res| {
            let weak = weak.clone();
            async move {
                if res.is_err() {
                    return;
                }

                if let Some(inner) = weak.upgrade() {
                    NotificationsSettings { inner }.update().await;
                }
            }
        });

        let handle = RUNTIME.spawn(fut).abort_handle();
        if let Some(previous) = self
            .inner
            .watch_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }

        self.update().await;
    }

    /// The SDK notification settings API, if the settings were loaded.
    fn api(&self) -> Option<MatrixNotificationSettings> {
        self.inner
            .api
            .lock()
            .expect("mutex is not poisoned")
            .clone()
    }

    /// The SDK notification settings API, or the error to give for a change
    /// asked before the settings were loaded.
    fn api_or_err(&self) -> Result<MatrixNotificationSettings, NotificationsError> {
        self.api().ok_or_else(|| {
            error!("Cannot update notifications settings when API is not initialized");
            NotificationsError::NotLoaded
        })
    }

    /// Whether notifications are enabled for this Matrix account.
    #[must_use]
    pub fn account_enabled(&self) -> bool {
        self.inner.account_enabled.get()
    }

    /// Subscribe to whether notifications are enabled for this Matrix
    /// account.
    pub fn subscribe_account_enabled(&self) -> Subscriber<bool> {
        self.inner.account_enabled.subscribe()
    }

    /// The global setting about which messages trigger notifications.
    #[must_use]
    pub fn global_setting(&self) -> NotificationsGlobalSetting {
        self.inner.global_setting.get()
    }

    /// Subscribe to the global setting about which messages trigger
    /// notifications.
    pub fn subscribe_global_setting(&self) -> Subscriber<NotificationsGlobalSetting> {
        self.inner.global_setting.subscribe()
    }

    /// The list of keywords that trigger notifications.
    #[must_use]
    pub fn keywords(&self) -> Vec<String> {
        self.inner.keywords.get()
    }

    /// Subscribe to the list of keywords that trigger notifications.
    pub fn subscribe_keywords(&self) -> Subscriber<Vec<String>> {
        self.inner.keywords.subscribe()
    }

    /// The observable of the given special push rule.
    fn special_rule(&self, rule: NotificationsSpecialRule) -> &SharedObservable<bool> {
        match rule {
            NotificationsSpecialRule::UserMention => &self.inner.mention_rule_enabled,
            NotificationsSpecialRule::RoomMention => &self.inner.room_mention_rule_enabled,
            NotificationsSpecialRule::Invite => &self.inner.invite_rule_enabled,
            NotificationsSpecialRule::Call => &self.inner.call_rule_enabled,
        }
    }

    /// The state of the given special push rule.
    #[must_use]
    pub fn special_rule_enabled(&self, rule: NotificationsSpecialRule) -> bool {
        self.special_rule(rule).get()
    }

    /// Subscribe to the state of the given special push rule.
    pub fn subscribe_special_rule_enabled(
        &self,
        rule: NotificationsSpecialRule,
    ) -> Subscriber<bool> {
        self.special_rule(rule).subscribe()
    }

    /// The per-room notification settings, by room ID.
    ///
    /// Any room not in this map uses the global setting.
    #[must_use]
    pub fn per_room_settings(&self) -> HashMap<OwnedRoomId, NotificationsRoomSetting> {
        self.inner.per_room_settings.get()
    }

    /// Subscribe to the per-room notification settings.
    pub fn subscribe_per_room_settings(
        &self,
    ) -> Subscriber<HashMap<OwnedRoomId, NotificationsRoomSetting>> {
        self.inner.per_room_settings.subscribe()
    }

    /// The notification setting of the room with the given ID.
    #[must_use]
    pub fn per_room_setting(&self, room_id: &RoomId) -> NotificationsRoomSetting {
        self.inner
            .per_room_settings
            .read()
            .get(room_id)
            .copied()
            .unwrap_or_default()
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

        let account_enabled = match handle.await.expect("task was not aborted") {
            // The rule disables notifications, so we need to invert the boolean.
            Ok(enabled) => !enabled,
            Err(rule_error) => {
                error!("Could not get account notifications setting: {rule_error}");
                true
            }
        };
        self.inner.account_enabled.set_if_not_eq(account_enabled);

        let global_setting = if default_rooms_notifications_is_all(api.clone(), false).await {
            NotificationsGlobalSetting::All
        } else if default_rooms_notifications_is_all(api, true).await {
            NotificationsGlobalSetting::DirectAndMentions
        } else {
            NotificationsGlobalSetting::MentionsOnly
        };
        self.inner.global_setting.set_if_not_eq(global_setting);

        self.update_keywords().await;
        self.update_special_rules().await;
        self.update_per_room_settings().await;
    }

    /// Update the state of the special push rules from the SDK API.
    async fn update_special_rules(&self) {
        for rule in NotificationsSpecialRule::ALL {
            let Some(api) = self.api() else {
                return;
            };

            let (kind, rule_id) = rule.kind_and_id();
            let handle = spawn_tokio!(async move { api.is_push_rule_enabled(kind, rule_id).await });

            match handle.await.expect("task was not aborted") {
                Ok(enabled) => {
                    self.special_rule(rule).set_if_not_eq(enabled);
                }
                Err(rule_error) => {
                    error!("Could not get the state of the {rule:?} push rule: {rule_error}");
                }
            }
        }
    }

    /// Update the list of keywords from the SDK API.
    async fn update_keywords(&self) {
        let Some(api) = self.api() else {
            return;
        };

        let keywords = spawn_tokio!(async move { api.enabled_keywords().await })
            .await
            .expect("task was not aborted");

        self.inner
            .keywords
            .set_if_not_eq(keywords.into_iter().collect());
    }

    /// Update the per-room settings from the SDK API, and tell the rooms.
    ///
    /// Since we do not know when a room was added or removed, every room
    /// in the list is told, as the application told every one of its
    /// rooms.
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
        .expect("task was not aborted");

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

            if let Some(setting) = handle.await.expect("task was not aborted") {
                per_room_settings.insert(room_id, setting.into());
            }
        }

        self.inner
            .per_room_settings
            .set_if_not_eq(per_room_settings.clone());

        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        for room in session.room_list().snapshot() {
            let setting = per_room_settings
                .get(room.room_id())
                .copied()
                .unwrap_or_default();
            room.set_notifications_setting(setting);
        }
    }

    /// Set whether notifications are enabled for this Matrix account.
    pub async fn set_account_enabled(&self, enabled: bool) -> Result<(), NotificationsError> {
        let api = self.api_or_err()?;

        let handle = spawn_tokio!(async move {
            api.set_push_rule_enabled(
                RuleKind::Override,
                PredefinedOverrideRuleId::Master,
                // The rule disables notifications, so we need to invert the boolean.
                !enabled,
            )
            .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(()) => {
                self.inner.account_enabled.set_if_not_eq(enabled);
                Ok(())
            }
            Err(rule_error) => {
                error!("Could not change account notifications setting: {rule_error}");
                Err(NotificationsError::Server(Box::new(rule_error)))
            }
        }
    }

    /// Set the global setting about which messages trigger notifications.
    pub async fn set_global_setting(
        &self,
        setting: NotificationsGlobalSetting,
    ) -> Result<(), NotificationsError> {
        let api = self.api_or_err()?;

        let (group_all, one_to_one_all) = match setting {
            NotificationsGlobalSetting::All => (true, true),
            NotificationsGlobalSetting::DirectAndMentions => (false, true),
            NotificationsGlobalSetting::MentionsOnly => (false, false),
        };

        if let Err(rule_error) =
            set_default_rooms_notifications_all(api.clone(), false, group_all).await
        {
            error!("Could not change global group chats notifications setting: {rule_error}");
            return Err(NotificationsError::Server(Box::new(rule_error)));
        }
        if let Err(rule_error) =
            set_default_rooms_notifications_all(api, true, one_to_one_all).await
        {
            error!("Could not change global 1-to-1 chats notifications setting: {rule_error}");
            return Err(NotificationsError::Server(Box::new(rule_error)));
        }

        self.inner.global_setting.set_if_not_eq(setting);

        Ok(())
    }

    /// Remove a keyword from the list.
    pub async fn remove_keyword(&self, keyword: String) -> Result<(), NotificationsError> {
        let api = self.api_or_err()?;

        let keyword_clone = keyword.clone();
        let handle = spawn_tokio!(async move { api.remove_keyword(&keyword_clone).await });

        if let Err(rule_error) = handle.await.expect("task was not aborted") {
            error!("Could not remove notification keyword `{keyword}`: {rule_error}");
            return Err(NotificationsError::Server(Box::new(rule_error)));
        }

        self.update_keywords().await;

        Ok(())
    }

    /// Add a keyword to the list.
    pub async fn add_keyword(&self, keyword: String) -> Result<(), NotificationsError> {
        let api = self.api_or_err()?;

        let keyword_clone = keyword.clone();
        let handle = spawn_tokio!(async move { api.add_keyword(keyword_clone).await });

        if let Err(rule_error) = handle.await.expect("task was not aborted") {
            error!("Could not add notification keyword `{keyword}`: {rule_error}");
            return Err(NotificationsError::Server(Box::new(rule_error)));
        }

        self.update_keywords().await;

        Ok(())
    }

    /// Set whether the given special push rule is enabled.
    pub async fn set_special_rule_enabled(
        &self,
        rule: NotificationsSpecialRule,
        enabled: bool,
    ) -> Result<(), NotificationsError> {
        let api = self.api_or_err()?;

        let (kind, rule_id) = rule.kind_and_id();
        let handle =
            spawn_tokio!(async move { api.set_push_rule_enabled(kind, rule_id, enabled).await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => {
                self.special_rule(rule).set_if_not_eq(enabled);
                Ok(())
            }
            Err(rule_error) => {
                error!("Could not change the state of the {rule:?} push rule: {rule_error}");
                Err(NotificationsError::Server(Box::new(rule_error)))
            }
        }
    }

    /// Set the notification setting for the room with the given ID.
    pub async fn set_per_room_setting(
        &self,
        room_id: OwnedRoomId,
        setting: NotificationsRoomSetting,
    ) -> Result<(), NotificationsError> {
        let api = self.api_or_err()?;

        let room_id_clone = room_id.clone();
        let handle = if let Some(mode) = setting.to_notification_mode() {
            spawn_tokio!(async move { api.set_room_notification_mode(&room_id_clone, mode).await })
        } else {
            spawn_tokio!(async move { api.delete_user_defined_room_rules(&room_id_clone).await })
        };

        if let Err(rule_error) = handle.await.expect("task was not aborted") {
            error!("Could not update notifications setting for room `{room_id}`: {rule_error}");
            return Err(NotificationsError::Server(Box::new(rule_error)));
        }

        self.update_per_room_settings().await;

        Ok(())
    }
}

/// Start loading the settings of the given session on the runtime, for a
/// caller that cannot await.
pub(crate) fn spawn_load(settings: &NotificationsSettings) {
    let settings = settings.clone();
    RUNTIME.spawn(async move {
        settings.load().await;
    });
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
    .expect("task was not aborted");

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
    .expect("task was not aborted")
}

pub(crate) use commune_core::session::NotificationsSpecialRule;
use commune_core::session::{
    NotificationsGlobalSetting as CoreGlobalSetting, NotificationsRoomSetting as CoreRoomSetting,
    NotificationsSettings as CoreNotificationsSettings,
};
use gtk::{glib, prelude::*, subclass::prelude::*};
use ruma::OwnedRoomId;
use tokio::task::AbortHandle;
use tracing::error;

use crate::{core_bridge::ObjectWatcher, session::Session, spawn_tokio};

/// The possible values for the global notifications setting.
///
/// The core's [`NotificationsGlobalSetting`](CoreGlobalSetting) as a
/// `glib` enum, for the properties that carry it.
#[derive(Debug, Default, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "NotificationsGlobalSetting")]
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
    pub(crate) fn as_str(self) -> &'static str {
        CoreGlobalSetting::from(self).as_str()
    }

    /// Construct a `NotificationsGlobalSetting` from its string representation.
    ///
    /// Panics if the string does not match a variant of this enum.
    pub(crate) fn from_str(s: &str) -> Self {
        s.parse::<CoreGlobalSetting>()
            .unwrap_or_else(|()| panic!("Unknown NotificationsGlobalSetting: {s}"))
            .into()
    }
}

impl From<CoreGlobalSetting> for NotificationsGlobalSetting {
    fn from(value: CoreGlobalSetting) -> Self {
        match value {
            CoreGlobalSetting::All => Self::All,
            CoreGlobalSetting::DirectAndMentions => Self::DirectAndMentions,
            CoreGlobalSetting::MentionsOnly => Self::MentionsOnly,
        }
    }
}

impl From<NotificationsGlobalSetting> for CoreGlobalSetting {
    fn from(value: NotificationsGlobalSetting) -> Self {
        match value {
            NotificationsGlobalSetting::All => Self::All,
            NotificationsGlobalSetting::DirectAndMentions => Self::DirectAndMentions,
            NotificationsGlobalSetting::MentionsOnly => Self::MentionsOnly,
        }
    }
}

/// The possible values for a room notifications setting.
///
/// The core's [`NotificationsRoomSetting`](CoreRoomSetting) as a `glib`
/// enum, for the properties that carry it.
#[derive(Debug, Default, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "NotificationsRoomSetting")]
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

impl From<CoreRoomSetting> for NotificationsRoomSetting {
    fn from(value: CoreRoomSetting) -> Self {
        match value {
            CoreRoomSetting::Global => Self::Global,
            CoreRoomSetting::All => Self::All,
            CoreRoomSetting::MentionsOnly => Self::MentionsOnly,
            CoreRoomSetting::Mute => Self::Mute,
        }
    }
}

impl From<NotificationsRoomSetting> for CoreRoomSetting {
    fn from(value: NotificationsRoomSetting) -> Self {
        match value {
            NotificationsRoomSetting::Global => Self::Global,
            NotificationsRoomSetting::All => Self::All,
            NotificationsRoomSetting::MentionsOnly => Self::MentionsOnly,
            NotificationsRoomSetting::Mute => Self::Mute,
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
        session: glib::WeakRef<Session>,
        /// Whether notifications are enabled for this Matrix account.
        #[property(get)]
        account_enabled: Cell<bool>,
        /// Whether notifications are enabled for this session.
        #[property(get, set = Self::set_session_enabled, explicit_notify)]
        session_enabled: Cell<bool>,
        /// The global setting about which messages trigger notifications.
        #[property(get, builder(NotificationsGlobalSetting::default()))]
        global_setting: Cell<NotificationsGlobalSetting>,
        /// The list of keywords that trigger notifications.
        #[property(get)]
        keywords_list: gtk::StringList,
        /// Whether mentions of the user trigger notifications.
        #[property(get)]
        mention_rule_enabled: Cell<bool>,
        /// Whether mentions of the whole room trigger notifications.
        #[property(get)]
        room_mention_rule_enabled: Cell<bool>,
        /// Whether invites trigger notifications.
        #[property(get)]
        invite_rule_enabled: Cell<bool>,
        /// Whether incoming calls trigger notifications.
        #[property(get)]
        call_rule_enabled: Cell<bool>,
        /// The task following the core's settings.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NotificationsSettings {
        const NAME: &'static str = "NotificationsSettings";
        type Type = super::NotificationsSettings;
    }

    #[glib::derived_properties]
    impl ObjectImpl for NotificationsSettings {
        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

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

            self.watch_core();
        }

        /// Set whether notifications are enabled for this session.
        fn set_session_enabled(&self, enabled: bool) {
            if self.session_enabled.get() == enabled {
                return;
            }

            if !enabled && let Some(session) = self.session.upgrade() {
                session.notifications().clear();
            }

            self.session_enabled.set(enabled);
            self.obj().notify_session_enabled();
        }

        /// The core's settings, while the session is there.
        pub(super) fn core(&self) -> Option<CoreNotificationsSettings> {
            self.session
                .upgrade()
                .map(|session| session.core().notifications_settings().clone())
        }

        /// Follow the core's settings.
        ///
        /// The core reads them once the session is ready, which is when the
        /// application read them too.
        fn watch_core(&self) {
            type S = super::NotificationsSettings;

            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }

            let Some(core) = self.core() else {
                return;
            };

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe_account_enabled(), |obj: &S, enabled| {
                    obj.imp().set_account_enabled(enabled);
                })
                .follow(core.subscribe_global_setting(), |obj: &S, setting| {
                    obj.imp().set_global_setting(setting.into());
                })
                .follow(core.subscribe_keywords(), |obj: &S, keywords| {
                    obj.imp().update_keywords_list(&keywords);
                })
                .follow(
                    core.subscribe_special_rule_enabled(NotificationsSpecialRule::UserMention),
                    |obj: &S, enabled| {
                        obj.imp().set_special_rule_enabled(
                            NotificationsSpecialRule::UserMention,
                            enabled,
                        );
                    },
                )
                .follow(
                    core.subscribe_special_rule_enabled(NotificationsSpecialRule::RoomMention),
                    |obj: &S, enabled| {
                        obj.imp().set_special_rule_enabled(
                            NotificationsSpecialRule::RoomMention,
                            enabled,
                        );
                    },
                )
                .follow(
                    core.subscribe_special_rule_enabled(NotificationsSpecialRule::Invite),
                    |obj: &S, enabled| {
                        obj.imp()
                            .set_special_rule_enabled(NotificationsSpecialRule::Invite, enabled);
                    },
                )
                .follow(
                    core.subscribe_special_rule_enabled(NotificationsSpecialRule::Call),
                    |obj: &S, enabled| {
                        obj.imp()
                            .set_special_rule_enabled(NotificationsSpecialRule::Call, enabled);
                    },
                )
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.set_account_enabled(core.account_enabled());
            self.set_global_setting(core.global_setting().into());
            self.update_keywords_list(&core.keywords());
            for rule in NotificationsSpecialRule::ALL {
                self.set_special_rule_enabled(rule, core.special_rule_enabled(rule));
            }
        }

        /// Mirror whether the given special push rule is enabled.
        fn set_special_rule_enabled(&self, rule: NotificationsSpecialRule, enabled: bool) {
            let cell = match rule {
                NotificationsSpecialRule::UserMention => &self.mention_rule_enabled,
                NotificationsSpecialRule::RoomMention => &self.room_mention_rule_enabled,
                NotificationsSpecialRule::Invite => &self.invite_rule_enabled,
                NotificationsSpecialRule::Call => &self.call_rule_enabled,
            };

            if cell.get() == enabled {
                return;
            }
            cell.set(enabled);

            let obj = self.obj();
            match rule {
                NotificationsSpecialRule::UserMention => obj.notify_mention_rule_enabled(),
                NotificationsSpecialRule::RoomMention => obj.notify_room_mention_rule_enabled(),
                NotificationsSpecialRule::Invite => obj.notify_invite_rule_enabled(),
                NotificationsSpecialRule::Call => obj.notify_call_rule_enabled(),
            }
        }

        /// Mirror whether notifications are enabled for this Matrix account.
        fn set_account_enabled(&self, enabled: bool) {
            if self.account_enabled.get() == enabled {
                return;
            }

            self.account_enabled.set(enabled);
            self.obj().notify_account_enabled();
        }

        /// Mirror the global setting about which messages trigger
        /// notifications.
        fn set_global_setting(&self, setting: NotificationsGlobalSetting) {
            if self.global_setting.get() == setting {
                return;
            }

            self.global_setting.set(setting);
            self.obj().notify_global_setting();
        }

        /// Update the list of keywords with the given one.
        ///
        /// The `GtkStringList` is spliced from the first keyword that differs,
        /// so that the rows before it stay where they are.
        fn update_keywords_list(&self, keywords: &[String]) {
            let list = &self.keywords_list;
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
    }
}

glib::wrapper! {
    /// The notifications settings of a `Session`.
    ///
    /// The settings are the core's; this presents them, and keeps the
    /// per-session switch, which is this application's own.
    pub struct NotificationsSettings(ObjectSubclass<imp::NotificationsSettings>);
}

impl NotificationsSettings {
    /// Create a new `NotificationsSettings`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set whether notifications are enabled for this Matrix account.
    pub(crate) async fn set_account_enabled(&self, enabled: bool) -> Result<(), ()> {
        let core = self.imp().core().ok_or(())?;

        spawn_tokio!(async move { core.set_account_enabled(enabled).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not change account notifications setting: {error}");
            })
    }

    /// Set the global setting about which messages trigger notifications.
    pub(crate) async fn set_global_setting(
        &self,
        setting: NotificationsGlobalSetting,
    ) -> Result<(), ()> {
        let core = self.imp().core().ok_or(())?;

        spawn_tokio!(async move { core.set_global_setting(setting.into()).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not change global notifications setting: {error}");
            })
    }

    /// Remove a keyword from the list.
    pub(crate) async fn remove_keyword(&self, keyword: String) -> Result<(), ()> {
        let core = self.imp().core().ok_or(())?;

        spawn_tokio!(async move { core.remove_keyword(keyword).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not remove notification keyword: {error}");
            })
    }

    /// Add a keyword to the list.
    pub(crate) async fn add_keyword(&self, keyword: String) -> Result<(), ()> {
        let core = self.imp().core().ok_or(())?;

        spawn_tokio!(async move { core.add_keyword(keyword).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not add notification keyword: {error}");
            })
    }

    /// The state of the given special push rule.
    pub(crate) fn special_rule_enabled(&self, rule: NotificationsSpecialRule) -> bool {
        match rule {
            NotificationsSpecialRule::UserMention => self.mention_rule_enabled(),
            NotificationsSpecialRule::RoomMention => self.room_mention_rule_enabled(),
            NotificationsSpecialRule::Invite => self.invite_rule_enabled(),
            NotificationsSpecialRule::Call => self.call_rule_enabled(),
        }
    }

    /// Set whether the given special push rule is enabled.
    pub(crate) async fn set_special_rule_enabled(
        &self,
        rule: NotificationsSpecialRule,
        enabled: bool,
    ) -> Result<(), ()> {
        let core = self.imp().core().ok_or(())?;

        spawn_tokio!(async move { core.set_special_rule_enabled(rule, enabled).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not change the state of the {rule:?} push rule: {error}");
            })
    }

    /// Set the notification setting for the room with the given ID.
    pub(crate) async fn set_per_room_setting(
        &self,
        room_id: OwnedRoomId,
        setting: NotificationsRoomSetting,
    ) -> Result<(), ()> {
        let core = self.imp().core().ok_or(())?;

        spawn_tokio!(async move { core.set_per_room_setting(room_id, setting.into()).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not update the notifications setting of the room: {error}");
            })
    }
}

impl Default for NotificationsSettings {
    fn default() -> Self {
        Self::new()
    }
}

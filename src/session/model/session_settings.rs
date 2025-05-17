use std::collections::BTreeSet;

use gtk::{glib, glib::closure_local, prelude::*, subclass::prelude::*};
use indexmap::IndexSet;
use ruma::{serde::SerializeAsRefStr, OwnedServerName};
use serde::{Deserialize, Serialize};

use super::{Room, SidebarSectionName};
use crate::{session_list::SessionListSettings, Application};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct StoredSessionSettings {
    /// Custom servers to explore.
    #[serde(default, skip_serializing_if = "IndexSet::is_empty")]
    explore_custom_servers: IndexSet<OwnedServerName>,

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

    /// The sections that are expanded.
    #[serde(default)]
    sections_expanded: SectionsExpanded,

    /// Which rooms display media previews for this session.
    #[serde(default, skip_serializing_if = "ruma::serde::is_default")]
    media_previews_enabled: MediaPreviewsSetting,

    /// Whether to display avatars in invites.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    invite_avatars_enabled: bool,
}

impl Default for StoredSessionSettings {
    fn default() -> Self {
        Self {
            explore_custom_servers: Default::default(),
            notifications_enabled: true,
            public_read_receipts_enabled: true,
            typing_enabled: true,
            sections_expanded: Default::default(),
            media_previews_enabled: Default::default(),
            invite_avatars_enabled: true,
        }
    }
}

mod imp {
    use std::{
        cell::{OnceCell, RefCell},
        marker::PhantomData,
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SessionSettings)]
    pub struct SessionSettings {
        /// The ID of the session these settings are for.
        #[property(get, construct_only)]
        session_id: OnceCell<String>,
        /// The stored settings.
        pub(super) stored_settings: RefCell<StoredSessionSettings>,
        /// Whether notifications are enabled for this session.
        #[property(get = Self::notifications_enabled, set = Self::set_notifications_enabled, explicit_notify, default = true)]
        notifications_enabled: PhantomData<bool>,
        /// Whether public read receipts are enabled for this session.
        #[property(get = Self::public_read_receipts_enabled, set = Self::set_public_read_receipts_enabled, explicit_notify, default = true)]
        public_read_receipts_enabled: PhantomData<bool>,
        /// Whether typing notifications are enabled for this session.
        #[property(get = Self::typing_enabled, set = Self::set_typing_enabled, explicit_notify, default = true)]
        typing_enabled: PhantomData<bool>,
        /// Whether to display avatars in invites.
        #[property(get = Self::invite_avatars_enabled, set = Self::set_invite_avatars_enabled, explicit_notify, default = true)]
        invite_avatars_enabled: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionSettings {
        const NAME: &'static str = "SessionSettings";
        type Type = super::SessionSettings;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SessionSettings {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("media-previews-enabled-changed").build()]);
            SIGNALS.as_ref()
        }
    }

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

            self.stored_settings.borrow_mut().notifications_enabled = enabled;
            session_list_settings().save();
            self.obj().notify_notifications_enabled();
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

            self.stored_settings
                .borrow_mut()
                .public_read_receipts_enabled = enabled;
            session_list_settings().save();
            self.obj().notify_public_read_receipts_enabled();
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

            self.stored_settings.borrow_mut().typing_enabled = enabled;
            session_list_settings().save();
            self.obj().notify_typing_enabled();
        }

        /// Whether to display avatars in invites.
        fn invite_avatars_enabled(&self) -> bool {
            self.stored_settings.borrow().invite_avatars_enabled
        }

        /// Set whether to display avatars in invites.
        fn set_invite_avatars_enabled(&self, enabled: bool) {
            if self.invite_avatars_enabled() == enabled {
                return;
            }

            self.stored_settings.borrow_mut().invite_avatars_enabled = enabled;
            session_list_settings().save();
            self.obj().notify_invite_avatars_enabled();
        }
    }
}

glib::wrapper! {
    /// The settings of a `Session`.
    pub struct SessionSettings(ObjectSubclass<imp::SessionSettings>);
}

impl SessionSettings {
    /// Create a new `SessionSettings` for the given session ID.
    pub(crate) fn new(session_id: &str) -> Self {
        glib::Object::builder()
            .property("session-id", session_id)
            .build()
    }

    /// Restore existing `SessionSettings` with the given session ID and stored
    /// settings.
    pub(crate) fn restore(session_id: &str, stored_settings: StoredSessionSettings) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session-id", session_id)
            .build();
        *obj.imp().stored_settings.borrow_mut() = stored_settings;
        obj
    }

    /// The stored settings.
    pub(crate) fn stored_settings(&self) -> StoredSessionSettings {
        self.imp().stored_settings.borrow().clone()
    }

    /// Delete the settings from the application settings.
    pub(crate) fn delete(&self) {
        session_list_settings().remove(&self.session_id());
    }

    /// Custom servers to explore.
    pub(crate) fn explore_custom_servers(&self) -> IndexSet<OwnedServerName> {
        self.imp()
            .stored_settings
            .borrow()
            .explore_custom_servers
            .clone()
    }

    /// Set the custom servers to explore.
    pub(crate) fn set_explore_custom_servers(&self, servers: IndexSet<OwnedServerName>) {
        if self.explore_custom_servers() == servers {
            return;
        }

        self.imp()
            .stored_settings
            .borrow_mut()
            .explore_custom_servers = servers;
        session_list_settings().save();
    }

    /// Whether the section with the given name is expanded.
    pub(crate) fn is_section_expanded(&self, section_name: SidebarSectionName) -> bool {
        self.imp()
            .stored_settings
            .borrow()
            .sections_expanded
            .is_section_expanded(section_name)
    }

    /// Set whether the section with the given name is expanded.
    pub(crate) fn set_section_expanded(&self, section_name: SidebarSectionName, expanded: bool) {
        self.imp()
            .stored_settings
            .borrow_mut()
            .sections_expanded
            .set_section_expanded(section_name, expanded);
        session_list_settings().save();
    }

    /// Whether the given room should display media previews.
    pub(crate) fn should_room_show_media_previews(&self, room: &Room) -> bool {
        self.imp()
            .stored_settings
            .borrow()
            .media_previews_enabled
            .should_room_show_media_previews(room)
    }

    /// Which rooms display media previews.
    pub(crate) fn media_previews_global_enabled(&self) -> MediaPreviewsGlobalSetting {
        self.imp()
            .stored_settings
            .borrow()
            .media_previews_enabled
            .global
    }

    /// Set which rooms display media previews.
    pub(crate) fn set_media_previews_global_enabled(&self, setting: MediaPreviewsGlobalSetting) {
        self.imp()
            .stored_settings
            .borrow_mut()
            .media_previews_enabled
            .global = setting;
        session_list_settings().save();
        self.emit_by_name::<()>("media-previews-enabled-changed", &[]);
    }

    /// Connect to the signal emitted when the media previews setting changed.
    pub fn connect_media_previews_enabled_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "media-previews-enabled-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

/// The sections that are expanded.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct SectionsExpanded(BTreeSet<SidebarSectionName>);

impl SectionsExpanded {
    /// Whether the section with the given name is expanded.
    pub(crate) fn is_section_expanded(&self, section_name: SidebarSectionName) -> bool {
        self.0.contains(&section_name)
    }

    /// Set whether the section with the given name is expanded.
    pub(crate) fn set_section_expanded(
        &mut self,
        section_name: SidebarSectionName,
        expanded: bool,
    ) {
        if expanded {
            self.0.insert(section_name);
        } else {
            self.0.remove(&section_name);
        }
    }
}

impl Default for SectionsExpanded {
    fn default() -> Self {
        Self(BTreeSet::from([
            SidebarSectionName::VerificationRequest,
            SidebarSectionName::Invited,
            SidebarSectionName::Favorite,
            SidebarSectionName::Normal,
            SidebarSectionName::LowPriority,
        ]))
    }
}

/// Setting about which rooms display media previews.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MediaPreviewsSetting {
    /// The default setting for all rooms.
    #[serde(default, skip_serializing_if = "ruma::serde::is_default")]
    global: MediaPreviewsGlobalSetting,
}

impl MediaPreviewsSetting {
    // Whether the given room should show room previews according to this setting.
    pub(crate) fn should_room_show_media_previews(&self, room: &Room) -> bool {
        self.global.should_room_show_media_previews(room)
    }
}

/// Possible values of the global setting about which rooms display media
/// previews.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    strum::AsRefStr,
    strum::EnumString,
    SerializeAsRefStr,
)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum MediaPreviewsGlobalSetting {
    /// All rooms show media previews.
    All,
    /// Only private rooms show media previews.
    #[default]
    Private,
    /// No rooms show media previews.
    None,
}

impl MediaPreviewsGlobalSetting {
    /// Whether the given room should show room previews according to this
    /// setting.
    pub(crate) fn should_room_show_media_previews(self, room: &Room) -> bool {
        match self {
            Self::All => true,
            Self::Private => !room.join_rule().anyone_can_join(),
            Self::None => false,
        }
    }
}

impl<'de> Deserialize<'de> for MediaPreviewsGlobalSetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let cow = ruma::serde::deserialize_cow_str(deserializer)?;
        cow.parse().map_err(serde::de::Error::custom)
    }
}

/// The session list settings of the application.
fn session_list_settings() -> SessionListSettings {
    Application::default().session_list().settings()
}

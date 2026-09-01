use commune_core::settings::SessionSettings as CoreSessionSettings;
use gtk::{glib, prelude::*, subclass::prelude::*};
use indexmap::IndexSet;
use ruma::{OwnedServerName, events::media_preview_config::MediaPreviews};

use super::SidebarSectionName;

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SessionSettings)]
    pub struct SessionSettings {
        /// The settings this object is a shell around.
        pub(super) inner: OnceCell<CoreSessionSettings>,
        /// Whether notifications are enabled for this session.
        #[property(get = Self::notifications_enabled, set = Self::set_notifications_enabled, explicit_notify, default = true)]
        notifications_enabled: PhantomData<bool>,
        /// Whether public read receipts are enabled for this session.
        #[property(get = Self::public_read_receipts_enabled, set = Self::set_public_read_receipts_enabled, explicit_notify, default = true)]
        public_read_receipts_enabled: PhantomData<bool>,
        /// Whether typing notifications are enabled for this session.
        #[property(get = Self::typing_enabled, set = Self::set_typing_enabled, explicit_notify, default = true)]
        typing_enabled: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionSettings {
        const NAME: &'static str = "SessionSettings";
        type Type = super::SessionSettings;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SessionSettings {}

    impl SessionSettings {
        /// The settings this object is a shell around.
        pub(super) fn inner(&self) -> &CoreSessionSettings {
            self.inner.get().expect("settings are initialized")
        }

        /// Whether notifications are enabled for this session.
        fn notifications_enabled(&self) -> bool {
            self.inner().notifications_enabled()
        }

        /// Set whether notifications are enabled for this session.
        fn set_notifications_enabled(&self, enabled: bool) {
            if self.notifications_enabled() == enabled {
                return;
            }

            self.inner().set_notifications_enabled(enabled);
            self.obj().notify_notifications_enabled();
        }

        /// Whether public read receipts are enabled for this session.
        fn public_read_receipts_enabled(&self) -> bool {
            self.inner().public_read_receipts_enabled()
        }

        /// Set whether public read receipts are enabled for this session.
        fn set_public_read_receipts_enabled(&self, enabled: bool) {
            if self.public_read_receipts_enabled() == enabled {
                return;
            }

            self.inner().set_public_read_receipts_enabled(enabled);
            self.obj().notify_public_read_receipts_enabled();
        }

        /// Whether typing notifications are enabled for this session.
        fn typing_enabled(&self) -> bool {
            self.inner().typing_enabled()
        }

        /// Set whether typing notifications are enabled for this session.
        fn set_typing_enabled(&self, enabled: bool) {
            if self.typing_enabled() == enabled {
                return;
            }

            self.inner().set_typing_enabled(enabled);
            self.obj().notify_typing_enabled();
        }
    }
}

glib::wrapper! {
    /// The settings of a [`Session`](super::Session).
    ///
    /// The values, their JSON and their persistence are
    /// [`commune_core::settings::SessionSettings`]; this is the `GObject`
    /// shell three `.blp` rows bind to bidirectionally, and the only thing
    /// it adds is the `notify` they need.
    ///
    /// The core has no change notification for these — it does not need
    /// one. Nothing but a setter here can change a session setting, so a
    /// `notify` emitted by the setter cannot be missed, and this shell owes
    /// nothing to the `eyeball` half of the bridge.
    pub struct SessionSettings(ObjectSubclass<imp::SessionSettings>);
}

impl SessionSettings {
    /// Create a `SessionSettings` shell around the given core settings.
    pub(crate) fn new(inner: CoreSessionSettings) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp()
            .inner
            .set(inner)
            .expect("settings should not be initialized");
        obj
    }

    /// The settings this object is a shell around.
    pub(crate) fn inner(&self) -> &CoreSessionSettings {
        self.imp().inner()
    }

    /// The version of the stored settings.
    ///
    /// Only the global-account-data migration has a use for this.
    pub(crate) fn stored_version(&self) -> u8 {
        self.inner().stored_settings().version()
    }

    /// The version-0 setting about which rooms display media previews, if
    /// it is still stored.
    pub(crate) fn legacy_media_previews_enabled(&self) -> Option<MediaPreviews> {
        self.inner()
            .stored_settings()
            .legacy_media_previews_enabled()
    }

    /// The version-0 setting about whether to display avatars in invites,
    /// if it is still stored.
    pub(crate) fn legacy_invite_avatars_enabled(&self) -> Option<bool> {
        self.inner()
            .stored_settings()
            .legacy_invite_avatars_enabled()
    }

    /// Apply the migration of the stored settings from version 0 to version 1.
    pub(crate) fn apply_version_1_migration(&self) {
        self.inner().apply_version_1_migration();
    }

    /// Delete the settings from the application settings.
    pub(crate) fn delete(&self) {
        self.inner().delete();
    }

    /// Custom servers to explore.
    pub(crate) fn explore_custom_servers(&self) -> IndexSet<OwnedServerName> {
        self.inner().explore_custom_servers()
    }

    /// Set the custom servers to explore.
    pub(crate) fn set_explore_custom_servers(&self, servers: IndexSet<OwnedServerName>) {
        self.inner().set_explore_custom_servers(servers);
    }

    /// Whether the section with the given name is expanded.
    pub(crate) fn is_section_expanded(&self, section_name: SidebarSectionName) -> bool {
        self.inner().is_section_expanded(section_name.into())
    }

    /// Set whether the section with the given name is expanded.
    pub(crate) fn set_section_expanded(&self, section_name: SidebarSectionName, expanded: bool) {
        self.inner()
            .set_section_expanded(section_name.into(), expanded);
    }
}

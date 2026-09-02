pub(crate) use commune_core::session::QUICK_REACTIONS_LEN;
use commune_core::session::{
    DEFAULT_INVITE_AVATARS_ENABLED, DEFAULT_MEDIA_PREVIEWS,
    GlobalAccountData as CoreGlobalAccountData,
};
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use ruma::events::media_preview_config::MediaPreviews;
use tokio::task::AbortHandle;
use tracing::error;

use super::{Room, Session};
use crate::{core_bridge::ObjectWatcher, spawn, spawn_tokio};

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::GlobalAccountData)]
    pub struct GlobalAccountData {
        /// The session this account data belongs to.
        #[property(get, construct_only)]
        session: OnceCell<Session>,
        /// Which rooms display media previews for this session, mirrored
        /// from the core.
        pub(super) media_previews_enabled: RefCell<MediaPreviews>,
        /// Whether to display avatars in invites.
        #[property(get, default = DEFAULT_INVITE_AVATARS_ENABLED)]
        invite_avatars_enabled: Cell<bool>,
        /// The task following the core's account data.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    impl Default for GlobalAccountData {
        fn default() -> Self {
            Self {
                session: Default::default(),
                media_previews_enabled: RefCell::new(DEFAULT_MEDIA_PREVIEWS),
                invite_avatars_enabled: Cell::new(DEFAULT_INVITE_AVATARS_ENABLED),
                watch_handle: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GlobalAccountData {
        const NAME: &'static str = "GlobalAccountData";
        type Type = super::GlobalAccountData;
    }

    #[glib::derived_properties]
    impl ObjectImpl for GlobalAccountData {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("media-previews-enabled-changed").build(),
                    Signal::builder("recent-emoji-changed").build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            self.watch_core();

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.apply_migrations().await;
                }
            ));
        }

        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl GlobalAccountData {
        /// The session these settings are for.
        fn session(&self) -> &Session {
            self.session.get().expect("session should be initialized")
        }

        /// The core's account data.
        pub(super) fn core(&self) -> CoreGlobalAccountData {
            self.session().core().global_account_data().clone()
        }

        /// Follow the core's account data.
        fn watch_core(&self) {
            type A = super::GlobalAccountData;

            let core = self.core();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(
                    core.subscribe_media_previews_enabled(),
                    |obj: &A, setting| {
                        obj.imp().set_media_previews_enabled(setting);
                    },
                )
                .follow(
                    core.subscribe_invite_avatars_enabled(),
                    |obj: &A, enabled| {
                        obj.imp().set_invite_avatars_enabled(enabled);
                    },
                )
                .follow(core.subscribe_recent_emoji(), |obj: &A, _recent_emoji| {
                    obj.emit_by_name::<()>("recent-emoji-changed", &[]);
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.set_media_previews_enabled(core.media_previews_enabled());
            self.set_invite_avatars_enabled(core.invite_avatars_enabled());
        }

        /// Mirror which rooms display media previews.
        fn set_media_previews_enabled(&self, setting: MediaPreviews) {
            if *self.media_previews_enabled.borrow() == setting {
                return;
            }

            self.media_previews_enabled.replace(setting);
            self.obj()
                .emit_by_name::<()>("media-previews-enabled-changed", &[]);
        }

        /// Mirror whether to display avatars in invites.
        fn set_invite_avatars_enabled(&self, enabled: bool) {
            if self.invite_avatars_enabled.get() == enabled {
                return;
            }

            self.invite_avatars_enabled.set(enabled);
            self.obj().notify_invite_avatars_enabled();
        }

        /// Apply any necessary migrations.
        ///
        /// The stored settings are this application's, so the migration is
        /// too. It runs once the core has read the account data, so that a
        /// value the account already has is not written again.
        async fn apply_migrations(&self) {
            let session_settings = self.session().settings();

            if session_settings.stored_version() != 0 {
                // No migration to apply.
                return;
            }

            let core = self.core();
            spawn_tokio!(async move { core.ensure_loaded().await })
                .await
                .expect("task was not aborted");

            // Align the account data with the stored settings.
            let obj = self.obj();

            let stored_media_previews_enabled = session_settings
                .legacy_media_previews_enabled()
                .unwrap_or(DEFAULT_MEDIA_PREVIEWS);
            let _ = obj
                .set_media_previews_enabled(stored_media_previews_enabled)
                .await;

            let stored_invite_avatars_enabled = session_settings
                .legacy_invite_avatars_enabled()
                .unwrap_or(DEFAULT_INVITE_AVATARS_ENABLED);
            let _ = obj
                .set_invite_avatars_enabled(stored_invite_avatars_enabled)
                .await;

            session_settings.apply_version_1_migration();
        }
    }
}

glib::wrapper! {
    /// The settings in the global account data of a [`Session`].
    ///
    /// The settings are the core's; this presents them.
    pub struct GlobalAccountData(ObjectSubclass<imp::GlobalAccountData>);
}

impl GlobalAccountData {
    /// Create a new `GlobalAccountData` for the given session.
    pub(crate) fn new(session: &Session) -> Self {
        glib::Object::builder::<Self>()
            .property("session", session)
            .build()
    }

    /// Which rooms display media previews.
    pub(crate) fn media_previews_enabled(&self) -> MediaPreviews {
        self.imp().media_previews_enabled.borrow().clone()
    }

    /// Whether the given room should display media previews.
    pub(crate) fn should_room_show_media_previews(&self, room: &Room) -> bool {
        self.imp()
            .core()
            .should_room_show_media_previews(room.core())
    }

    /// Set which rooms display media previews.
    pub(crate) async fn set_media_previews_enabled(
        &self,
        setting: MediaPreviews,
    ) -> Result<(), ()> {
        let core = self.imp().core();
        let handle = spawn_tokio!(async move { core.set_media_previews_enabled(setting).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not change media previews enabled setting: {error}");
            })
    }

    /// Set whether to display avatars in invites.
    pub(crate) async fn set_invite_avatars_enabled(&self, enabled: bool) -> Result<(), ()> {
        let core = self.imp().core();
        let handle = spawn_tokio!(async move { core.set_invite_avatars_enabled(enabled).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not change invite avatars enabled setting: {error}");
            })
    }

    /// The quick reactions of an account that has never reacted.
    pub(crate) fn default_quick_reactions() -> Vec<String> {
        CoreGlobalAccountData::default_quick_reactions()
    }

    /// The emoji to present as quick reactions, most used first.
    pub(crate) fn quick_reactions(&self) -> Vec<String> {
        self.imp().core().quick_reactions()
    }

    /// Record that the given emoji was used.
    pub(crate) async fn record_emoji_use(&self, emoji: &str) {
        let core = self.imp().core();
        let emoji = emoji.to_owned();

        spawn_tokio!(async move { core.record_emoji_use(&emoji).await })
            .await
            .expect("task was not aborted");
    }

    /// Connect to the signal emitted when the recently used emoji changed.
    pub(crate) fn connect_recent_emoji_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "recent-emoji-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
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

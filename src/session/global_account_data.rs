use std::collections::HashSet;

use futures_util::StreamExt;
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::event_handler::EventHandlerDropGuard;
use ruma::events::{
    media_preview_config::{InviteAvatars, MediaPreviewConfigEventContent, MediaPreviews},
    recent_emoji::{RecentEmoji, RecentEmojiEvent, RecentEmojiEventContent},
};
use tokio::task::AbortHandle;
use tracing::error;

use super::{Room, Session};
use crate::{session::JoinRuleValue, spawn, spawn_tokio};

/// We default the media previews setting to private.
const DEFAULT_MEDIA_PREVIEWS: MediaPreviews = MediaPreviews::Private;
/// We enable the invite avatars by default.
const DEFAULT_INVITE_AVATARS_ENABLED: bool = true;

/// The number of quick reactions presented in the message context menu.
pub(crate) const QUICK_REACTIONS_LEN: usize = 7;
/// The quick reactions of an account that has never reacted.
///
/// They also fill out the list until enough emoji have been used.
const DEFAULT_QUICK_REACTIONS: &[&str] = &[
    "\u{1F44D}\u{FE0F}",
    "\u{1F44E}\u{FE0F}",
    "\u{1F604}",
    "\u{1F389}",
    "\u{1F615}",
    "\u{2764}\u{FE0F}",
    "\u{1F680}",
];

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
        /// Which rooms display media previews for this session.
        pub(super) media_previews_enabled: RefCell<MediaPreviews>,
        /// Whether to display avatars in invites.
        #[property(get, default = DEFAULT_INVITE_AVATARS_ENABLED)]
        invite_avatars_enabled: Cell<bool>,
        /// The emoji this account has reacted with, in the order the account
        /// data lists them.
        pub(super) recent_emoji: RefCell<Vec<RecentEmoji>>,
        abort_handle: RefCell<Option<AbortHandle>>,
        recent_emoji_drop_guard: RefCell<Option<EventHandlerDropGuard>>,
    }

    impl Default for GlobalAccountData {
        fn default() -> Self {
            Self {
                session: Default::default(),
                media_previews_enabled: RefCell::new(DEFAULT_MEDIA_PREVIEWS),
                invite_avatars_enabled: Cell::new(DEFAULT_INVITE_AVATARS_ENABLED),
                recent_emoji: Default::default(),
                abort_handle: Default::default(),
                recent_emoji_drop_guard: Default::default(),
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

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.init_media_previews_settings().await;
                    imp.init_recent_emoji().await;
                    imp.apply_migrations().await;
                }
            ));
        }

        fn dispose(&self) {
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }

            self.recent_emoji_drop_guard.take();
        }
    }

    impl GlobalAccountData {
        /// The session these settings are for.
        fn session(&self) -> &Session {
            self.session.get().expect("session should be initialized")
        }

        /// Initialize the media previews settings from the account data and
        /// watch for changes.
        pub(super) async fn init_media_previews_settings(&self) {
            let client = self.session().client();
            let handle =
                spawn_tokio!(async move { client.account().observe_media_preview_config().await });

            let (account_data, stream) = match handle.await.expect("task was not aborted") {
                Ok((account_data, stream)) => (account_data, stream),
                Err(error) => {
                    error!("Could not initialize media preview settings: {error}");
                    return;
                }
            };

            self.update_media_previews_settings(account_data.unwrap_or_default());

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let fut = stream.for_each(move |account_data| {
                let obj_weak = obj_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.imp().update_media_previews_settings(account_data);
                            }
                        });
                    });
                }
            });

            let abort_handle = spawn_tokio!(fut).abort_handle();
            self.abort_handle.replace(Some(abort_handle));
        }

        /// Update the media previews settings with the given account data.
        fn update_media_previews_settings(&self, account_data: MediaPreviewConfigEventContent) {
            let media_previews = account_data
                .media_previews
                .unwrap_or(DEFAULT_MEDIA_PREVIEWS);
            let media_previews_enabled_changed =
                *self.media_previews_enabled.borrow() != media_previews;
            if media_previews_enabled_changed {
                *self.media_previews_enabled.borrow_mut() = media_previews;
                self.obj()
                    .emit_by_name::<()>("media-previews-enabled-changed", &[]);
            }

            let invite_avatars_enabled = account_data
                .invite_avatars
                .map_or(DEFAULT_INVITE_AVATARS_ENABLED, |invite_avatars| {
                    invite_avatars == InviteAvatars::On
                });
            let invite_avatars_enabled_changed =
                self.invite_avatars_enabled.get() != invite_avatars_enabled;
            if invite_avatars_enabled_changed {
                self.invite_avatars_enabled.set(invite_avatars_enabled);
                self.obj().notify_invite_avatars_enabled();
            }
        }

        /// Initialize the recently used emoji from the account data and watch
        /// for changes.
        pub(super) async fn init_recent_emoji(&self) {
            let client = self.session().client();

            let client_clone = client.clone();
            let handle = spawn_tokio!(async move {
                client_clone
                    .account()
                    .account_data::<RecentEmojiEventContent>()
                    .await
            });

            match handle.await.expect("task was not aborted") {
                Ok(Some(raw)) => match raw.deserialize() {
                    Ok(content) => self.update_recent_emoji(content.recent_emoji),
                    Err(error) => error!("Could not deserialize recent emoji: {error}"),
                },
                Ok(None) => {}
                Err(error) => error!("Could not get recent emoji: {error}"),
            }

            // Another client on the account reorders this list as it is used
            // there, and the list is the same list.
            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let handle = client.add_event_handler(move |event: RecentEmojiEvent| {
                let obj_weak = obj_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.imp().update_recent_emoji(event.content.recent_emoji);
                            }
                        });
                    });
                }
            });

            let drop_guard = client.event_handler_drop_guard(handle);
            self.recent_emoji_drop_guard.replace(Some(drop_guard));
        }

        /// Update the recently used emoji with the given list.
        fn update_recent_emoji(&self, recent_emoji: Vec<RecentEmoji>) {
            if *self.recent_emoji.borrow() == recent_emoji {
                return;
            }

            self.recent_emoji.replace(recent_emoji);
            self.obj().emit_by_name::<()>("recent-emoji-changed", &[]);
        }

        /// Record that the given emoji was used.
        pub(super) async fn record_emoji_use(&self, emoji: &str) {
            if !is_emoji(emoji) {
                return;
            }

            let mut content = RecentEmojiEventContent::new(self.recent_emoji.borrow().clone());
            content.increment_emoji_total(emoji);

            // Show it straight away rather than waiting for the round trip.
            self.update_recent_emoji(content.recent_emoji.clone());

            let client = self.session().client();
            let handle =
                spawn_tokio!(async move { client.account().set_account_data(content).await });

            if let Err(error) = handle.await.expect("task was not aborted") {
                // The list is a convenience, so a failure here is not worth
                // telling the user about. The next use tries again.
                error!("Could not save recent emoji: {error}");
            }
        }

        /// Apply any necessary migrations.
        pub(super) async fn apply_migrations(&self) {
            let session_settings = self.session().settings();

            if session_settings.stored_version() != 0 {
                // No migration to apply.
                return;
            }

            // Align the account data with the stored settings.
            let stored_media_previews_enabled = session_settings
                .legacy_media_previews_enabled()
                .unwrap_or(DEFAULT_MEDIA_PREVIEWS);
            let _ = self
                .set_media_previews_enabled(stored_media_previews_enabled)
                .await;

            let stored_invite_avatars_enabled = session_settings
                .legacy_invite_avatars_enabled()
                .unwrap_or(DEFAULT_INVITE_AVATARS_ENABLED);
            let _ = self
                .set_invite_avatars_enabled(stored_invite_avatars_enabled)
                .await;

            session_settings.apply_version_1_migration();
        }

        /// Set which rooms display media previews.
        pub(super) async fn set_media_previews_enabled(
            &self,
            setting: MediaPreviews,
        ) -> Result<(), ()> {
            if *self.media_previews_enabled.borrow() == setting {
                return Ok(());
            }

            let client = self.session().client();
            let setting_clone = setting.clone();
            let handle = spawn_tokio!(async move {
                client
                    .account()
                    .set_media_previews_display_policy(setting_clone)
                    .await
            });

            if let Err(error) = handle.await.expect("task was not aborted") {
                error!("Could not change media previews enabled setting: {error}");
                return Err(());
            }

            self.media_previews_enabled.replace(setting);

            self.obj()
                .emit_by_name::<()>("media-previews-enabled-changed", &[]);

            Ok(())
        }

        /// Set whether to display avatars in invites.
        pub(super) async fn set_invite_avatars_enabled(&self, enabled: bool) -> Result<(), ()> {
            if self.invite_avatars_enabled.get() == enabled {
                return Ok(());
            }

            let client = self.session().client();
            let setting = if enabled {
                InviteAvatars::On
            } else {
                InviteAvatars::Off
            };
            let handle = spawn_tokio!(async move {
                client
                    .account()
                    .set_invite_avatars_display_policy(setting)
                    .await
            });

            if let Err(error) = handle.await.expect("task was not aborted") {
                error!("Could not change invite avatars enabled setting: {error}");
                return Err(());
            }

            self.invite_avatars_enabled.set(enabled);
            self.obj().notify_invite_avatars_enabled();

            Ok(())
        }
    }
}

glib::wrapper! {
    /// The settings in the global account data of a [`Session`].
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
        match &*self.imp().media_previews_enabled.borrow() {
            MediaPreviews::Off => false,
            MediaPreviews::Private => matches!(
                room.join_rule().value(),
                JoinRuleValue::Invite | JoinRuleValue::RoomMembership
            ),
            _ => true,
        }
    }

    /// Set which rooms display media previews.
    pub(crate) async fn set_media_previews_enabled(
        &self,
        setting: MediaPreviews,
    ) -> Result<(), ()> {
        self.imp().set_media_previews_enabled(setting).await
    }

    /// Set whether to display avatars in invites.
    pub(crate) async fn set_invite_avatars_enabled(&self, enabled: bool) -> Result<(), ()> {
        self.imp().set_invite_avatars_enabled(enabled).await
    }

    /// The quick reactions of an account that has never reacted.
    pub(crate) fn default_quick_reactions() -> Vec<String> {
        DEFAULT_QUICK_REACTIONS
            .iter()
            .map(|emoji| (*emoji).to_owned())
            .collect()
    }

    /// The emoji to present as quick reactions, most used first.
    pub(crate) fn quick_reactions(&self) -> Vec<String> {
        quick_reactions_from(&self.imp().recent_emoji.borrow())
    }

    /// Record that the given emoji was used.
    pub(crate) async fn record_emoji_use(&self, emoji: &str) {
        self.imp().record_emoji_use(emoji).await;
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

/// The emoji to present as quick reactions, given the recently used emoji.
///
/// The list is sorted by number of uses rather than by recency, so that the
/// buttons stay where they were long enough to be aimed at, and is filled out
/// with the defaults until enough emoji have been used.
fn quick_reactions_from(recent_emoji: &[RecentEmoji]) -> Vec<String> {
    let content = RecentEmojiEventContent::new(recent_emoji.to_owned());

    let mut reactions = Vec::with_capacity(QUICK_REACTIONS_LEN);
    let mut keys = HashSet::with_capacity(QUICK_REACTIONS_LEN);

    for emoji in content.recent_emoji_sorted_by_total() {
        if reactions.len() == QUICK_REACTIONS_LEN {
            break;
        }

        if keys.insert(emoji_key(&emoji.emoji)) {
            reactions.push(emoji.emoji);
        }
    }

    for emoji in DEFAULT_QUICK_REACTIONS {
        if reactions.len() == QUICK_REACTIONS_LEN {
            break;
        }

        if keys.insert(emoji_key(emoji)) {
            reactions.push((*emoji).to_owned());
        }
    }

    reactions
}

/// Whether the given reaction key is an emoji worth remembering.
///
/// A reaction key is any string and clients do send words, but
/// `m.recent_emoji` is a list of emoji. Letters are the giveaway: no emoji
/// carries one, while a word or an `mxc:` URI is nothing but letters. Digits
/// are left alone, because the keycaps are built out of them.
fn is_emoji(key: &str) -> bool {
    !key.is_empty() && !key.chars().any(|c| c.is_ascii_alphabetic())
}

/// The key under which two spellings of the same emoji are the same emoji.
///
/// A thumbs up sent with the emoji presentation selector and one sent without
/// it look identical, so presenting both would look like a bug.
fn emoji_key(emoji: &str) -> String {
    emoji.chars().filter(|c| *c != '\u{FE0F}').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a list of recent emoji from pairs of emoji and total.
    fn recent(emoji: &[(&str, u32)]) -> Vec<RecentEmoji> {
        emoji
            .iter()
            .map(|(emoji, total)| {
                let mut recent = RecentEmoji::new((*emoji).to_owned());
                recent.total = (*total).into();
                recent
            })
            .collect()
    }

    #[test]
    fn an_account_that_never_reacted_gets_the_defaults() {
        assert_eq!(quick_reactions_from(&[]), DEFAULT_QUICK_REACTIONS);
    }

    #[test]
    fn the_most_used_come_first_and_the_defaults_fill_the_rest() {
        let reactions = quick_reactions_from(&recent(&[("\u{1F602}", 3), ("\u{1F525}", 12)]));

        assert_eq!(reactions.len(), QUICK_REACTIONS_LEN);
        assert_eq!(&reactions[..2], &["\u{1F525}", "\u{1F602}"]);
        // The rest are defaults, in their own order.
        assert_eq!(&reactions[2..5], &DEFAULT_QUICK_REACTIONS[..3]);
    }

    #[test]
    fn a_used_default_is_not_presented_twice() {
        // The thumbs up is both a default and, here, the most used emoji.
        let reactions = quick_reactions_from(&recent(&[("\u{1F44D}\u{FE0F}", 20)]));

        assert_eq!(reactions.len(), QUICK_REACTIONS_LEN);
        assert_eq!(
            reactions
                .iter()
                .filter(|e| emoji_key(e) == "\u{1F44D}")
                .count(),
            1
        );
    }

    #[test]
    fn the_two_spellings_of_an_emoji_are_one_emoji() {
        // Reacted without the presentation selector, which is how most clients
        // send it; the default carries one.
        let reactions = quick_reactions_from(&recent(&[("\u{1F44D}", 20)]));

        assert_eq!(reactions.len(), QUICK_REACTIONS_LEN);
        assert_eq!(
            reactions
                .iter()
                .filter(|e| emoji_key(e) == "\u{1F44D}")
                .count(),
            1
        );
        // The spelling the account actually uses is the one presented.
        assert_eq!(reactions[0], "\u{1F44D}");
    }

    #[test]
    fn enough_used_emoji_leave_no_room_for_a_default() {
        let used = recent(&[
            ("a", 8),
            ("b", 7),
            ("c", 6),
            ("d", 5),
            ("e", 4),
            ("f", 3),
            ("g", 2),
            ("h", 1),
        ]);
        let reactions = quick_reactions_from(&used);

        assert_eq!(reactions, ["a", "b", "c", "d", "e", "f", "g"]);
    }

    #[test]
    fn a_word_is_not_an_emoji() {
        // Some clients let you react with arbitrary text.
        assert!(!is_emoji("lol"));
        assert!(!is_emoji("mxc://example.org/abcdef"));
        assert!(!is_emoji(""));
    }

    #[test]
    fn a_keycap_is_an_emoji() {
        // Built out of a digit, which is why the check is for letters only.
        assert!(is_emoji("1\u{FE0F}\u{20E3}"));
        assert!(is_emoji("\u{1F44D}"));
    }

    #[test]
    fn a_tie_is_broken_by_the_order_of_the_list() {
        // The event stores the list most recently used first, so of two emoji
        // used as often, the one used later wins.
        let reactions = quick_reactions_from(&recent(&[("\u{1F440}", 2), ("\u{1F64F}", 2)]));

        assert_eq!(&reactions[..2], &["\u{1F440}", "\u{1F64F}"]);
    }
}

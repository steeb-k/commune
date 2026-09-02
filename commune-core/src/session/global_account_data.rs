//! The settings in the global account data of a session, headless.
//!
//! The application's `session/global_account_data.rs` with the `GObject`
//! removed: which rooms show media previews and whether invites show
//! avatars, followed through the SDK's media-preview stream; the emoji the
//! account reacted with, followed through `io.element.recent_emoji`; and
//! the quick reactions made of them, most used first and filled out with
//! the defaults. Nothing here is a sentence: the defaults are emoji.
//!
//! What stayed with the application is the migration from its own stored
//! settings to the account data, since the stored settings are its own.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use eyeball::{SharedObservable, Subscriber};
use futures_util::StreamExt;
use matrix_sdk::event_handler::EventHandlerDropGuard;
use ruma::events::{
    media_preview_config::{InviteAvatars, MediaPreviewConfigEventContent, MediaPreviews},
    recent_emoji::{RecentEmoji, RecentEmojiEvent, RecentEmojiEventContent},
};
use tokio::task::AbortHandle;
use tracing::error;

use super::{JoinRuleValue, Room, WeakSession};
use crate::{RUNTIME, UserFacingError, spawn_tokio, utils::LoadingState};

/// Which rooms show media previews when the account data says nothing.
///
/// Private: the specification leaves the default to the client, and this
/// one errs on the side of not loading what a stranger sent.
pub const DEFAULT_MEDIA_PREVIEWS: MediaPreviews = MediaPreviews::Private;
/// Whether invites show avatars when the account data says nothing.
pub const DEFAULT_INVITE_AVATARS_ENABLED: bool = true;

/// The number of quick reactions presented in the message context menu.
pub const QUICK_REACTIONS_LEN: usize = 7;
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

/// What can go wrong while changing the account data.
#[derive(Debug, thiserror::Error)]
pub enum AccountDataError {
    /// The session this account data belongs to is gone.
    #[error("the session is no longer available")]
    NoSession,
    /// The homeserver refused the change.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl UserFacingError for AccountDataError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NoSession => "The session is no longer available.".to_owned(),
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// The settings in the global account data of a session.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct GlobalAccountData {
    inner: Arc<GlobalAccountDataInner>,
}

#[derive(Debug)]
struct GlobalAccountDataInner {
    /// The session this account data belongs to.
    session: WeakSession,
    /// Which rooms display media previews.
    media_previews_enabled: SharedObservable<MediaPreviews>,
    /// Whether to display avatars in invites.
    invite_avatars_enabled: SharedObservable<bool>,
    /// The emoji this account has reacted with, in the order the account
    /// data lists them.
    recent_emoji: SharedObservable<Vec<RecentEmoji>>,
    /// How far the first read has got.
    state: SharedObservable<LoadingState>,
    /// The task following the media preview settings.
    watch_handle: Mutex<Option<AbortHandle>>,
    /// The SDK event handler following the recent emoji.
    recent_emoji_drop_guard: Mutex<Option<EventHandlerDropGuard>>,
}

impl Drop for GlobalAccountDataInner {
    fn drop(&mut self) {
        if let Ok(Some(handle)) = self.watch_handle.get_mut().map(Option::take) {
            handle.abort();
        }
    }
}

impl GlobalAccountData {
    /// Create the global account data of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        Self {
            inner: Arc::new(GlobalAccountDataInner {
                session,
                media_previews_enabled: SharedObservable::new(DEFAULT_MEDIA_PREVIEWS),
                invite_avatars_enabled: SharedObservable::new(DEFAULT_INVITE_AVATARS_ENABLED),
                recent_emoji: SharedObservable::new(Vec::new()),
                state: SharedObservable::new(LoadingState::Initial),
                watch_handle: Mutex::new(None),
                recent_emoji_drop_guard: Mutex::new(None),
            }),
        }
    }

    /// Read the account data, and follow it from there.
    pub async fn load(&self) {
        self.inner.state.set_if_not_eq(LoadingState::Loading);
        self.init_media_previews().await;
        self.init_recent_emoji().await;
        self.inner.state.set_if_not_eq(LoadingState::Ready);
    }

    /// Read the account data unless it has already been read.
    pub async fn ensure_loaded(&self) {
        if self.inner.state.get() == LoadingState::Ready {
            return;
        }
        self.load().await;
    }

    /// How far the first read has got.
    #[must_use]
    pub fn state(&self) -> LoadingState {
        self.inner.state.get()
    }

    /// Subscribe to how far the first read has got.
    pub fn subscribe_state(&self) -> Subscriber<LoadingState> {
        self.inner.state.subscribe()
    }

    /// Which rooms display media previews.
    #[must_use]
    pub fn media_previews_enabled(&self) -> MediaPreviews {
        self.inner.media_previews_enabled.get()
    }

    /// Subscribe to which rooms display media previews.
    pub fn subscribe_media_previews_enabled(&self) -> Subscriber<MediaPreviews> {
        self.inner.media_previews_enabled.subscribe()
    }

    /// Whether to display avatars in invites.
    #[must_use]
    pub fn invite_avatars_enabled(&self) -> bool {
        self.inner.invite_avatars_enabled.get()
    }

    /// Subscribe to whether to display avatars in invites.
    pub fn subscribe_invite_avatars_enabled(&self) -> Subscriber<bool> {
        self.inner.invite_avatars_enabled.subscribe()
    }

    /// The emoji this account has reacted with, in the order the account
    /// data lists them.
    #[must_use]
    pub fn recent_emoji(&self) -> Vec<RecentEmoji> {
        self.inner.recent_emoji.get()
    }

    /// Subscribe to the emoji this account has reacted with.
    pub fn subscribe_recent_emoji(&self) -> Subscriber<Vec<RecentEmoji>> {
        self.inner.recent_emoji.subscribe()
    }

    /// Whether the given room should display media previews.
    #[must_use]
    pub fn should_room_show_media_previews(&self, room: &Room) -> bool {
        match self.media_previews_enabled() {
            MediaPreviews::Off => false,
            MediaPreviews::Private => matches!(
                room.join_rule().state().value,
                JoinRuleValue::Invite | JoinRuleValue::RoomMembership
            ),
            _ => true,
        }
    }

    /// Set which rooms display media previews.
    pub async fn set_media_previews_enabled(
        &self,
        setting: MediaPreviews,
    ) -> Result<(), AccountDataError> {
        if self.media_previews_enabled() == setting {
            return Ok(());
        }

        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(AccountDataError::NoSession)?;

        let client = session.client();
        let setting_clone = setting.clone();
        let handle = spawn_tokio!(async move {
            client
                .account()
                .set_media_previews_display_policy(setting_clone)
                .await
        });

        if let Err(set_error) = handle.await.expect("task was not aborted") {
            error!("Could not change media previews enabled setting: {set_error}");
            return Err(AccountDataError::Server(Box::new(set_error)));
        }

        self.inner.media_previews_enabled.set_if_not_eq(setting);

        Ok(())
    }

    /// Set whether to display avatars in invites.
    pub async fn set_invite_avatars_enabled(&self, enabled: bool) -> Result<(), AccountDataError> {
        if self.invite_avatars_enabled() == enabled {
            return Ok(());
        }

        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(AccountDataError::NoSession)?;

        let client = session.client();
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

        if let Err(set_error) = handle.await.expect("task was not aborted") {
            error!("Could not change invite avatars enabled setting: {set_error}");
            return Err(AccountDataError::Server(Box::new(set_error)));
        }

        self.inner.invite_avatars_enabled.set_if_not_eq(enabled);

        Ok(())
    }

    /// The quick reactions of an account that has never reacted.
    #[must_use]
    pub fn default_quick_reactions() -> Vec<String> {
        DEFAULT_QUICK_REACTIONS
            .iter()
            .map(|emoji| (*emoji).to_owned())
            .collect()
    }

    /// The emoji to present as quick reactions, most used first.
    #[must_use]
    pub fn quick_reactions(&self) -> Vec<String> {
        quick_reactions_from(&self.recent_emoji())
    }

    /// Record that the given emoji was used.
    ///
    /// The list is a convenience, so a failure to save it is logged and not
    /// returned: the next use tries again.
    pub async fn record_emoji_use(&self, emoji: &str) {
        if !is_emoji(emoji) {
            return;
        }

        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        let mut content = RecentEmojiEventContent::new(self.recent_emoji());
        content.increment_emoji_total(emoji);

        // Show it straight away rather than waiting for the round trip.
        self.update_recent_emoji(content.recent_emoji.clone());

        let client = session.client();
        let handle = spawn_tokio!(async move { client.account().set_account_data(content).await });

        if let Err(save_error) = handle.await.expect("task was not aborted") {
            error!("Could not save recent emoji: {save_error}");
        }
    }

    /// Read the media preview settings from the account data and follow
    /// them.
    async fn init_media_previews(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        let client = session.client();
        let handle =
            spawn_tokio!(async move { client.account().observe_media_preview_config().await });

        let (account_data, stream) = match handle.await.expect("task was not aborted") {
            Ok((account_data, stream)) => (account_data, stream),
            Err(observe_error) => {
                error!("Could not initialize media preview settings: {observe_error}");
                return;
            }
        };

        self.update_media_previews(&account_data.unwrap_or_default());

        let weak = Arc::downgrade(&self.inner);
        let fut = stream.for_each(move |account_data| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade() {
                    GlobalAccountData { inner }.update_media_previews(&account_data);
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
    }

    /// Update the media preview settings with the given account data.
    fn update_media_previews(&self, account_data: &MediaPreviewConfigEventContent) {
        let media_previews = account_data
            .media_previews
            .clone()
            .unwrap_or(DEFAULT_MEDIA_PREVIEWS);
        self.inner
            .media_previews_enabled
            .set_if_not_eq(media_previews);

        let invite_avatars_enabled = account_data
            .invite_avatars
            .as_ref()
            .map_or(DEFAULT_INVITE_AVATARS_ENABLED, |invite_avatars| {
                *invite_avatars == InviteAvatars::On
            });
        self.inner
            .invite_avatars_enabled
            .set_if_not_eq(invite_avatars_enabled);
    }

    /// Read the recently used emoji from the account data and follow them.
    async fn init_recent_emoji(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        let client = session.client();

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
                Err(deserialize_error) => {
                    error!("Could not deserialize recent emoji: {deserialize_error}");
                }
            },
            Ok(None) => {}
            Err(load_error) => error!("Could not get recent emoji: {load_error}"),
        }

        // Another client on the account reorders this list as it is used
        // there, and the list is the same list.
        let weak = Arc::downgrade(&self.inner);
        let handle = client.add_event_handler(move |event: RecentEmojiEvent| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade() {
                    GlobalAccountData { inner }.update_recent_emoji(event.content.recent_emoji);
                }
            }
        });

        let drop_guard = client.event_handler_drop_guard(handle);
        *self
            .inner
            .recent_emoji_drop_guard
            .lock()
            .expect("mutex is not poisoned") = Some(drop_guard);
    }

    /// Update the recently used emoji with the given list.
    fn update_recent_emoji(&self, recent_emoji: Vec<RecentEmoji>) {
        self.inner.recent_emoji.set_if_not_eq(recent_emoji);
    }
}

/// Start loading the account data of the given session on the runtime, for
/// a `Session` accessor that cannot await.
pub(crate) fn spawn_load(account_data: &GlobalAccountData) {
    let account_data = account_data.clone();
    RUNTIME.spawn(async move {
        account_data.load().await;
    });
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

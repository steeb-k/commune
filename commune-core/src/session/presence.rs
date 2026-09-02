//! What the homeserver has said about who is around, headless.
//!
//! The application's `session/presence.rs` with the `GObject` removed: the
//! presence events sync carries, kept by user; the store read for a user
//! who has not moved since this client started; and telling the homeserver
//! whether we are here. The setting behind that last one is the
//! application's — a `GSettings` key — so the core only takes the answer.
//!
//! The application announced a change with a signal carrying the user's ID
//! and had the user read the list again. The core keeps one observable per
//! user instead, since a subscriber of an observable always sees the latest
//! value and a channel of IDs could fall behind a burst of presence and
//! lose one. What is known about a user is `None` until sync or the store
//! says something, and that is what lets the store read know it is needed.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use eyeball::{SharedObservable, Subscriber};
use matrix_sdk::event_handler::EventHandlerDropGuard;
use ruma::{
    OwnedUserId, UserId,
    events::presence::{PresenceEvent, PresenceEventContent},
    presence::PresenceState,
};
use tracing::error;

use super::WeakSession;
use crate::{UserFacingError, spawn_tokio};

/// What a homeserver says about whether a user is around.
///
/// The Presence module is optional and most homeservers ship it switched off,
/// so [`Self::Unknown`] is both the default and the common case. It is kept
/// apart from [`Self::Offline`] on purpose: a server with presence enabled
/// says "offline" out loud, and a server without it says nothing at all. The
/// two look the same in the interface and must not be confused in the model.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy)]
pub enum Presence {
    /// The homeserver has told us nothing about this user.
    #[default]
    Unknown,
    /// The user is around.
    Online,
    /// The user is idle.
    Unavailable,
    /// The user is away.
    Offline,
}

impl Presence {
    /// Whether this is worth drawing a badge for.
    ///
    /// Only being around and being idle are. Offline and unknown both mean
    /// "not known to be here", and drawing them apart would ask people to tell
    /// a server that keeps presence from a server that does not.
    #[must_use]
    pub fn is_visible(self) -> bool {
        matches!(self, Self::Online | Self::Unavailable)
    }
}

impl From<&PresenceState> for Presence {
    fn from(state: &PresenceState) -> Self {
        match state {
            PresenceState::Online => Self::Online,
            PresenceState::Unavailable => Self::Unavailable,
            PresenceState::Offline => Self::Offline,
            // The state is non-exhaustive: a state we do not know is not a
            // state we can draw.
            _ => Self::Unknown,
        }
    }
}

/// What is known about one user's presence.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UserPresence {
    /// Whether the user is around.
    pub presence: Presence,
    /// The message the user set to go with it, if any.
    pub status_message: Option<String>,
    /// How long ago the user last did something, in milliseconds.
    ///
    /// Only meaningful together with `currently_active`, and only present when
    /// the homeserver chooses to send it.
    pub last_active_ago: Option<u64>,
    /// Whether the user is currently active.
    pub currently_active: bool,
}

impl From<&PresenceEventContent> for UserPresence {
    fn from(content: &PresenceEventContent) -> Self {
        Self {
            presence: (&content.presence).into(),
            status_message: content
                .status_msg
                .as_ref()
                .map(|message| message.trim().to_owned())
                .filter(|message| !message.is_empty()),
            last_active_ago: content.last_active_ago.map(Into::into),
            currently_active: content.currently_active.unwrap_or_default(),
        }
    }
}

/// What can go wrong while telling the homeserver whether we are here.
#[derive(Debug, thiserror::Error)]
pub enum PresenceError {
    /// The session this list belongs to is gone.
    #[error("the session is no longer available")]
    NoSession,
    /// The homeserver refused.
    ///
    /// A homeserver without the Presence module does, and that is the
    /// common case rather than a fault. Boxed because `matrix_sdk::Error`
    /// is large enough that carrying it by value makes every `Result` in
    /// this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl UserFacingError for PresenceError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NoSession => "The session is no longer available.".to_owned(),
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// What the homeserver has said about who is around.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct PresenceList {
    inner: Arc<PresenceListInner>,
}

#[derive(Debug)]
struct PresenceListInner {
    /// The session this list belongs to.
    session: WeakSession,
    /// What is known about each user, by user ID.
    ///
    /// `None` until sync or the store said something about the user; an
    /// entry exists as soon as someone subscribes to it.
    presences: Mutex<HashMap<OwnedUserId, SharedObservable<Option<UserPresence>>>>,
    /// The SDK event handler following presence in sync.
    drop_guard: Mutex<Option<EventHandlerDropGuard>>,
}

impl PresenceList {
    /// Create the presence list of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        Self {
            inner: Arc::new(PresenceListInner {
                session,
                presences: Mutex::new(HashMap::new()),
                drop_guard: Mutex::new(None),
            }),
        }
    }

    /// Listen for presence arriving in sync.
    ///
    /// Presence is a top-level field of the sync response rather than
    /// anything belonging to a room, and this client uses classic sync, so
    /// an event handler is the whole of the live half. Installing it twice
    /// does nothing.
    pub fn watch(&self) {
        let mut guard = self.inner.drop_guard.lock().expect("mutex is not poisoned");
        if guard.is_some() {
            return;
        }

        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        let client = session.client();

        let weak = Arc::downgrade(&self.inner);
        let handle = client.add_event_handler(move |event: PresenceEvent| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade() {
                    let presence = UserPresence::from(&event.content);
                    PresenceList { inner }.update(event.sender, presence);
                }
            }
        });

        *guard = Some(client.event_handler_drop_guard(handle));
    }

    /// Tell the homeserver whether we are here.
    ///
    /// This is not new behaviour being switched on — it is behaviour being
    /// made switchable. The `set_presence` parameter of `/sync` defaults to
    /// `online` when a client omits it, and the SDK's client-owned value
    /// defaults to `Online` too, so every sync this client has ever made has
    /// told the homeserver we are here. The application's setting defaults
    /// to on for that reason: turning it off by default would quietly change
    /// what other people see, which is not ours to do. The switch is the off
    /// that did not exist before.
    ///
    /// Sent at once rather than with the next sync, so the change is visible
    /// without waiting, and because the setting being flipped is somebody
    /// asking for exactly that.
    pub async fn share_own_presence(&self, share: bool) -> Result<(), PresenceError> {
        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(PresenceError::NoSession)?;

        let presence = if share {
            PresenceState::Online
        } else {
            PresenceState::Offline
        };

        let client = session.client();
        let handle = spawn_tokio!(async move { client.set_presence(presence, None, true).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|set_error| PresenceError::Server(Box::new(set_error)))
    }

    /// What is known about the presence of the user with the given ID.
    #[must_use]
    pub fn get(&self, user_id: &UserId) -> UserPresence {
        let presences = self.inner.presences.lock().expect("mutex is not poisoned");

        match presences.get(user_id) {
            Some(known) => known.get().unwrap_or_default(),
            None => UserPresence::default(),
        }
    }

    /// Subscribe to what is known about the presence of the user with the
    /// given ID.
    ///
    /// `None` until sync or the store has said something about them.
    pub fn subscribe(&self, user_id: &UserId) -> Subscriber<Option<UserPresence>> {
        self.inner
            .presences
            .lock()
            .expect("mutex is not poisoned")
            .entry(user_id.to_owned())
            .or_insert_with(|| SharedObservable::new(None))
            .subscribe()
    }

    /// Load what the store already knows about the given user.
    ///
    /// Sync only sends presence when it changes, so a user we have never seen
    /// change would have no presence at all until they did something. The
    /// store carries what earlier syncs delivered. Nothing is read for a user
    /// something is already known about.
    pub async fn load(&self, user_id: &UserId) {
        if self
            .inner
            .presences
            .lock()
            .expect("mutex is not poisoned")
            .get(user_id)
            .is_some_and(|known| known.get().is_some())
        {
            return;
        }
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        let client = session.client();
        let user_id = user_id.to_owned();
        let user_id_clone = user_id.clone();
        let handle = spawn_tokio!(async move {
            client
                .state_store()
                .get_presence_event(&user_id_clone)
                .await
        });

        let raw = match handle.await.expect("task was not aborted") {
            Ok(Some(raw)) => raw,
            // A homeserver with presence switched off never sends any, which is
            // the common case and not worth a line in the log.
            Ok(None) => return,
            Err(read_error) => {
                error!("Could not read the stored presence of {user_id}: {read_error}");
                return;
            }
        };

        match raw.deserialize() {
            Ok(event) => self.update(user_id, UserPresence::from(&event.content)),
            Err(deserialize_error) => {
                error!(
                    "Could not deserialize the stored presence of {user_id}: {deserialize_error}"
                );
            }
        }
    }

    /// Record what is known about the given user.
    ///
    /// A subscriber only hears about it when it changed.
    fn update(&self, user_id: OwnedUserId, presence: UserPresence) {
        self.inner
            .presences
            .lock()
            .expect("mutex is not poisoned")
            .entry(user_id)
            .or_insert_with(|| SharedObservable::new(None))
            .set_if_not_eq(Some(presence));
    }
}

//! The account's ignored users, headless.
//!
//! The application's `session/ignored_users.rs` with the `GObject` and the
//! `gio::ListModel` removed. Two things the facade did not have and the
//! application always did, and both are behaviour rather than shape:
//!
//! * **It follows the list.** The application subscribes to the SDK's
//!   ignore-list changes and re-reads the account data whenever one arrives, so
//!   ignoring somebody on another device is visible here. The facade read the
//!   account data once, per call, and never again.
//! * **It refuses a redundant request.** Adding a user already on the list, or
//!   removing one that is not on it, is a no-op with a warning rather than a
//!   round trip.
//!
//! What stayed behind is the `items_changed` computation: working out the
//! shortest splice that turns the old list into the new one exists to keep
//! `gio::ListModel` from rebuilding rows, and belongs to the bridge.

use std::sync::{Arc, Mutex};

use eyeball::{SharedObservable, Subscriber};
use ruma::{OwnedUserId, UserId, events::ignored_user_list::IgnoredUserListEventContent};
use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

use super::WeakSession;
use crate::{RUNTIME, UserFacingError, spawn_tokio, utils::LoadingState};

/// What can go wrong while changing the ignored users list.
#[derive(Debug, thiserror::Error)]
pub enum IgnoredUsersError {
    /// The session this list belongs to is gone.
    #[error("the session is no longer available")]
    NoSession,
    /// The homeserver refused the change.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl UserFacingError for IgnoredUsersError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NoSession => "The session is no longer available.".to_owned(),
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// The list of users the account ignores.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct IgnoredUsers {
    inner: Arc<IgnoredUsersInner>,
}

#[derive(Debug)]
struct IgnoredUsersInner {
    /// The session this list belongs to.
    session: WeakSession,
    /// The ignored users, in the order the account data lists them.
    ///
    /// A `Vec` rather than the application's `IndexSet`: the keys of
    /// `m.ignored_user_list` are already unique, the order is the only
    /// other thing the set was preserving, and the index lookups it made
    /// cheap were `gio::ListModel`'s. Checking the rest of the lesson from
    /// `utils/matrix/` — what the type is actually doing — leaves a plain
    /// vector, over a handful of entries.
    list: SharedObservable<Vec<OwnedUserId>>,
    /// How far the first read has got.
    ///
    /// The facade's `ignored_users()` used to fetch the account data on
    /// every call, and a session counts as active before `prepare()` has
    /// finished — so a read that only consulted the cache could answer
    /// "nobody" during startup where the old one answered correctly.
    /// [`Self::ensure_loaded()`] is what keeps that guarantee.
    state: SharedObservable<LoadingState>,
    /// The task watching the SDK for changes to the list.
    watch_handle: Mutex<Option<AbortHandle>>,
}

impl Drop for IgnoredUsersInner {
    fn drop(&mut self) {
        if let Ok(Some(handle)) = self.watch_handle.get_mut().map(Option::take) {
            handle.abort();
        }
    }
}

impl IgnoredUsers {
    /// Create the ignored users list of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        Self {
            inner: Arc::new(IgnoredUsersInner {
                session,
                list: SharedObservable::new(Vec::new()),
                state: SharedObservable::new(LoadingState::Initial),
                watch_handle: Mutex::new(None),
            }),
        }
    }

    /// Load the list from the store, and follow it from there.
    ///
    /// Watching is installed before the first read, so a change that lands
    /// during it is not lost.
    pub async fn load(&self) {
        self.watch();
        self.reload().await;
    }

    /// Read the list unless it has already been read.
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

    /// A snapshot of the ignored users.
    #[must_use]
    pub fn snapshot(&self) -> Vec<OwnedUserId> {
        self.inner.list.get()
    }

    /// Whether the given user is on the list.
    #[must_use]
    pub fn contains(&self, user_id: &UserId) -> bool {
        self.inner
            .list
            .read()
            .iter()
            .any(|ignored| ignored == user_id)
    }

    /// The current list, and every version of it that follows.
    pub fn subscribe(&self) -> Subscriber<Vec<OwnedUserId>> {
        self.inner.list.subscribe()
    }

    /// Ignore the given user: their messages disappear everywhere.
    ///
    /// A user already on the list is left alone, as the application does,
    /// rather than spending a request to say what the server already knows.
    pub async fn add(&self, user_id: &UserId) -> Result<(), IgnoredUsersError> {
        if self.contains(user_id) {
            warn!("{user_id} is already ignored, not asking again");
            return Ok(());
        }

        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(IgnoredUsersError::NoSession)?;
        let client = session.client();
        let owned = user_id.to_owned();

        spawn_tokio!(async move { client.account().ignore_user(&owned).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| IgnoredUsersError::Server(Box::new(error)))?;

        // The subscription will bring the same answer, but not before the
        // next sync; the application updates its own list here too, so the
        // change is visible the moment the server took it.
        self.inner.list.update(|list| {
            if !list.iter().any(|ignored| ignored == user_id) {
                list.push(user_id.to_owned());
            }
        });

        Ok(())
    }

    /// Stop ignoring the given user.
    pub async fn remove(&self, user_id: &UserId) -> Result<(), IgnoredUsersError> {
        if !self.contains(user_id) {
            warn!("{user_id} is not ignored, not asking again");
            return Ok(());
        }

        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(IgnoredUsersError::NoSession)?;
        let client = session.client();
        let owned = user_id.to_owned();

        spawn_tokio!(async move { client.account().unignore_user(&owned).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| IgnoredUsersError::Server(Box::new(error)))?;

        self.inner
            .list
            .update(|list| list.retain(|ignored| ignored != user_id));

        Ok(())
    }

    /// Watch the SDK for changes to the list, replacing any previous watch.
    fn watch(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        let subscriber = session.client().subscribe_to_ignore_user_list_changes();

        // The subscriber carries the new list, and the application reads
        // the account data again anyway rather than trusting it — the
        // event is the source either way, and one path keeps one shape.
        let weak = Arc::downgrade(&self.inner);
        let handle = RUNTIME
            .spawn(async move {
                let mut subscriber = subscriber;
                while subscriber.next().await.is_some() {
                    let Some(inner) = weak.upgrade() else { break };
                    Self { inner }.reload().await;
                }
            })
            .abort_handle();

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

    /// Read the list out of the account data and publish it.
    async fn reload(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        self.inner.state.set_if_not_eq(LoadingState::Loading);
        let client = session.client();

        let raw = spawn_tokio!(async move {
            client
                .account()
                .account_data::<IgnoredUserListEventContent>()
                .await
        })
        .await
        .expect("task was not aborted");

        let raw = match raw {
            Ok(Some(raw)) => raw,
            Ok(None) => {
                debug!("Got no ignored users list");
                self.inner.list.set_if_not_eq(Vec::new());
                self.inner.state.set_if_not_eq(LoadingState::Ready);
                return;
            }
            Err(error) => {
                error!("Could not get ignored users list: {error}");
                self.inner.state.set_if_not_eq(LoadingState::Error);
                return;
            }
        };

        match raw.deserialize() {
            Ok(content) => {
                self.inner
                    .list
                    .set_if_not_eq(content.ignored_users.into_keys().collect());
                self.inner.state.set_if_not_eq(LoadingState::Ready);
            }
            Err(error) => {
                error!("Could not deserialize ignored users list: {error}");
                self.inner.state.set_if_not_eq(LoadingState::Error);
            }
        }
    }
}

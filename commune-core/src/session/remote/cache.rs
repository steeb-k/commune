//! Remote data, asked for once and kept for a while.
//!
//! The application's `RemoteCache` (`src/session/remote/cache.rs`) with
//! the `GObject`s replaced by entries: an entry is what a page follows —
//! the value once it arrived, and how far the request for it has got — and
//! the cache is what decides whether to ask again. A room's data is trusted
//! for a day, a user's profile for an hour, and a preview of a URL forever:
//! a page changing under a message that linked to it does not change what
//! the message said.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use eyeball::{SharedObservable, Subscriber};
use quick_cache::sync::Cache;
use ruma::{OwnedRoomAliasId, OwnedRoomId, OwnedRoomOrAliasId, OwnedUserId, RoomId, UserId};
use url::Url;

use super::{RemoteRoom, RemoteUserProfile, UrlPreview, UrlPreviewSupport, url_host};
use crate::{RUNTIME, matrix::MatrixRoomIdUri, session::WeakSession, utils::LoadingState};

/// The time after which the data of a room is assumed to be stale.
///
/// This matches 1 day.
pub const ROOM_DATA_VALIDITY_DURATION: Duration = Duration::from_hours(24);

/// The time after which the profile of a user is assumed to be stale.
///
/// This matches 1 hour.
pub const PROFILE_VALIDITY_DURATION: Duration = Duration::from_hours(1);

/// How many remote rooms are kept.
const ROOMS_CAPACITY: usize = 30;

/// How many remote users are kept.
const USERS_CAPACITY: usize = 30;

/// How many URL previews are kept.
///
/// A screenful of the timeline holds far fewer links than this, so
/// scrolling back over one never asks for it twice.
const URL_PREVIEWS_CAPACITY: usize = 100;

/// What is known about a remote room, and how far the request for it has
/// got.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteRoomState {
    /// The room, once the homeserver described it.
    ///
    /// Kept through a reload, so that a page keeps showing what it had
    /// while the newer answer is on its way.
    pub data: Option<RemoteRoom>,
    /// How far the request has got.
    pub loading_state: LoadingState,
}

/// A remote room the cache keeps: its data, and the request for it.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct RemoteRoomEntry {
    inner: Arc<RemoteRoomEntryInner>,
}

#[derive(Debug)]
struct RemoteRoomEntryInner {
    /// The session to ask with.
    session: WeakSession,
    /// The Matrix URI this room was asked for by.
    uri: MatrixRoomIdUri,
    /// What is known about the room.
    state: SharedObservable<RemoteRoomState>,
    /// The time of the last request.
    last_request_time: Mutex<Option<Instant>>,
}

impl RemoteRoomEntry {
    /// An entry for the room at the given URI, with nothing known yet.
    fn new(session: WeakSession, uri: MatrixRoomIdUri) -> Self {
        Self {
            inner: Arc::new(RemoteRoomEntryInner {
                session,
                uri,
                state: SharedObservable::new(RemoteRoomState::default()),
                last_request_time: Mutex::new(None),
            }),
        }
    }

    /// The Matrix URI this room was asked for by.
    #[must_use]
    pub fn uri(&self) -> &MatrixRoomIdUri {
        &self.inner.uri
    }

    /// What is known about the room.
    #[must_use]
    pub fn state(&self) -> RemoteRoomState {
        self.inner.state.get()
    }

    /// Subscribe to what is known about the room.
    pub fn subscribe(&self) -> Subscriber<RemoteRoomState> {
        self.inner.state.subscribe()
    }

    /// The ID of this room: the one the homeserver said, else the one it
    /// was asked for by, if that is an ID.
    #[must_use]
    pub fn room_id(&self) -> Option<OwnedRoomId> {
        self.state()
            .data
            .map(|data| data.room_id)
            .or_else(|| OwnedRoomId::try_from(self.inner.uri.id.clone()).ok())
    }

    /// The canonical alias of this room: the one the homeserver said, else
    /// the one it was asked for by, if that is an alias.
    #[must_use]
    pub fn canonical_alias(&self) -> Option<OwnedRoomAliasId> {
        self.state()
            .data
            .and_then(|data| data.canonical_alias)
            .or_else(|| OwnedRoomAliasId::try_from(self.inner.uri.id.clone()).ok())
    }

    /// Whether the data of the room is considered to be stale.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        self.inner
            .last_request_time
            .lock()
            .expect("mutex is not poisoned")
            .is_none_or(|last_time| last_time.elapsed() > ROOM_DATA_VALIDITY_DURATION)
    }

    /// Ask for the data of the room again if it is considered to be stale.
    pub fn load_if_stale(&self) {
        if !self.is_stale() {
            // The data is still valid, nothing to do.
            return;
        }

        // Set the request time right away, to prevent several requests at
        // the same time.
        *self
            .inner
            .last_request_time
            .lock()
            .expect("mutex is not poisoned") = Some(Instant::now());

        let entry = self.clone();
        RUNTIME.spawn(async move {
            entry.load().await;
        });
    }

    /// Ask for the data of the room.
    async fn load(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            self.inner
                .last_request_time
                .lock()
                .expect("mutex is not poisoned")
                .take();
            return;
        };

        self.inner.state.update(|state| {
            state.loading_state = LoadingState::Loading;
        });

        if let Ok(data) = session.remote_room(self.inner.uri.clone()).await {
            self.inner.state.set(RemoteRoomState {
                data: Some(data),
                loading_state: LoadingState::Ready,
            });
        } else {
            // Already logged; reset the request time so that the next time
            // it is asked for tries again.
            self.inner
                .last_request_time
                .lock()
                .expect("mutex is not poisoned")
                .take();
            self.inner.state.update(|state| {
                state.loading_state = LoadingState::Error;
            });
        }
    }
}

/// What is known about a remote user, and how far the request for their
/// profile has got.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteUserState {
    /// The profile, once the homeserver described it.
    pub profile: Option<RemoteUserProfile>,
    /// How far the request has got.
    pub loading_state: LoadingState,
}

/// A remote user the cache keeps: their profile, and the request for it.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct RemoteUserEntry {
    inner: Arc<RemoteUserEntryInner>,
}

#[derive(Debug)]
struct RemoteUserEntryInner {
    /// The session to ask with.
    session: WeakSession,
    /// The ID of the user.
    user_id: OwnedUserId,
    /// What is known about the user.
    state: SharedObservable<RemoteUserState>,
    /// The time of the last request.
    last_request_time: Mutex<Option<Instant>>,
}

impl RemoteUserEntry {
    /// An entry for the user with the given ID, with nothing known yet.
    fn new(session: WeakSession, user_id: OwnedUserId) -> Self {
        Self {
            inner: Arc::new(RemoteUserEntryInner {
                session,
                user_id,
                state: SharedObservable::new(RemoteUserState::default()),
                last_request_time: Mutex::new(None),
            }),
        }
    }

    /// The ID of the user.
    #[must_use]
    pub fn user_id(&self) -> &UserId {
        &self.inner.user_id
    }

    /// What is known about the user.
    #[must_use]
    pub fn state(&self) -> RemoteUserState {
        self.inner.state.get()
    }

    /// Subscribe to what is known about the user.
    pub fn subscribe(&self) -> Subscriber<RemoteUserState> {
        self.inner.state.subscribe()
    }

    /// Whether the profile of the user is considered to be stale.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        self.inner
            .last_request_time
            .lock()
            .expect("mutex is not poisoned")
            .is_none_or(|last_time| last_time.elapsed() > PROFILE_VALIDITY_DURATION)
    }

    /// Ask for the profile of the user again if it is considered to be
    /// stale.
    pub fn load_if_stale(&self) {
        if !self.is_stale() {
            // The data is still valid, nothing to do.
            return;
        }

        // Set the request time right away, to prevent several requests at
        // the same time.
        *self
            .inner
            .last_request_time
            .lock()
            .expect("mutex is not poisoned") = Some(Instant::now());

        let entry = self.clone();
        RUNTIME.spawn(async move {
            entry.load().await;
        });
    }

    /// Ask for the profile of the user.
    async fn load(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            self.inner
                .last_request_time
                .lock()
                .expect("mutex is not poisoned")
                .take();
            return;
        };

        self.inner.state.update(|state| {
            state.loading_state = LoadingState::Loading;
        });

        if let Ok(profile) = session.remote_user_profile(&self.inner.user_id).await {
            self.inner.state.set(RemoteUserState {
                profile: Some(profile),
                loading_state: LoadingState::Ready,
            });
        } else {
            // Already logged; reset the request time so that the next time
            // it is asked for tries again.
            self.inner
                .last_request_time
                .lock()
                .expect("mutex is not poisoned")
                .take();
            self.inner.state.update(|state| {
                state.loading_state = LoadingState::Error;
            });
        }
    }
}

/// What is known about the preview of a URL, and how far the request for
/// it has got.
#[derive(Debug, Clone, Default)]
pub struct UrlPreviewState {
    /// The preview, once the homeserver described it.
    pub preview: Option<UrlPreview>,
    /// How far the request has got.
    pub loading_state: LoadingState,
}

/// A URL preview the cache keeps: the preview, and the request for it.
///
/// Asked for once, when the entry is made, and never again: a page
/// changing under a message that linked to it does not change what the
/// message said. Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct UrlPreviewEntry {
    inner: Arc<UrlPreviewEntryInner>,
}

#[derive(Debug)]
struct UrlPreviewEntryInner {
    /// The session to ask with.
    session: WeakSession,
    /// The URL that this is a preview of.
    url: Url,
    /// Whether the homeserver can answer a preview request at all.
    support: UrlPreviewSupport,
    /// What is known about the preview.
    state: SharedObservable<UrlPreviewState>,
}

impl UrlPreviewEntry {
    /// An entry for the preview of the given URL, with nothing known yet.
    fn new(session: WeakSession, url: Url, support: UrlPreviewSupport) -> Self {
        Self {
            inner: Arc::new(UrlPreviewEntryInner {
                session,
                url,
                support,
                state: SharedObservable::new(UrlPreviewState::default()),
            }),
        }
    }

    /// The URL that this is a preview of.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.inner.url
    }

    /// What is known about the preview.
    #[must_use]
    pub fn state(&self) -> UrlPreviewState {
        self.inner.state.get()
    }

    /// Subscribe to what is known about the preview.
    pub fn subscribe(&self) -> Subscriber<UrlPreviewState> {
        self.inner.state.subscribe()
    }

    /// The name of the site: what the homeserver said, else the host of
    /// the URL, which is what there is to show until it answers.
    #[must_use]
    pub fn site_name(&self) -> String {
        self.state()
            .preview
            .map_or_else(|| url_host(&self.inner.url), |preview| preview.site_name)
    }

    /// Ask for the preview.
    fn spawn_load(&self) {
        let entry = self.clone();
        RUNTIME.spawn(async move {
            entry.load().await;
        });
    }

    /// Ask for the preview.
    async fn load(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        self.inner.state.update(|state| {
            state.loading_state = LoadingState::Loading;
        });

        if let Ok(preview) = session
            .url_preview(&self.inner.url, &self.inner.support)
            .await
        {
            self.inner.state.set(UrlPreviewState {
                preview: Some(preview),
                loading_state: LoadingState::Ready,
            });
        } else {
            // Already logged where it matters; a card with nothing on it is
            // not drawn.
            self.inner.state.update(|state| {
                state.loading_state = LoadingState::Error;
            });
        }
    }
}

/// An API to query remote data and cache it.
///
/// Cheap to clone; every clone shares the same caches.
#[derive(Debug, Clone)]
pub struct RemoteCache {
    inner: Arc<RemoteCacheInner>,
}

#[derive(Debug)]
struct RemoteCacheInner {
    /// The session to ask with.
    session: WeakSession,
    /// Remote rooms.
    rooms: Cache<OwnedRoomOrAliasId, RemoteRoomEntry>,
    /// Remote users.
    users: Cache<OwnedUserId, RemoteUserEntry>,
    /// Previews of URLs, keyed by the URL they are a preview of.
    url_previews: Cache<String, UrlPreviewEntry>,
    /// Whether the homeserver can answer a URL preview request.
    url_previews_support: UrlPreviewSupport,
}

impl RemoteCache {
    /// Create the remote cache of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        Self {
            inner: Arc::new(RemoteCacheInner {
                session,
                rooms: Cache::new(ROOMS_CAPACITY),
                users: Cache::new(USERS_CAPACITY),
                url_previews: Cache::new(URL_PREVIEWS_CAPACITY),
                url_previews_support: UrlPreviewSupport::default(),
            }),
        }
    }

    /// The remote room for the given URI.
    ///
    /// A room already known under another of its identifiers — asked for by
    /// alias and now by ID, or the other way round — is the same room.
    #[must_use]
    pub fn room(&self, uri: MatrixRoomIdUri) -> RemoteRoomEntry {
        let rooms = &self.inner.rooms;

        // Check if the room is in the cache.
        if let Some(entry) = rooms.get(&uri.id) {
            entry.load_if_stale();
            return entry;
        }

        // Check if the alias or ID matches a room in the cache, in case the
        // URI uses another ID than the one we used as a key for the cache.
        let id_or_alias = <&RoomId>::try_from(&*uri.id);
        let found = rooms
            .iter()
            .find(|(_, entry)| match id_or_alias {
                Ok(room_id) => entry.room_id().is_some_and(|id| id == room_id),
                Err(room_alias) => entry
                    .canonical_alias()
                    .is_some_and(|alias| alias == room_alias),
            })
            .map(|(_, entry)| entry);

        if let Some(entry) = found {
            entry.load_if_stale();
            return entry;
        }

        // We did not find it, create the room.
        let id = uri.id.clone();
        let entry = RemoteRoomEntry::new(self.inner.session.clone(), uri);
        rooms.insert(id, entry.clone());
        entry.load_if_stale();

        entry
    }

    /// The remote user for the given ID.
    #[must_use]
    pub fn user(&self, user_id: OwnedUserId) -> RemoteUserEntry {
        let users = &self.inner.users;

        // Check if the user is in the cache.
        if let Some(entry) = users.get(&user_id) {
            entry.load_if_stale();
            return entry;
        }

        // We did not find it, create the user.
        let entry = RemoteUserEntry::new(self.inner.session.clone(), user_id.clone());
        users.insert(user_id, entry.clone());
        entry.load_if_stale();

        entry
    }

    /// The preview for the given URL.
    #[must_use]
    pub fn url_preview(&self, url: Url) -> UrlPreviewEntry {
        let url_previews = &self.inner.url_previews;
        let key = url.to_string();

        // Check if the preview is in the cache.
        if let Some(entry) = url_previews.get(&key) {
            return entry;
        }

        // We did not find it, request it. Unlike rooms and users, a preview
        // is never reloaded.
        let entry = UrlPreviewEntry::new(
            self.inner.session.clone(),
            url,
            self.inner.url_previews_support.clone(),
        );
        url_previews.insert(key, entry.clone());
        entry.spawn_load();

        entry
    }
}

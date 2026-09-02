use std::{cell::RefCell, fmt, rc::Rc};

use quick_cache::unsync::Cache;
use ruma::{OwnedRoomOrAliasId, OwnedUserId};
use url::Url;

use super::{RemoteRoom, RemoteUrlPreview, RemoteUser};
use crate::{session::Session, utils::matrix::MatrixRoomIdUri};

/// The data of the [`RemoteCache`].
///
/// One object per entry the core keeps, under the same key and with the
/// same capacity, so that a page asking twice gets the same object and the
/// two caches forget together.
struct RemoteCacheData {
    /// Remote rooms.
    rooms: RefCell<Cache<OwnedRoomOrAliasId, RemoteRoom>>,
    /// Remote users.
    users: RefCell<Cache<OwnedUserId, RemoteUser>>,
    /// Previews of URLs, keyed by the URL they are a preview of.
    url_previews: RefCell<Cache<String, RemoteUrlPreview>>,
}

/// An API to query remote data and cache it.
///
/// The data, the staleness and the requests are the core's
/// ([`commune_core::session::RemoteCache`]); this hands out the objects that
/// present its entries.
#[derive(Clone)]
pub(crate) struct RemoteCache {
    session: Session,
    data: Rc<RemoteCacheData>,
}

impl RemoteCache {
    /// Construct a new `RemoteCache` for the given session.
    pub(crate) fn new(session: Session) -> Self {
        Self {
            session,
            data: RemoteCacheData {
                rooms: Cache::new(30).into(),
                users: Cache::new(30).into(),
                url_previews: Cache::new(100).into(),
            }
            .into(),
        }
    }

    /// Get the remote room for the given URI.
    ///
    /// The core decides which room this is — a room asked for by alias and
    /// now by ID is the same room — and whether to ask the homeserver again.
    pub(crate) fn room(&self, uri: MatrixRoomIdUri) -> RemoteRoom {
        let entry = self.session.core().remote_cache().room(uri);
        let key = entry.uri().id.clone();

        let mut rooms = self.data.rooms.borrow_mut();

        if let Some(room) = rooms.get(&key) {
            return room.clone();
        }

        let room = RemoteRoom::new(&self.session, &entry);
        rooms.insert(key, room.clone());

        room
    }

    /// Get the remote user for the given ID.
    pub(crate) fn user(&self, user_id: OwnedUserId) -> RemoteUser {
        let entry = self.session.core().remote_cache().user(user_id.clone());

        let mut users = self.data.users.borrow_mut();

        if let Some(user) = users.get(&user_id) {
            return user.clone();
        }

        let user = RemoteUser::new(&self.session, &entry);
        users.insert(user_id, user.clone());

        user
    }

    /// Get the preview for the given URL.
    pub(crate) fn url_preview(&self, url: Url) -> RemoteUrlPreview {
        let key = url.to_string();
        let entry = self.session.core().remote_cache().url_preview(url);

        let mut url_previews = self.data.url_previews.borrow_mut();

        if let Some(preview) = url_previews.get(&key) {
            return preview.clone();
        }

        let preview = RemoteUrlPreview::new(&entry);
        url_previews.insert(key, preview.clone());

        preview
    }
}

impl fmt::Debug for RemoteCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteCache").finish_non_exhaustive()
    }
}

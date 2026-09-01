//! The public room directory of a homeserver.
//!
//! The headless counterpart of the application's `ExploreSearch`
//! (`src/session_view/explore/search.rs`) and `ExploreServer`: one page of
//! the directory for a query, as the application requests it. The
//! pagination token stays with the caller, because the application's
//! search object keeps it beside its `GListStore` and the FFI hands it
//! across as a value; the abort handle stays with the application, since
//! the task that runs a page is the bridge's to cancel.

use ruma::{
    OwnedServerName,
    api::client::directory::get_public_rooms_filtered,
    assign,
    directory::{Filter, RoomNetwork},
};
use tracing::error;

use super::{RemoteRoom, Session};
use crate::{UserFacingError, matrix::MatrixRoomIdUri, spawn_tokio};

/// The maximum size of a batch of public rooms.
const PUBLIC_ROOMS_BATCH_SIZE: u32 = 20;

/// What can go wrong while searching the directory.
#[derive(Debug, thiserror::Error)]
pub enum DirectoryError {
    /// The homeserver could not search the directory.
    ///
    /// Boxed because `matrix_sdk::HttpError` is large enough that carrying
    /// it by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::HttpError>),
}

impl UserFacingError for DirectoryError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// A search in the public rooms directory — the application's
/// `ExploreSearchData` with its `ExploreServer` folded in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublicRoomsQuery {
    /// The term to search.
    pub search_term: Option<String>,
    /// The server to search.
    ///
    /// If this is `None`, our own homeserver will be queried.
    pub server: Option<OwnedServerName>,
    /// The third-party network to search.
    ///
    /// If this is `None`, the Matrix network will be queried.
    pub third_party_network: Option<String>,
}

impl PublicRoomsQuery {
    /// Convert this query to a request continuing from the given batch.
    fn as_request(&self, next_batch: Option<String>) -> get_public_rooms_filtered::v3::Request {
        let room_network = if let Some(third_party_network) = &self.third_party_network {
            RoomNetwork::ThirdParty(third_party_network.clone())
        } else {
            RoomNetwork::Matrix
        };

        assign!(get_public_rooms_filtered::v3::Request::new(), {
            limit: Some(PUBLIC_ROOMS_BATCH_SIZE.into()),
            since: next_batch,
            room_network,
            server: self.server.clone(),
            // No `room_types` filter: an empty list means no filtering, so
            // spaces come back alongside rooms. `RoomTypeFilter::Default` is
            // what used to hide them.
            filter: assign!(
                Filter::new(),
                { generic_search_term: self.search_term.clone() }
            ),
        })
    }
}

/// One page of the public rooms directory.
#[derive(Debug, Clone)]
pub struct PublicRoomsPage {
    /// The rooms of this page.
    pub rooms: Vec<RemoteRoom>,
    /// The token to request the next page with, absent at the end.
    pub next_batch: Option<String>,
}

impl Session {
    /// One page of the public rooms directory for the given query,
    /// continuing from the given batch when there is one.
    pub async fn public_rooms(
        &self,
        query: &PublicRoomsQuery,
        next_batch: Option<String>,
    ) -> Result<PublicRoomsPage, DirectoryError> {
        let request = query.as_request(next_batch);

        let client = self.client();
        let handle = spawn_tokio!(async move { client.public_rooms_filtered(request).await });

        let response = handle
            .await
            .expect("task was not aborted")
            .map_err(|search_error| {
                error!("Could not search public rooms: {search_error}");
                Box::new(search_error)
            })?;

        let rooms = response
            .chunk
            .into_iter()
            .map(|data| {
                let id = data
                    .canonical_alias
                    .clone()
                    .map_or_else(|| data.room_id.clone().into(), Into::into);
                let uri = MatrixRoomIdUri {
                    id,
                    via: query.server.clone().into_iter().collect(),
                };

                RemoteRoom::with_data(uri, data)
            })
            .collect();

        Ok(PublicRoomsPage {
            rooms,
            next_batch: response.next_batch,
        })
    }
}

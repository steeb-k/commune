//! The `mutual_rooms` endpoint, which the pinned ruma does not carry at its
//! stable path.
//!
//! The endpoint was standardised as `GET /_matrix/client/v1/mutual_rooms`,
//! but the pinned ruma only knows the MSC2666 unstable path, and its request
//! macros refuse to expand outside its own tree. So the request is spelled
//! out over the SDK's own HTTP client: the stable path first, the unstable
//! name as the fallback for a homeserver that has not caught up.

use matrix_sdk::Client;
use ruma::{OwnedRoomId, OwnedUserId};
use serde::Deserialize;
use tracing::debug;

/// The maximum number of pages fetched.
///
/// The page size is the server's. Anybody sharing more rooms than several
/// pages of that is not going to read the whole list anyway.
const MAX_PAGES: usize = 5;

/// The paths the endpoint answers at, in the order they are tried.
const PATHS: &[&str] = &[
    "_matrix/client/v1/mutual_rooms",
    "_matrix/client/unstable/uk.half-shot.msc2666/user/mutual_rooms",
];

/// One page of the response of the `mutual_rooms` endpoint.
#[derive(Debug, Deserialize)]
struct MutualRoomsPage {
    /// The rooms both users are joined to.
    joined: Vec<OwnedRoomId>,

    /// The pagination token, when the server paginates this response.
    #[serde(default)]
    next_batch: Option<String>,
}

/// Fetch the rooms shared with the given user.
///
/// Returns `None` when the homeserver does not answer the endpoint, which is
/// not an error: the endpoint is recent, and a section that draws nothing is
/// how its absence should look.
pub(crate) async fn fetch_mutual_rooms(
    client: &Client,
    user_id: OwnedUserId,
) -> Option<Vec<OwnedRoomId>> {
    let access_token = client.access_token()?;

    for path in PATHS {
        let mut rooms = Vec::new();
        let mut from: Option<String> = None;
        let mut unrecognized = false;

        for _ in 0..MAX_PAGES {
            let Ok(mut url) = client.homeserver().join(path) else {
                return None;
            };
            url.query_pairs_mut()
                .append_pair("user_id", user_id.as_str());
            if let Some(from) = &from {
                url.query_pairs_mut().append_pair("from", from);
            }

            let response = match client
                .http_client()
                .get(url)
                .bearer_auth(&access_token)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    debug!("Could not fetch mutual rooms: {error}");
                    return None;
                }
            };

            if response.status() == 404 {
                // The homeserver does not know this path; the next one might
                // still be answered.
                unrecognized = true;
                break;
            }
            if !response.status().is_success() {
                debug!("Could not fetch mutual rooms: HTTP {}", response.status());
                return None;
            }

            let body = match response.bytes().await {
                Ok(body) => body,
                Err(error) => {
                    debug!("Could not read the mutual rooms response: {error}");
                    return None;
                }
            };
            let page = match serde_json::from_slice::<MutualRoomsPage>(&body) {
                Ok(page) => page,
                Err(error) => {
                    debug!("Could not parse the mutual rooms response: {error}");
                    return None;
                }
            };

            rooms.extend(page.joined);

            match page.next_batch {
                Some(token) => from = Some(token),
                None => break,
            }
        }

        if !unrecognized {
            return Some(rooms);
        }
    }

    None
}

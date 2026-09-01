//! A room that can only be described by asking the homeserver, i.e. one
//! that sync will not update.
//!
//! The value half of the application's `RemoteRoom`
//! (`src/session/remote/room.rs`): what a room summary says, the way it
//! is looked up — the summary endpoint first, the space hierarchy when
//! the homeserver does not have it — and how it is matched against the
//! rooms the session is in. What stayed in the application is the
//! `PillSource` it extends, the avatar image, the linkified topic and the
//! request-time bookkeeping that reloads stale data.

use matrix_sdk::reqwest::StatusCode;
use ruma::{
    OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId, OwnedRoomOrAliasId,
    api::client::{room::get_summary, space::get_hierarchy},
    assign,
    room::{JoinRuleSummary, RoomSummary, RoomType},
    uint,
};
use tracing::{debug, warn};

use crate::{
    UserFacingError,
    matrix::MatrixRoomIdUri,
    session::{Room, RoomList, Session},
    spawn_tokio,
    utils::OptionStringExt,
};

/// What can go wrong while looking a remote room up.
#[derive(Debug, thiserror::Error)]
pub enum RemoteRoomError {
    /// The summary endpoint failed for a reason other than not having the
    /// endpoint.
    ///
    /// Boxed because the SDK's errors are large enough that carrying them
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Summary(Box<matrix_sdk::HttpError>),
    /// The alias could not be resolved to a room ID for the hierarchy
    /// endpoint, which only takes IDs.
    #[error(transparent)]
    ResolveAlias(Box<matrix_sdk::HttpError>),
    /// The hierarchy endpoint failed.
    #[error(transparent)]
    Hierarchy(Box<matrix_sdk::HttpError>),
    /// The hierarchy endpoint answered, but not about the room asked for.
    #[error("the homeserver did not describe the room")]
    NotDescribed,
}

impl UserFacingError for RemoteRoomError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Summary(error) | Self::ResolveAlias(error) | Self::Hierarchy(error) => {
                error.to_string()
            }
            Self::NotDescribed => "Could not find the room.".to_owned(),
        }
    }
}

/// A room described by the homeserver rather than kept current by sync.
// The five flags are the application's five boolean properties on the
// same object, read one at a time by the interface; a state machine over
// them would be an invention.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRoom {
    /// The Matrix URI this room was asked for by.
    pub uri: MatrixRoomIdUri,
    /// The ID of this room.
    pub room_id: OwnedRoomId,
    /// The canonical alias of this room, if any.
    ///
    /// When the summary carries none and the URI is an alias, that alias.
    pub canonical_alias: Option<OwnedRoomAliasId>,
    /// The name that is set for this room.
    ///
    /// This can be empty; [`Self::display_name()`] is what the interface
    /// should show.
    pub name: Option<String>,
    /// The topic of this room, with nothing but whitespace counted as none.
    pub topic: Option<String>,
    /// The number of joined members in the room.
    pub joined_members_count: u32,
    /// Whether we can knock on the room.
    pub can_knock: bool,
    /// Whether this room is a space.
    pub is_space: bool,
    /// Whether this room can be read without joining it.
    pub is_world_readable: bool,
    /// Whether this room is encrypted.
    pub is_encrypted: bool,
    /// The avatar of this room, if any.
    pub avatar_url: Option<OwnedMxcUri>,
    /// Whether the space this room was listed from suggests it.
    ///
    /// This belongs to the `m.space.child` event rather than to the room,
    /// so it is only ever true for a room that came from a space's
    /// hierarchy.
    pub is_suggested: bool,
}

impl RemoteRoom {
    /// Construct a `RemoteRoom` for the given URI from the given summary.
    #[must_use]
    pub fn with_data(uri: MatrixRoomIdUri, data: impl Into<RoomSummary>) -> Self {
        let data = data.into();

        let canonical_alias = data
            .canonical_alias
            .or_else(|| OwnedRoomAliasId::try_from(uri.id.clone()).ok());
        let topic = data
            .topic
            .into_clean_string()
            .filter(|s| !s.is_empty() && s.find(|c: char| !c.is_whitespace()).is_some());

        Self {
            room_id: data.room_id,
            canonical_alias,
            name: data.name.into_clean_string(),
            topic,
            joined_members_count: data.num_joined_members.try_into().unwrap_or(u32::MAX),
            can_knock: matches!(
                data.join_rule,
                JoinRuleSummary::Knock | JoinRuleSummary::KnockRestricted(_)
            ),
            is_space: matches!(data.room_type, Some(RoomType::Space)),
            is_world_readable: data.world_readable,
            is_encrypted: data.encryption.is_some(),
            avatar_url: data.avatar_url,
            is_suggested: false,
            uri,
        }
    }

    /// The name to show for this room: its name, else its canonical alias,
    /// else the identifier it was asked for by.
    #[must_use]
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.canonical_alias.as_ref().map(ToString::to_string))
            .unwrap_or_else(|| self.uri.id.to_string())
    }

    /// The identifiers under which the session's own room list may hold
    /// this room: its ID, its canonical alias, and the identifier it was
    /// asked for by.
    #[must_use]
    pub fn identifiers(&self) -> Vec<OwnedRoomOrAliasId> {
        let id = self.uri.id.clone();
        let room_id = Some(self.room_id.clone())
            .filter(|room_id| room_id.as_str() != id.as_str())
            .map(Into::into);
        let canonical_alias = self
            .canonical_alias
            .clone()
            .filter(|alias| alias.as_str() != id.as_str())
            .map(Into::into);

        room_id
            .into_iter()
            .chain(canonical_alias)
            .chain(Some(id))
            .collect()
    }

    /// The room in the given list matching this one, if any — the
    /// application's `RoomListRoomInfo::local_room`.
    #[must_use]
    pub fn local_room(&self, room_list: &RoomList) -> Option<Room> {
        self.identifiers()
            .iter()
            .find_map(|identifier| room_list.get_by_identifier(identifier))
    }

    /// Whether the given list is currently joining this room.
    #[must_use]
    pub fn is_joining(&self, room_list: &RoomList) -> bool {
        self.identifiers()
            .iter()
            .any(|identifier| room_list.is_joining_room(identifier))
    }
}

impl Session {
    /// Describe the room at the given URI, asking the homeserver.
    ///
    /// The summary endpoint is tried first; when the homeserver does not
    /// have it, the space hierarchy endpoint, which works for any room the
    /// homeserver already knows.
    pub async fn remote_room(&self, uri: MatrixRoomIdUri) -> Result<RemoteRoom, RemoteRoomError> {
        match self.remote_room_from_summary(&uri).await {
            Ok(Some(room)) => Ok(room),
            Ok(None) => self.remote_room_from_space_hierarchy(uri).await,
            Err(error) => Err(error),
        }
    }

    /// Describe the room at the given URI with the room summary endpoint.
    ///
    /// At the time of writing this code, MSC3266 has been accepted but the
    /// endpoint is not part of a Matrix spec release.
    ///
    /// Returns `Ok(None)` if the endpoint is not supported by the
    /// homeserver.
    async fn remote_room_from_summary(
        &self,
        uri: &MatrixRoomIdUri,
    ) -> Result<Option<RemoteRoom>, RemoteRoomError> {
        let client = self.client();
        let request = get_summary::v1::Request::new(uri.id.clone(), uri.via.clone());
        let handle = spawn_tokio!(async move { client.send(request).await });

        match handle.await.expect("task was not aborted") {
            Ok(response) => Ok(Some(RemoteRoom::with_data(uri.clone(), response.summary))),
            Err(error) => {
                if error
                    .as_client_api_error()
                    .is_some_and(|error| error.status_code == StatusCode::NOT_FOUND)
                {
                    return Ok(None);
                }

                warn!(
                    "Could not get room details from summary endpoint for room `{}`: {error}",
                    uri.id
                );
                Err(RemoteRoomError::Summary(Box::new(error)))
            }
        }
    }

    /// Describe the room at the given URI with the space hierarchy
    /// endpoint.
    ///
    /// This endpoint should work for any room already known by the
    /// homeserver.
    async fn remote_room_from_space_hierarchy(
        &self,
        uri: MatrixRoomIdUri,
    ) -> Result<RemoteRoom, RemoteRoomError> {
        let client = self.client();

        // The endpoint only works with a room ID.
        let room_id = match OwnedRoomId::try_from(uri.id.clone()) {
            Ok(room_id) => room_id,
            Err(alias) => {
                let client = client.clone();
                let handle = spawn_tokio!(async move { client.resolve_room_alias(&alias).await });

                match handle.await.expect("task was not aborted") {
                    Ok(response) => response.room_id,
                    Err(error) => {
                        warn!("Could not resolve room alias `{}`: {error}", uri.id);
                        return Err(RemoteRoomError::ResolveAlias(Box::new(error)));
                    }
                }
            }
        };

        let request = assign!(get_hierarchy::v1::Request::new(room_id.clone()), {
            // We are only interested in the single room.
            limit: Some(uint!(1))
        });
        let handle = spawn_tokio!(async move { client.send(request).await });

        match handle.await.expect("task was not aborted") {
            Ok(response) => response
                .rooms
                .into_iter()
                .next()
                .filter(|chunk| chunk.summary.room_id == room_id)
                .map(|chunk| RemoteRoom::with_data(uri, chunk.summary))
                .ok_or_else(|| {
                    debug!("Space hierarchy endpoint did not return requested room");
                    RemoteRoomError::NotDescribed
                }),
            Err(error) => {
                warn!(
                    "Could not get room details from space hierarchy endpoint for room `{}`: {error}",
                    uri.id
                );
                Err(RemoteRoomError::Hierarchy(Box::new(error)))
            }
        }
    }
}

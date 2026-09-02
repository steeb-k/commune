//! The aliases of a room, headless.
//!
//! The application's `RoomAliases`: the canonical and alternative aliases
//! following the room info, and the edits the addresses subpage makes —
//! every one of them reading the `m.room.canonical_alias` event from the
//! store, changing it exactly as the application does, and sending it back
//! whole — plus the local aliases registered on the homeserver. The
//! application's `Result<(), ()>` refusals are named here; its two
//! specialised errors are variants of the one enum.

use eyeball::{SharedObservable, Subscriber};
use matrix_sdk::{
    deserialized_responses::RawSyncOrStrippedState, reqwest::StatusCode, room::Room as MatrixRoom,
};
use ruma::{
    OwnedRoomAliasId, RoomAliasId,
    api::client::{
        alias::{create_alias, delete_alias},
        room,
    },
    events::{SyncStateEvent, room::canonical_alias::RoomCanonicalAliasEventContent},
};
use tracing::error;

use crate::{UserFacingError, spawn_tokio};

/// An error encountered while changing the aliases of a room.
#[derive(Debug, thiserror::Error)]
pub enum AliasError {
    /// The change would change nothing: the alias is already, or is not,
    /// where it was asked to be.
    ///
    /// The application refuses these rather than sending an event that
    /// changes nothing.
    #[error("the aliases already are as asked")]
    NothingToDo,
    /// The alias is not registered.
    #[error("the alias is not registered")]
    NotRegistered,
    /// The alias is not registered to this room.
    #[error("the alias points to another room")]
    OtherRoom,
    /// The alias is already registered.
    #[error("the alias is already registered")]
    AlreadyInUse,
    /// The canonical alias event could not be read.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` here expensive.
    #[error(transparent)]
    Read(Box<matrix_sdk::Error>),
    /// The request failed.
    #[error(transparent)]
    Server(Box<matrix_sdk::Error>),
}

impl UserFacingError for AliasError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NotRegistered => "This address is not registered as a local address".to_owned(),
            Self::OtherRoom => "This address does not belong to this room".to_owned(),
            Self::AlreadyInUse => "This address is already registered".to_owned(),
            // The application's toasts name the action; an embedder that
            // knows which one it took says so itself.
            Self::NothingToDo | Self::Read(_) | Self::Server(_) => {
                "Could not update the addresses".to_owned()
            }
        }
    }
}

/// The aliases of a room, as the room info has them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AliasesState {
    /// The canonical alias.
    pub canonical_alias: Option<OwnedRoomAliasId>,
    /// The other aliases.
    pub alt_aliases: Vec<OwnedRoomAliasId>,
}

impl AliasesState {
    /// The main alias.
    ///
    /// This is the canonical alias if there is one, or the first of the alt
    /// aliases.
    #[must_use]
    pub fn alias(&self) -> Option<OwnedRoomAliasId> {
        self.canonical_alias
            .clone()
            .or_else(|| self.alt_aliases.first().cloned())
    }
}

/// Aliases of a room.
#[derive(Debug)]
pub struct RoomAliases {
    /// The room API of the SDK.
    matrix_room: MatrixRoom,
    /// The current aliases.
    state: SharedObservable<AliasesState>,
}

impl RoomAliases {
    /// Create the aliases of the given room.
    pub(crate) fn new(matrix_room: MatrixRoom) -> Self {
        Self {
            matrix_room,
            state: SharedObservable::new(AliasesState::default()),
        }
    }

    /// The current aliases.
    #[must_use]
    pub fn state(&self) -> AliasesState {
        self.state.get()
    }

    /// Subscribe to the aliases.
    pub fn subscribe(&self) -> Subscriber<AliasesState> {
        self.state.subscribe()
    }

    /// The canonical alias.
    #[must_use]
    pub fn canonical_alias(&self) -> Option<OwnedRoomAliasId> {
        self.state.get().canonical_alias
    }

    /// The other public aliases.
    #[must_use]
    pub fn alt_aliases(&self) -> Vec<OwnedRoomAliasId> {
        self.state.get().alt_aliases
    }

    /// The main alias.
    ///
    /// This is the canonical alias if there is one, or the first of the alt
    /// aliases.
    #[must_use]
    pub fn alias(&self) -> Option<OwnedRoomAliasId> {
        self.state.get().alias()
    }

    /// Update the aliases with the SDK data.
    pub(crate) fn update(&self) {
        self.state.set_if_not_eq(AliasesState {
            canonical_alias: self.matrix_room.canonical_alias(),
            alt_aliases: self.matrix_room.alt_aliases(),
        });
    }

    /// Get the content of the canonical alias event from the store.
    async fn canonical_alias_event_content(
        &self,
    ) -> Result<Option<RoomCanonicalAliasEventContent>, AliasError> {
        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            matrix_room
                .get_state_event_static::<RoomCanonicalAliasEventContent>()
                .await
        });

        let raw_event = match handle.await.expect("task was not aborted") {
            Ok(Some(RawSyncOrStrippedState::Sync(raw_event))) => raw_event,
            // We shouldn't need to load this in an invited room.
            Ok(_) => return Ok(None),
            Err(read_error) => {
                error!("Could not get canonical alias event: {read_error}");
                return Err(AliasError::Read(Box::new(read_error)));
            }
        };

        match raw_event.deserialize() {
            Ok(SyncStateEvent::Original(event)) => Ok(Some(event.content)),
            // The redacted event doesn't have a content.
            Ok(_) => Ok(None),
            Err(deserialize_error) => {
                error!("Could not deserialize canonical alias event: {deserialize_error}");
                Err(AliasError::Read(Box::new(deserialize_error.into())))
            }
        }
    }

    /// Send the given canonical alias content, whole.
    async fn send_canonical_alias_event(
        &self,
        event_content: RoomCanonicalAliasEventContent,
        action: &'static str,
    ) -> Result<(), AliasError> {
        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(event_content).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|send_error| {
                error!("Could not {action}: {send_error}");
                AliasError::Server(Box::new(send_error))
            })
    }

    /// Remove the given canonical alias.
    ///
    /// Checks that the canonical alias is the correct one before
    /// proceeding.
    pub async fn remove_canonical_alias(&self, alias: &RoomAliasId) -> Result<(), AliasError> {
        let mut event_content = self
            .canonical_alias_event_content()
            .await?
            .unwrap_or_default();

        // Remove the canonical alias, if it is there.
        if event_content.alias.take().is_none_or(|a| a != alias) {
            // Nothing to do.
            return Err(AliasError::NothingToDo);
        }

        self.send_canonical_alias_event(event_content, "remove canonical alias")
            .await
    }

    /// Set the given alias to be the canonical alias.
    ///
    /// Removes the given alias from the alt aliases if it is in the list.
    pub async fn set_canonical_alias(&self, alias: OwnedRoomAliasId) -> Result<(), AliasError> {
        let mut event_content = self
            .canonical_alias_event_content()
            .await?
            .unwrap_or_default();

        if event_content.alias.as_ref().is_some_and(|a| *a == alias) {
            // Nothing to do.
            return Err(AliasError::NothingToDo);
        }

        // Remove from the alt aliases, if it is there.
        let alt_alias_pos = event_content.alt_aliases.iter().position(|a| *a == alias);
        if let Some(pos) = alt_alias_pos {
            event_content.alt_aliases.remove(pos);
        }

        // Set as canonical alias.
        if let Some(old_canonical) = event_content.alias.replace(alias) {
            // Move the old canonical alias to the alt aliases, if it is not
            // there already.
            let has_old_canonical = event_content.alt_aliases.contains(&old_canonical);

            if !has_old_canonical {
                event_content.alt_aliases.push(old_canonical);
            }
        }

        self.send_canonical_alias_event(event_content, "set canonical alias")
            .await
    }

    /// Remove the given alt alias.
    ///
    /// Checks that it is in the list of alt aliases before proceeding.
    pub async fn remove_alt_alias(&self, alias: &RoomAliasId) -> Result<(), AliasError> {
        let mut event_content = self
            .canonical_alias_event_content()
            .await?
            .unwrap_or_default();

        // Remove from the alt aliases, if it is there.
        let alt_alias_pos = event_content.alt_aliases.iter().position(|a| a == alias);
        if let Some(pos) = alt_alias_pos {
            event_content.alt_aliases.remove(pos);
        } else {
            // Nothing to do.
            return Err(AliasError::NothingToDo);
        }

        self.send_canonical_alias_event(event_content, "remove alt alias")
            .await
    }

    /// Set the given alias to be an alt alias.
    ///
    /// The alias must be registered, and to this room.
    pub async fn add_alt_alias(&self, alias: OwnedRoomAliasId) -> Result<(), AliasError> {
        let mut event_content = self
            .canonical_alias_event_content()
            .await?
            .unwrap_or_default();

        // Do nothing if it is already present.
        if event_content.alias.as_ref().is_some_and(|a| *a == alias)
            || event_content.alt_aliases.contains(&alias)
        {
            error!("Cannot add alias already listed");
            return Err(AliasError::NothingToDo);
        }

        // Check that the alias exists and points to the proper room.
        let client = self.matrix_room.client();
        let alias_clone = alias.clone();
        let handle = spawn_tokio!(async move { client.resolve_room_alias(&alias_clone).await });

        match handle.await.expect("task was not aborted") {
            Ok(response) => {
                if response.room_id != self.matrix_room.room_id() {
                    error!("Cannot add alias that points to other room");
                    return Err(AliasError::OtherRoom);
                }
            }
            Err(resolve_error) => {
                error!("Could not check room alias: {resolve_error}");
                if resolve_error
                    .as_client_api_error()
                    .is_some_and(|e| e.status_code == StatusCode::NOT_FOUND)
                {
                    return Err(AliasError::NotRegistered);
                }

                return Err(AliasError::Server(Box::new(resolve_error.into())));
            }
        }

        // Add as alt alias.
        event_content.alt_aliases.push(alias);

        self.send_canonical_alias_event(event_content, "add alt alias")
            .await
    }

    /// Get the local aliases registered on the homeserver.
    pub async fn local_aliases(&self) -> Result<Vec<OwnedRoomAliasId>, AliasError> {
        let client = self.matrix_room.client();
        let room_id = self.matrix_room.room_id().to_owned();

        let handle =
            spawn_tokio!(
                async move { client.send(room::aliases::v3::Request::new(room_id)).await }
            );

        match handle.await.expect("task was not aborted") {
            Ok(response) => Ok(response.aliases),
            Err(fetch_error) => {
                error!("Could not fetch local room aliases: {fetch_error}");
                Err(AliasError::Server(Box::new(fetch_error.into())))
            }
        }
    }

    /// Unregister the given local alias.
    pub async fn unregister_local_alias(&self, alias: OwnedRoomAliasId) -> Result<(), AliasError> {
        let client = self.matrix_room.client();

        let request = delete_alias::v3::Request::new(alias);
        let handle = spawn_tokio!(async move { client.send(request).await });

        match handle.await.expect("task was not aborted") {
            Ok(_response) => Ok(()),
            Err(delete_error) => {
                error!("Could not unregister local alias: {delete_error}");
                Err(AliasError::Server(Box::new(delete_error.into())))
            }
        }
    }

    /// Register the given local alias.
    pub async fn register_local_alias(&self, alias: OwnedRoomAliasId) -> Result<(), AliasError> {
        let client = self.matrix_room.client();
        let room_id = self.matrix_room.room_id().to_owned();

        let request = create_alias::v3::Request::new(alias, room_id);
        let handle = spawn_tokio!(async move { client.send(request).await });

        match handle.await.expect("task was not aborted") {
            Ok(_response) => Ok(()),
            Err(create_error) => {
                error!("Could not register local alias: {create_error}");

                if create_error
                    .as_client_api_error()
                    .is_some_and(|e| e.status_code == StatusCode::CONFLICT)
                {
                    Err(AliasError::AlreadyInUse)
                } else {
                    Err(AliasError::Server(Box::new(create_error.into())))
                }
            }
        }
    }
}

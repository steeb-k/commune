//! Creating a room or a space.
//!
//! The request half of the application's `CreateRoomDialog`
//! (`src/session_view/create_room_dialog.rs`): what the dialog builds from
//! its form and what it makes of the answer. What stayed in the
//! application is the form and its validation — the address rules, the
//! "already taken" marking of the entry — because those are widgets and
//! sentences.

use matrix_sdk::Error;
use ruma::{
    OwnedRoomId,
    api::{
        client::room::{
            Visibility,
            create_room::{self, RoomPowerLevelsContentOverride, v3::CreationContent},
        },
        error::ErrorKind,
    },
    assign,
    events::{InitialStateEvent, TimelineEventType, room::encryption::RoomEncryptionEventContent},
    room::RoomType,
    serde::Raw,
};
use tracing::error;

use super::Session;
use crate::{UserFacingError, spawn_tokio};

/// What can go wrong while creating a room.
#[derive(Debug, thiserror::Error)]
pub enum CreateRoomError {
    /// The public address asked for is already taken.
    #[error("the address is already taken")]
    AddressTaken,
    /// The homeserver refused for another reason.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(Box<Error>),
}

impl UserFacingError for CreateRoomError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::AddressTaken => "The address is already taken.".to_owned(),
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// Who can join the room being created, and what follows from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateRoomVisibility {
    /// Only invited people can join.
    Private {
        /// Whether to encrypt the room from birth.
        ///
        /// Ignored for a space: its timeline is never drawn.
        encrypted: bool,
    },
    /// Anyone can find and join, under the given address on our
    /// homeserver — the local part, without `#` or the server name.
    Public {
        /// The local part of the room's alias.
        address: String,
    },
}

/// What to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRoomOptions {
    /// The name of the room.
    pub name: Option<String>,
    /// The topic of the room.
    pub topic: Option<String>,
    /// Whether a space is being made rather than a room.
    pub is_space: bool,
    /// Who can join.
    pub visibility: CreateRoomVisibility,
}

impl CreateRoomOptions {
    /// Convert these options to a request.
    fn into_request(self) -> create_room::v3::Request {
        let mut request = assign!(
            create_room::v3::Request::new(),
            {
                name: self.name.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(ToOwned::to_owned),
                topic: self.topic.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(ToOwned::to_owned),
            }
        );

        if self.is_space {
            // The one thing that makes a space a space.
            let mut creation_content = CreationContent::new();
            creation_content.room_type = Some(RoomType::Space);
            request.creation_content = Raw::new(&creation_content).ok();

            // A space is a room, and a room nobody has raised the bar in
            // is a room anybody can post to. Its timeline is never drawn,
            // so a message sent into it is a message nobody will ever see
            // — and the state events that make it a space are exactly what
            // should not be writable by everyone who joins.
            let mut power_levels = RoomPowerLevelsContentOverride::default();
            power_levels.events_default = Some(100.into());
            power_levels.events = [
                (TimelineEventType::SpaceChild, 50.into()),
                (TimelineEventType::RoomAvatar, 50.into()),
                (TimelineEventType::RoomName, 50.into()),
                (TimelineEventType::RoomTopic, 50.into()),
            ]
            .into();
            request.power_level_content_override = Raw::new(&power_levels).ok();
        }

        match self.visibility {
            CreateRoomVisibility::Private { encrypted } => {
                request.visibility = Visibility::Private;

                if !self.is_space && encrypted {
                    let event = InitialStateEvent::with_empty_state_key(
                        RoomEncryptionEventContent::with_recommended_defaults(),
                    );
                    request.initial_state = vec![event.to_raw_any()];
                }
            }
            CreateRoomVisibility::Public { address } => {
                request.visibility = Visibility::Public;
                request.room_alias_name = Some(address.trim().to_owned());
            }
        }

        request
    }
}

impl Session {
    /// Create a room or a space, returning its ID.
    ///
    /// The room reaches the room list with the next sync;
    /// `RoomList::get_wait` is how the application waits for it.
    pub async fn create_room(
        &self,
        options: CreateRoomOptions,
    ) -> Result<OwnedRoomId, CreateRoomError> {
        let is_space = options.is_space;
        let request = options.into_request();

        let client = self.client();
        let handle = spawn_tokio!(async move { client.create_room(request).await });

        match handle.await.expect("task was not aborted") {
            Ok(matrix_room) => Ok(matrix_room.room_id().to_owned()),
            Err(create_error) => {
                if is_space {
                    error!("Could not create a new space: {create_error}");
                } else {
                    error!("Could not create a new room: {create_error}");
                }

                // Handle the room address already taken error.
                if create_error
                    .client_api_error_kind()
                    .is_some_and(|kind| *kind == ErrorKind::RoomInUse)
                {
                    return Err(CreateRoomError::AddressTaken);
                }

                Err(CreateRoomError::Server(Box::new(create_error)))
            }
        }
    }
}

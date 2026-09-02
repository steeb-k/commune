//! The last messages of a room that can be read without joining it.
//!
//! This is a peek, in the sense of the Matrix specification: the room's
//! `m.room.history_visibility` is `world_readable`, so `/messages` answers
//! for somebody who is not a member. Nothing here syncs, nothing here
//! paginates, and nothing here can be replied to.
//!
//! The value half of the application's `RoomPeek` and `PeekedMessage`
//! (`src/session/remote/room_peek.rs`). What stayed in the application is
//! the `gio::ListStore` of rows and the abort handle that drops an answer
//! about a room the dialog has since moved on from.

use std::collections::HashMap;

use ruma::{
    MilliSecondsSinceUnixEpoch, OwnedUserId, RoomId,
    api::client::{
        filter::{LazyLoadOptions, RoomEventFilter},
        message::get_message_events,
    },
    assign,
    events::{
        AnyStateEvent, AnySyncTimelineEvent, MessageLikeEventType,
        room::message::OriginalSyncRoomMessageEvent,
    },
    serde::Raw,
};
use tracing::debug;

use crate::{
    UserFacingError, matrix::original_message_event_from_raw, session::Session, spawn_tokio,
};

/// How many messages to show.
///
/// This is a taste of the room, not its history: there is no scrollback, so
/// the number is whatever fills a dialog and stops.
const PEEK_LIMIT: u32 = 20;

/// What can go wrong while peeking at a room.
#[derive(Debug, thiserror::Error)]
pub enum RoomPeekError {
    /// The homeserver refused, or does not have the room.
    ///
    /// A room that is not `world_readable` answers this way, which is the
    /// expected outcome for most rooms. Boxed because `matrix_sdk::HttpError`
    /// is large enough that carrying it by value makes every `Result` in
    /// this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::HttpError>),
}

impl UserFacingError for RoomPeekError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// One message read from a room that was not joined.
#[derive(Debug, Clone)]
pub struct PeekedMessage {
    /// The Matrix event.
    event: OriginalSyncRoomMessageEvent,
    /// The display name of the sender, if it is known and unambiguous.
    sender_name: Option<String>,
}

impl PeekedMessage {
    /// The Matrix event.
    #[must_use]
    pub fn event(&self) -> &OriginalSyncRoomMessageEvent {
        &self.event
    }

    /// The name to present the sender of this message under.
    ///
    /// Falls back to the user ID, which is what a room whose member list we
    /// cannot read leaves us with.
    #[must_use]
    pub fn sender_name(&self) -> String {
        self.sender_name
            .clone()
            .unwrap_or_else(|| self.event.sender.to_string())
    }

    /// The timestamp of this message.
    #[must_use]
    pub fn timestamp(&self) -> MilliSecondsSinceUnixEpoch {
        self.event.origin_server_ts
    }

    /// The textual content of this message.
    #[must_use]
    pub fn body(&self) -> &str {
        self.event.content.msgtype.body()
    }
}

impl Session {
    /// Read the last messages of the given room without joining it, oldest
    /// first.
    pub async fn peek_room(&self, room_id: &RoomId) -> Result<Vec<PeekedMessage>, RoomPeekError> {
        // Only messages, and only the member events of the people who sent
        // them: a preview has no room for state changes, and lazy-loading
        // is the only way to learn a sender's name without being able to
        // ask for the member list of a room we are not in.
        let filter = assign!(RoomEventFilter::default(), {
            types: Some(vec![MessageLikeEventType::RoomMessage.to_string()]),
            lazy_load_options: LazyLoadOptions::Enabled {
                include_redundant_members: false,
            },
        });
        let request = assign!(get_message_events::v3::Request::backward(room_id.to_owned()), {
            limit: PEEK_LIMIT.into(),
            filter,
        });

        let client = self.client();
        let handle = spawn_tokio!(async move { client.send(request).await });

        let response = handle
            .await
            .expect("task was not aborted")
            .map_err(|peek_error| {
                // A room that is not `world_readable`, or that this
                // homeserver does not have, answers with an error here.
                // This is the expected outcome for most rooms.
                debug!("Could not read the messages of room `{room_id}`: {peek_error}");
                RoomPeekError::Server(Box::new(peek_error))
            })?;

        let names = sender_names(&response.state);

        // The request walks backwards from the end of the room, so the
        // response is newest first and the list wants the opposite.
        let messages = response
            .chunk
            .iter()
            .rev()
            .filter_map(|raw| {
                // The JSON of a timeline event is the JSON of a sync
                // timeline event plus a `room_id`, so it deserializes as
                // one. There is no `JsonCastable` for the pair.
                let event = original_message_event_from_raw(
                    raw.cast_ref_unchecked::<AnySyncTimelineEvent>(),
                )?;
                let sender_name = names.get(&event.sender).cloned();

                Some(PeekedMessage { event, sender_name })
            })
            .collect::<Vec<_>>();

        if messages.is_empty() {
            debug!("Nothing readable in the last messages of room `{room_id}`");
        }

        Ok(messages)
    }
}

/// The display name to use for each sender named by the given member
/// events.
///
/// A name shared by two people is not used for either of them, as the spec
/// requires: in a room we cannot see the member list of, the user ID is the
/// only thing left that tells them apart.
fn sender_names(state: &[Raw<AnyStateEvent>]) -> HashMap<OwnedUserId, String> {
    let mut names = HashMap::new();
    let mut counts: HashMap<String, usize> = HashMap::new();

    for raw_event in state {
        let Ok(AnyStateEvent::RoomMember(event)) = raw_event.deserialize() else {
            continue;
        };
        let Some(event) = event.as_original() else {
            continue;
        };
        let Some(name) = event
            .content
            .displayname
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };

        *counts.entry(name.to_owned()).or_default() += 1;
        names.insert(event.state_key.clone(), name.to_owned());
    }

    names.retain(|_, name| counts.get(name).copied().unwrap_or_default() == 1);
    names
}

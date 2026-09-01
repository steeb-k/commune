//! The media history of a room: its images, videos, files and audio,
//! newest first.
//!
//! The headless counterpart of the application's `HistoryViewerTimeline`
//! and `HistoryViewerEvent` (`src/session_view/room_details/history_viewer/`):
//! the same request with the same filter, and the same rule for which
//! messages are on which page. The pagination token and the "reached the
//! start" flag stay with the caller, because the application's timeline
//! object keeps them beside its `GListModel` and the FFI hands the token
//! across as a value.

use matrix_sdk::room::MessagesOptions;
use ruma::{
    OwnedEventId, UInt,
    api::client::filter::{RoomEventFilter, UrlFilter},
    assign,
    events::{
        MessageLikeEventType,
        room::message::{MessageType, OriginalSyncRoomMessageEvent},
    },
};

use crate::{UserFacingError, matrix::original_message_event_from_raw, spawn_tokio};

/// The number of events requested per page, as the application requests
/// them.
const PAGE_SIZE: u32 = 20;

/// What can go wrong while loading the media history.
#[derive(Debug, thiserror::Error)]
pub enum MediaHistoryError {
    /// The homeserver could not load the events.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl UserFacingError for MediaHistoryError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// The types of events that can be displayed in the history viewers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaHistoryKind {
    /// A file.
    File,
    /// An image or a video.
    Media,
    /// An audio file.
    Audio,
}

impl MediaHistoryKind {
    /// The kind for the given message, if it belongs in a history viewer.
    #[must_use]
    pub fn with_msgtype(msgtype: &MessageType) -> Option<Self> {
        let kind = match msgtype {
            MessageType::Audio(_) => Self::Audio,
            MessageType::File(_) => Self::File,
            MessageType::Image(_) | MessageType::Video(_) => Self::Media,
            _ => return None,
        };

        Some(kind)
    }
}

/// An event in the media history.
#[derive(Debug, Clone)]
pub struct MediaHistoryEvent {
    event: OriginalSyncRoomMessageEvent,
    kind: MediaHistoryKind,
}

impl MediaHistoryEvent {
    /// Construct a `MediaHistoryEvent` for the given raw event, if it is
    /// viewable in one of the history viewers.
    fn try_new(raw: &ruma::serde::Raw<ruma::events::AnySyncTimelineEvent>) -> Option<Self> {
        let event = original_message_event_from_raw(raw)?;
        let kind = MediaHistoryKind::with_msgtype(&event.content.msgtype)?;

        Some(Self { event, kind })
    }

    /// The Matrix event.
    #[must_use]
    pub fn event(&self) -> &OriginalSyncRoomMessageEvent {
        &self.event
    }

    /// The ID of the event.
    #[must_use]
    pub fn event_id(&self) -> OwnedEventId {
        self.event.event_id.clone()
    }

    /// The page this event belongs on.
    #[must_use]
    pub fn kind(&self) -> MediaHistoryKind {
        self.kind
    }
}

/// One page of the media history.
#[derive(Debug, Clone)]
pub struct MediaHistoryPage {
    /// The media events of this page, newest first.
    pub events: Vec<MediaHistoryEvent>,
    /// The token to request the next page with.
    ///
    /// The homeserver omits it when no further events are available.
    pub end: Option<String>,
}

/// Load one page of the media history of the given room, going backwards
/// from the given token, or from the end of the room without one.
pub(super) async fn load_page(
    matrix_room: &matrix_sdk::room::Room,
    is_encrypted: bool,
    from: Option<&str>,
) -> Result<MediaHistoryPage, MediaHistoryError> {
    let matrix_room = matrix_room.clone();
    let from = from.map(ToOwned::to_owned);

    let handle = spawn_tokio!(async move {
        // If the room is encrypted, the messages content cannot be filtered
        // with URLs.
        let filter = if is_encrypted {
            let filter_types = vec![
                MessageLikeEventType::RoomEncrypted.to_string(),
                MessageLikeEventType::RoomMessage.to_string(),
            ];
            assign!(RoomEventFilter::default(), {
                types: Some(filter_types),
            })
        } else {
            let filter_types = vec![MessageLikeEventType::RoomMessage.to_string()];
            assign!(RoomEventFilter::default(), {
                types: Some(filter_types),
                url_filter: Some(UrlFilter::EventsWithUrl),
            })
        };
        let options = assign!(MessagesOptions::backward().from(from.as_deref()), {
            limit: UInt::from(PAGE_SIZE),
            filter,
        });

        matrix_room.messages(options).await
    });

    let response = handle
        .await
        .expect("task was not aborted")
        .map_err(Box::new)?;

    let events = response
        .chunk
        .iter()
        .filter_map(|event| MediaHistoryEvent::try_new(event.raw()))
        .collect();

    Ok(MediaHistoryPage {
        events,
        end: response.end,
    })
}

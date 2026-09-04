//! The one event a push names, fetched and decrypted on the device.
//!
//! This is the application's `utils::android_push::show_pushed_event` and
//! `fetch_pushed_event` without the GTK around them. A push for an
//! encrypted room carries ciphertext, so what a notification says has to
//! come from the event itself, read back through the SDK's notification
//! client — one `/context` request, decryption retried — and worded by the
//! embedder from the [`NotificationBody`] the core extracts.

use std::time::Duration;

use matrix_sdk_ui::notification_client::{
    NotificationClient, NotificationEvent, NotificationProcessSetup, NotificationStatus,
};
use ruma::{OwnedEventId, OwnedRoomId, events::AnyStrippedStateEvent};
use tracing::{debug, warn};

use super::body::NotificationBody;
use crate::{matrix::AnySyncOrStrippedTimelineEvent, session::Session, session_list::SessionList};

/// How long to wait for the sessions to restore in a process the push
/// itself started: sixteen half-seconds, well inside the ten seconds the
/// freezer allows a woken process.
const SESSION_WAIT_ROUNDS: u32 = 16;
/// The pause between two looks at the session list.
const SESSION_WAIT_STEP: Duration = Duration::from_millis(500);
/// The bound on the fetch: a process frozen mid-fetch comes back with its
/// sockets dead, and a request with no timeout of its own hangs forever.
const FETCH_TIMEOUT: Duration = Duration::from_secs(25);

/// What a pushed event says, once fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushedNotification {
    /// The name of the room, as the SDK computes it.
    pub room_name: String,
    /// The name of the sender, disambiguated with the user ID when another
    /// member shares it, or the localpart when there is none.
    pub sender_name: String,
    /// The ID of the sender.
    pub sender_id: String,
    /// Whether our own user sent the event.
    pub is_own: bool,
    /// Whether the room is a direct chat.
    pub is_direct: bool,
    /// What the notification says.
    pub body: NotificationBody,
    /// When the event was sent, in milliseconds since the Unix epoch;
    /// zero for an invite, which carries no timestamp.
    pub timestamp: u64,
}

/// Fetch, and where needed decrypt, the one event a push names, and say
/// what a notification for it would say.
///
/// `None` means the event should not become a notification: no session
/// knows the room, the push rules filtered it out, it was redacted or is
/// gone, it is a call invite (the calls module rings for those), or it is
/// of a kind a notification cannot word.
pub async fn fetch_pushed_event(
    session_list: &SessionList,
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
) -> Option<PushedNotification> {
    // Find the session that knows the room. In a process the push itself
    // started — the ordinary case — the sessions are still restoring, so
    // this waits for them, bounded.
    let mut found = None;
    for _ in 0..SESSION_WAIT_ROUNDS {
        found = session_list
            .ready_sessions()
            .into_iter()
            .find(|session| session.client().get_room(&room_id).is_some());
        if found.is_some() {
            break;
        }
        tokio::time::sleep(SESSION_WAIT_STEP).await;
    }

    let Some(session) = found else {
        warn!("No session knows the room of a pushed event");
        return None;
    };
    debug!(
        session = session.session_id(),
        "Fetching a pushed event for a session"
    );

    let fetched =
        tokio::time::timeout(FETCH_TIMEOUT, fetch_item(&session, room_id, event_id)).await;

    let item = match fetched {
        Ok(Ok(Some(item))) => item,
        Ok(Ok(None)) => return None,
        Ok(Err(error)) => {
            warn!("Could not fetch a pushed event: {error}");
            return None;
        }
        Err(_) => {
            warn!("Fetching a pushed event timed out");
            return None;
        }
    };

    let event = match item.event {
        NotificationEvent::Timeline(event) => AnySyncOrStrippedTimelineEvent::Sync(event),
        NotificationEvent::Invite(event) => AnySyncOrStrippedTimelineEvent::Stripped(Box::new(
            AnyStrippedStateEvent::RoomMember(*event),
        )),
    };

    if NotificationBody::is_call_invite(&event) {
        // The calls module is the one that rings, notifies and withdraws.
        return None;
    }

    let sender_id = event.sender();
    let sender_name = match &item.sender_display_name {
        Some(name) if item.is_sender_name_ambiguous => format!("{name} ({sender_id})"),
        Some(name) => name.clone(),
        None => sender_id.localpart().to_owned(),
    };

    let Some(body) = NotificationBody::of(&event, session.user_id()) else {
        debug!("Received push for event of unexpected type {event:?}");
        return None;
    };

    let timestamp = match &event {
        AnySyncOrStrippedTimelineEvent::Sync(ev) => ev.origin_server_ts().0.into(),
        AnySyncOrStrippedTimelineEvent::Stripped(_) => 0,
    };

    Some(PushedNotification {
        room_name: item.room_computed_display_name,
        sender_name,
        sender_id: sender_id.to_string(),
        is_own: sender_id == session.user_id(),
        is_direct: item.is_direct_message_room,
        body,
        timestamp,
    })
}

/// The SDK's item for the event, or `None` if it should not become a
/// notification.
///
/// On the process setup: `SingleProcess` wants the SDK's own sync service,
/// which the core does not run — it has its own sync loop.
/// `MultipleProcesses` is the constructible truth: a cross-process store
/// lock that nothing here ever contends, since the push wake and the
/// application share one process.
async fn fetch_item(
    session: &Session,
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
) -> Result<
    Option<matrix_sdk_ui::notification_client::NotificationItem>,
    matrix_sdk_ui::notification_client::Error,
> {
    let notification_client = NotificationClient::new(
        session.client(),
        NotificationProcessSetup::MultipleProcesses,
    )
    .await?;

    // Not `get_notification()`: that tries a short-lived sliding sync
    // first, which is more requests and more machinery than the freezer's
    // budget likes, and against a homeserver without sliding sync it errors
    // rather than falling through. `/context` is one request, still retries
    // decryption, and is where the full path falls back to anyway.
    let status = notification_client
        .get_notification_with_context(&room_id, &event_id)
        .await?;

    match status {
        NotificationStatus::Event(item) => Ok(Some(*item)),
        NotificationStatus::EventFilteredOut => {
            debug!("The push rules filtered a pushed event out");
            Ok(None)
        }
        NotificationStatus::EventRedacted => {
            debug!("A pushed event was redacted");
            Ok(None)
        }
        NotificationStatus::EventNotFound => {
            warn!("A pushed event could not be found on the homeserver");
            Ok(None)
        }
    }
}

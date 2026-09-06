//! The timeline of a room, headless.
//!
//! The application's `Timeline` as a thin orchestration of
//! [`matrix_sdk_ui::timeline::Timeline`] whose items and diffs pass straight
//! through to the subscriber — the application's `GListModel` splicing, and
//! therefore the diff minimizer, have nothing to translate for and are not
//! needed here. The event filter is the application's `show_in_timeline`,
//! verbatim; the live, pinned and thread focuses, back-pagination and
//! receipts are the application's; and what the message toolbar sends
//! through the timeline — messages, replies, edits, attachments, voice
//! messages, locations, stickers — is sent from here, with the upload-size
//! preflight the toolbar makes. Since Phase 4's module 8 the timeline
//! focused on a single event is here too, with the forward pagination only
//! it can do, and the application's `Timeline` is a view over this one.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use eyeball::{SharedObservable, Subscriber};
use eyeball_im::{Vector, VectorDiff};
use futures_util::{Stream, StreamExt};
use matrix_sdk::{
    attachment::{
        AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo, BaseVideoInfo, Thumbnail,
    },
    room::edit::EditedContent,
};
use matrix_sdk_ui::timeline::{
    AttachmentConfig, AttachmentSource, EventTimelineItem, RoomExt, Timeline as SdkTimeline,
    TimelineEventFocusThreadMode, TimelineEventItemId, TimelineFocus,
    TimelineItem as SdkTimelineItem, default_event_filter,
};
use ruma::{
    EventId, OwnedEventId, UInt, UserId,
    api::client::receipt::create_receipt::v3::ReceiptType as ApiReceiptType,
    events::{
        AnyMessageLikeEventContent, AnySyncMessageLikeEvent, AnySyncStateEvent,
        AnySyncTimelineEvent, Mentions, SyncMessageLikeEvent, SyncStateEvent,
        room::message::{
            LocationMessageEventContent, MessageType, RoomMessageEventContent,
            RoomMessageEventContentWithoutRelation,
        },
        sticker::StickerEventContent,
        tag::TagName,
    },
    room_version_rules::RoomVersionRules,
};
use tracing::{error, warn};

use super::{RoomCategory, WeakSession};
use crate::{
    RUNTIME, UserFacingError,
    klipy::SelectedGif,
    matrix::{ext_traits::TimelineItemContentExt, media::MediaMessage},
    spawn_tokio,
    utils::{LoadingState, OptionStringExt, format_size},
};

/// The number of events to request when loading more history.
pub const MAX_BATCH_SIZE: u16 = 20;

/// An error encountered while acting on a timeline.
#[derive(Debug, thiserror::Error)]
pub enum TimelineError {
    /// The timeline could not be built.
    #[error("the timeline is not available")]
    NoTimeline,
    /// The event is not in this timeline.
    #[error("the event is not in the timeline")]
    UnknownEvent,
    /// The file is larger than the homeserver accepts.
    ///
    /// The value is the limit, for the embedder to name in its own words
    /// and units: the application formats it with `glib::format_size`
    /// inside a translated sentence.
    #[error("the file is larger than the {max_bytes} bytes the homeserver accepts")]
    UploadTooLarge {
        /// The homeserver's upload limit, in bytes.
        max_bytes: u64,
    },
    /// The file to send could not be read.
    #[error(transparent)]
    Read(#[from] std::io::Error),
    /// The SDK refused or failed to send.
    ///
    /// Boxed because the SDK's error is large enough that carrying it by
    /// value makes every `Result` here expensive.
    #[error(transparent)]
    Send(Box<matrix_sdk_ui::timeline::Error>),
}

impl From<matrix_sdk_ui::timeline::Error> for TimelineError {
    fn from(error: matrix_sdk_ui::timeline::Error) -> Self {
        Self::Send(Box::new(error))
    }
}

impl UserFacingError for TimelineError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::UploadTooLarge { max_bytes } => format!(
                "This file is too large, the homeserver takes up to {}",
                format_size(*max_bytes)
            ),
            Self::UnknownEvent => "The event is not in the timeline".to_owned(),
            Self::Read(_) => "Could not read the file".to_owned(),
            // The application's toasts name the action; an embedder that
            // knows which one it took says so itself.
            Self::NoTimeline | Self::Send(_) => "Could not send the message".to_owned(),
        }
    }
}

/// The maximum size of a GIF that is downloaded to be sent, in bytes.
///
/// `SelectedGif` prefers a variant under a smaller limit but can fall back
/// to a larger one, so this is a guard rather than the preferred size.
pub const MAX_GIF_SIZE: u64 = 16 * 1024 * 1024;

/// An error encountered while sending a GIF.
#[derive(Debug, thiserror::Error)]
pub enum SendGifError {
    /// The timeline could not be built.
    #[error("the timeline is not available")]
    NoTimeline,
    /// The GIF could not be downloaded from the service.
    #[error(transparent)]
    Download(#[from] crate::http::HttpError),
    /// The GIF could not be uploaded to the homeserver.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` here expensive.
    #[error(transparent)]
    Upload(Box<matrix_sdk::Error>),
    /// The sticker carrying the GIF could not be sent.
    #[error(transparent)]
    Send(Box<matrix_sdk_ui::timeline::Error>),
}

impl UserFacingError for SendGifError {
    fn to_user_facing(&self) -> String {
        // The application says the same thing for every step.
        "Could not send GIF".to_owned()
    }
}

/// Upload the given GIF to the homeserver and return the source to refer to it.
///
/// The GIF is encrypted first if it is going to an encrypted room, so a GIF is
/// no less private than any other image sent there.
async fn upload_gif(
    client: &matrix_sdk::Client,
    is_encrypted: bool,
    data: Vec<u8>,
) -> Result<ruma::events::sticker::StickerMediaSource, SendGifError> {
    use ruma::events::sticker::StickerMediaSource;

    if is_encrypted {
        let mut cursor = std::io::Cursor::new(data);
        let file = client
            .upload_encrypted_file(&mut cursor)
            .await
            .map_err(|upload_error| SendGifError::Upload(Box::new(upload_error)))?;

        Ok(StickerMediaSource::Encrypted(Box::new(file)))
    } else {
        let response = client
            .media()
            .upload(&mime::IMAGE_GIF, data, None)
            .await
            .map_err(|upload_error| SendGifError::Upload(Box::new(upload_error)))?;

        Ok(StickerMediaSource::Plain(response.content_uri))
    }
}

/// The timeline of a room.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct Timeline {
    inner: Arc<TimelineInner>,
}

/// What a timeline shows.
#[derive(Debug, Clone)]
pub enum TimelineFocusKind {
    /// The room's live timeline.
    Live,
    /// The thread rooted at the given event.
    Thread {
        /// The thread's root event.
        root: ruma::OwnedEventId,
    },
    /// The room's pinned events.
    Pinned,
    /// The given event and its surroundings.
    ///
    /// Such a timeline is centered on a single event — a search result or
    /// a permalink — and can be paginated in both directions, but it never
    /// receives new events from sync, so it cannot replace the live
    /// timeline of the room.
    Event {
        /// The event the timeline is centered on.
        target: ruma::OwnedEventId,
    },
}

#[derive(Debug)]
struct TimelineInner {
    /// The room API of the SDK.
    matrix_room: matrix_sdk::room::Room,
    /// The session the room belongs to, for what a reaction records.
    session: WeakSession,
    /// What this timeline shows.
    focus: TimelineFocusKind,
    /// The underlying SDK timeline.
    matrix_timeline: tokio::sync::OnceCell<Arc<SdkTimeline>>,
    /// Whether this is the timeline of the server notices room.
    ///
    /// Read by the event filter, which runs off the runtime's main task,
    /// so it cannot ask the room for its category; the room's category
    /// keeps it current instead.
    is_server_notice_room: Arc<AtomicBool>,
    /// The loading state of the timeline.
    state: SharedObservable<LoadingState>,
    /// Whether the start of the room's history has been reached.
    has_reached_start: SharedObservable<bool>,
    /// Whether the end of the room's history has been reached.
    ///
    /// Always true once built, except for a timeline focused on an event,
    /// which is the only one that can be paginated forwards.
    has_reached_end: SharedObservable<bool>,
    /// Whether events are being loaded at the start of the timeline.
    is_loading_start: SharedObservable<bool>,
    /// Whether events are being loaded at the end of the timeline.
    is_loading_end: SharedObservable<bool>,
}

impl TimelineInner {
    /// Forget what was known about the ends of the history.
    ///
    /// The SDK clears or resets its items when the timeline starts over —
    /// after a gap it could not fill, or a focus it rebuilt — and what was
    /// loaded before says nothing about what is loaded now. The pinned
    /// events are still the whole of their timeline, and every timeline
    /// but the one focused on an event is still at the end.
    fn reset_reach(&self) {
        if !matches!(self.focus, TimelineFocusKind::Pinned) {
            self.has_reached_start.set_if_not_eq(false);
        }
        if matches!(self.focus, TimelineFocusKind::Event { .. }) {
            self.has_reached_end.set_if_not_eq(false);
        }
    }
}

/// What the embedder measured about a media file before sending it: the
/// message toolbar's `load_image_info` and `load_video_info` results, as
/// far as the embedder's media stack can produce them. Every field is
/// optional, and a missing one is simply absent from the event's info.
#[derive(Debug, Default)]
pub struct MediaMeasure {
    /// The width of a picture or a video, in pixels.
    pub width: Option<u32>,
    /// The height of a picture or a video, in pixels.
    pub height: Option<u32>,
    /// The duration of a video or an audio file.
    pub duration: Option<Duration>,
    /// The Blurhash of a picture or a video's first frame.
    pub blurhash: Option<String>,
    /// The thumbnail of a picture or a video, uploaded alongside it.
    pub thumbnail: Option<Thumbnail>,
}

impl Timeline {
    /// Create the live timeline of the given room.
    pub(crate) fn new(matrix_room: matrix_sdk::room::Room, session: WeakSession) -> Self {
        Self::with_focus(matrix_room, TimelineFocusKind::Live, session)
    }

    /// Create a timeline of the given room with the given focus.
    pub(crate) fn with_focus(
        matrix_room: matrix_sdk::room::Room,
        focus: TimelineFocusKind,
        session: WeakSession,
    ) -> Self {
        Self {
            inner: Arc::new(TimelineInner {
                matrix_room,
                focus,
                session,
                matrix_timeline: tokio::sync::OnceCell::new(),
                is_server_notice_room: Arc::new(AtomicBool::new(false)),
                state: SharedObservable::new(LoadingState::Initial),
                has_reached_start: SharedObservable::new(false),
                has_reached_end: SharedObservable::new(false),
                is_loading_start: SharedObservable::new(false),
                is_loading_end: SharedObservable::new(false),
            }),
        }
    }

    /// Follow the category of the room, for the event filter.
    ///
    /// The filter hides `m.server_notice` messages outside the server
    /// notices room; the room's tags at build time say whether this is
    /// that room, and the category says so from then on, as the
    /// application's filter followed the room's category.
    pub(crate) fn watch_category(
        &self,
        current: RoomCategory,
        mut categories: Subscriber<RoomCategory>,
    ) {
        self.inner
            .is_server_notice_room
            .store(current == RoomCategory::ServerNotice, Ordering::Relaxed);

        let weak = Arc::downgrade(&self.inner);
        RUNTIME.spawn(async move {
            while let Some(category) = categories.next().await {
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                inner
                    .is_server_notice_room
                    .store(category == RoomCategory::ServerNotice, Ordering::Relaxed);
            }
        });
    }

    /// What this timeline shows.
    #[must_use]
    pub fn focus(&self) -> &TimelineFocusKind {
        &self.inner.focus
    }

    /// The loading state of the timeline.
    #[must_use]
    pub fn state(&self) -> LoadingState {
        self.inner.state.get()
    }

    /// Subscribe to the loading state of the timeline.
    pub fn subscribe_state(&self) -> Subscriber<LoadingState> {
        self.inner.state.subscribe()
    }

    /// Whether the start of the room's history has been reached.
    #[must_use]
    pub fn has_reached_start(&self) -> bool {
        self.inner.has_reached_start.get()
    }

    /// Subscribe to whether the start of the room's history has been
    /// reached.
    pub fn subscribe_has_reached_start(&self) -> Subscriber<bool> {
        self.inner.has_reached_start.subscribe()
    }

    /// Whether the end of the room's history has been reached.
    ///
    /// This is always `true` for the live timeline, which is by definition
    /// at the end of the room's history.
    #[must_use]
    pub fn has_reached_end(&self) -> bool {
        self.inner.has_reached_end.get()
    }

    /// Subscribe to whether the end of the room's history has been
    /// reached.
    pub fn subscribe_has_reached_end(&self) -> Subscriber<bool> {
        self.inner.has_reached_end.subscribe()
    }

    /// Whether events are being loaded at the end of the timeline.
    #[must_use]
    pub fn is_loading_end(&self) -> bool {
        self.inner.is_loading_end.get()
    }

    /// Subscribe to whether events are being loaded at the end of the
    /// timeline.
    pub fn subscribe_is_loading_end(&self) -> Subscriber<bool> {
        self.inner.is_loading_end.subscribe()
    }

    /// The media message of the item with the given unique ID, if it is
    /// one.
    ///
    /// The item is looked up in this timeline so that an encrypted source
    /// comes with its keys — the application's `Event::media_message()`,
    /// reached by ID because that is what crosses the FFI.
    pub async fn media_message(&self, unique_id: &str) -> Option<MediaMessage> {
        use matrix_sdk_ui::timeline::{MsgLikeKind, TimelineItemContent};

        let matrix_timeline = self.matrix_timeline().await?;
        let items = matrix_timeline.items().await;
        let item = items.iter().find(|item| item.unique_id().0 == unique_id)?;
        let event = item.as_event()?;

        let TimelineItemContent::MsgLike(msg_like) = event.content() else {
            return None;
        };
        match &msg_like.kind {
            MsgLikeKind::Message(message) => MediaMessage::from_message(message.msgtype()),
            MsgLikeKind::Sticker(sticker) => Some(sticker.content().clone().into()),
            _ => None,
        }
    }

    /// The underlying SDK timeline, built on first use.
    ///
    /// Returns `None` if it could not be built; the state observable says
    /// so too.
    pub async fn matrix_timeline(&self) -> Option<Arc<SdkTimeline>> {
        let inner = &self.inner;

        inner
            .matrix_timeline
            .get_or_try_init(|| async {
                inner.state.set_if_not_eq(LoadingState::Loading);

                match build_sdk_timeline(
                    inner.matrix_room.clone(),
                    inner.focus.clone(),
                    inner.is_server_notice_room.clone(),
                )
                .await
                {
                    Ok(timeline) => {
                        if matches!(inner.focus, TimelineFocusKind::Pinned) {
                            // The pinned events are the whole of this
                            // timeline. The SDK refuses to paginate it, so
                            // never ask.
                            inner.has_reached_start.set_if_not_eq(true);
                        }
                        if !matches!(inner.focus, TimelineFocusKind::Event { .. }) {
                            // Every other timeline is at the end of the
                            // room's history.
                            inner.has_reached_end.set_if_not_eq(true);
                        }
                        inner.state.set_if_not_eq(LoadingState::Ready);
                        Ok(Arc::new(timeline))
                    }
                    Err(build_error) => {
                        error!("Could not create timeline: {build_error}");
                        inner.state.set_if_not_eq(LoadingState::Error);
                        Err(())
                    }
                }
            })
            .await
            .ok()
            .cloned()
    }

    /// The current items, and the stream of the changes that follow them.
    ///
    /// The items and diffs are the SDK's own, passed through.
    pub async fn subscribe_items(
        &self,
    ) -> Option<(
        Vector<Arc<SdkTimelineItem>>,
        impl Stream<Item = Vec<VectorDiff<Arc<SdkTimelineItem>>>> + use<>,
    )> {
        let matrix_timeline = self.matrix_timeline().await?;

        let timeline = matrix_timeline.clone();
        let handle = spawn_tokio!(async move { timeline.subscribe().await });
        let (values, stream) = handle.await.expect("task was not aborted");

        // A clear or a reset is the SDK starting over, and the ends of the
        // history are unknown again.
        let weak = Arc::downgrade(&self.inner);
        let stream = stream.inspect(move |diffs| {
            if diffs
                .iter()
                .any(|diff| matches!(diff, VectorDiff::Clear | VectorDiff::Reset { .. }))
                && let Some(inner) = weak.upgrade()
            {
                inner.reset_reach();
            }
        });

        Some((values, stream))
    }

    /// Whether events are being loaded at the start of the timeline.
    #[must_use]
    pub fn is_loading_start(&self) -> bool {
        self.inner.is_loading_start.get()
    }

    /// Subscribe to whether events are being loaded at the start of the
    /// timeline.
    pub fn subscribe_is_loading_start(&self) -> Subscriber<bool> {
        self.inner.is_loading_start.subscribe()
    }

    /// Whether more events can be loaded at the start of the timeline with
    /// the current state.
    ///
    /// We do not want to load twice at the same time, and it is useless to
    /// try to load more history before the timeline is ready or if we have
    /// reached the start of the timeline.
    #[must_use]
    pub fn can_paginate_backwards(&self) -> bool {
        self.state() != LoadingState::Initial
            && !self.is_loading_start()
            && !self.has_reached_start()
    }

    /// Load one batch of events at the start of the timeline, if the
    /// current state allows it.
    ///
    /// One batch is what one request for older history is; the
    /// application loads batches until its caller says stop, which is
    /// [`Self::paginate_backwards_while()`]. A failure puts the timeline in
    /// the error state, which the state observable reports.
    pub async fn paginate_backwards(&self) {
        self.paginate_backwards_while(|| false).await;
    }

    /// Load events at the start of the timeline, batch after batch, until
    /// the given function says to stop or the start is reached.
    ///
    /// Nothing is loaded when the current state does not allow it. The
    /// timeline is loading at its start for the whole of the walk, not
    /// for each batch of it.
    pub async fn paginate_backwards_while(&self, mut continue_fn: impl FnMut() -> bool) {
        if !self.can_paginate_backwards() {
            return;
        }
        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return;
        };

        let inner = &self.inner;
        inner.is_loading_start.set_if_not_eq(true);
        inner.state.set_if_not_eq(LoadingState::Loading);

        loop {
            let timeline = matrix_timeline.clone();
            let handle =
                spawn_tokio!(async move { timeline.paginate_backwards(MAX_BATCH_SIZE).await });

            match handle.await.expect("task was not aborted") {
                Ok(reached_start) => {
                    if reached_start {
                        inner.has_reached_start.set_if_not_eq(true);
                        break;
                    }
                }
                Err(paginate_error) => {
                    error!("Could not load timeline: {paginate_error}");
                    inner.state.set_if_not_eq(LoadingState::Error);
                    break;
                }
            }

            if !continue_fn() {
                break;
            }
        }

        inner.is_loading_start.set_if_not_eq(false);
        if inner.state.get() != LoadingState::Error {
            inner.state.set_if_not_eq(LoadingState::Ready);
        }
    }

    /// Whether more events can be loaded at the end of the timeline with
    /// the current state.
    ///
    /// Only a timeline focused on an event can load events forwards: every
    /// other one is already at the end of the room's history.
    #[must_use]
    pub fn can_paginate_forwards(&self) -> bool {
        matches!(self.inner.focus, TimelineFocusKind::Event { .. })
            && self.state() != LoadingState::Initial
            && !self.is_loading_end()
            && !self.has_reached_end()
    }

    /// Load events at the end of the timeline, batch after batch, until
    /// the given function says to stop or the end is reached.
    ///
    /// Nothing is loaded when the current state does not allow it.
    pub async fn paginate_forwards_while(&self, mut continue_fn: impl FnMut() -> bool) {
        if !self.can_paginate_forwards() {
            return;
        }
        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return;
        };

        let inner = &self.inner;
        inner.is_loading_end.set_if_not_eq(true);
        inner.state.set_if_not_eq(LoadingState::Loading);

        loop {
            let timeline = matrix_timeline.clone();
            let handle =
                spawn_tokio!(async move { timeline.paginate_forwards(MAX_BATCH_SIZE).await });

            match handle.await.expect("task was not aborted") {
                Ok(reached_end) => {
                    if reached_end {
                        inner.has_reached_end.set_if_not_eq(true);
                        break;
                    }
                }
                Err(paginate_error) => {
                    error!("Could not load timeline: {paginate_error}");
                    inner.state.set_if_not_eq(LoadingState::Error);
                    break;
                }
            }

            if !continue_fn() {
                break;
            }
        }

        inner.is_loading_end.set_if_not_eq(false);
        if inner.state.get() != LoadingState::Error {
            inner.state.set_if_not_eq(LoadingState::Ready);
        }
    }

    /// Send the given receipt through this timeline, its type already
    /// resolved against the public-read-receipts setting (the room does
    /// that resolution).
    ///
    /// The SDK scopes the receipt to what the timeline shows: sent through
    /// a thread timeline, it is a receipt for that thread, not for the
    /// room.
    pub async fn send_receipt_resolved(
        &self,
        receipt_type: ApiReceiptType,
        position: ReceiptPosition,
    ) {
        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return;
        };

        let handle = spawn_tokio!(async move {
            match position {
                ReceiptPosition::End => matrix_timeline.mark_as_read(receipt_type).await,
                ReceiptPosition::Event(event_id) => {
                    matrix_timeline
                        .send_single_receipt(receipt_type, event_id)
                        .await
                }
            }
        });

        if let Err(receipt_error) = handle.await.expect("task was not aborted") {
            error!("Could not send read receipt: {receipt_error}");
        }
    }

    /// Whether this timeline has unread messages.
    ///
    /// Returns `None` if it is not possible to know, for example if there
    /// are no events in the timeline.
    pub(crate) async fn has_unread_messages(&self) -> Option<bool> {
        let matrix_timeline = self.matrix_timeline().await?;
        let own_user_id = self.inner.matrix_room.own_user_id().to_owned();

        let timeline = matrix_timeline.clone();
        let own_user_id_clone = own_user_id.clone();
        let user_receipt_item = spawn_tokio!(async move {
            timeline
                .latest_user_read_receipt_timeline_event_id(&own_user_id_clone)
                .await
        })
        .await
        .expect("task was not aborted");

        let timeline = matrix_timeline.clone();
        let items = spawn_tokio!(async move { timeline.items().await })
            .await
            .expect("task was not aborted");

        for item in items.iter().rev() {
            let Some(event) = item.as_event() else {
                continue;
            };
            if !event.is_remote_event() {
                continue;
            }

            if user_receipt_item.is_some()
                && event.event_id().map(ToOwned::to_owned) == user_receipt_item
            {
                // The event is the oldest one, we have read it all.
                return Some(false);
            }
            if event.content().counts_as_unread() {
                // There is at least one unread event.
                return Some(true);
            }
        }

        // This should only happen if we do not have a read receipt item in
        // the timeline, and there are not enough events in the timeline to
        // know if there are unread messages.
        None
    }

    /// A stream that fires when our own user's read receipt moves in this
    /// timeline.
    pub async fn subscribe_own_read_receipts(&self) -> Option<impl Stream<Item = ()> + use<>> {
        let matrix_timeline = self.matrix_timeline().await?;

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .subscribe_own_user_read_receipts_changed()
                .await
        });

        Some(handle.await.expect("task was not aborted"))
    }

    /// The latest activity among this timeline's current items, per the
    /// application's `counts_as_activity` rules.
    pub(crate) async fn latest_activity(&self) -> Option<u64> {
        let matrix_timeline = self.matrix_timeline().await?;
        let own_user_id = self.inner.matrix_room.own_user_id().to_owned();

        let items = spawn_tokio!(async move { matrix_timeline.items().await })
            .await
            .expect("task was not aborted");

        for item in items.iter().rev() {
            let Some(event) = item.as_event() else {
                continue;
            };
            if event.is_remote_event() && event.content().counts_as_activity(&own_user_id) {
                return Some(event.timestamp().get().into());
            }
        }

        None
    }

    /// Send the given message through this timeline: the composer's
    /// content, with no relation.
    ///
    /// Sent through a thread timeline, the message carries the thread
    /// relation; the SDK adds it.
    pub async fn send_message(
        &self,
        content: RoomMessageEventContentWithoutRelation,
    ) -> Result<(), TimelineError> {
        let matrix_timeline = self
            .matrix_timeline()
            .await
            .ok_or(TimelineError::NoTimeline)?;

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .send(content.with_relation(None).into())
                .await
        });

        handle
            .await
            .expect("task was not aborted")
            .map(|_send_handle| ())
            .map_err(|send_error| {
                error!("Could not send message: {send_error}");
                TimelineError::from(send_error)
            })
    }

    /// Send the given message as a reply to the given event.
    pub async fn send_reply(
        &self,
        content: RoomMessageEventContentWithoutRelation,
        in_reply_to: OwnedEventId,
    ) -> Result<(), TimelineError> {
        let matrix_timeline = self
            .matrix_timeline()
            .await
            .ok_or(TimelineError::NoTimeline)?;

        let handle =
            spawn_tokio!(async move { matrix_timeline.send_reply(content, in_reply_to).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|send_error| {
                error!("Could not send reply: {send_error}");
                TimelineError::from(send_error)
            })
    }

    /// Replace the content of the given event with the given message.
    ///
    /// The edit event is made by the room and sent through its send queue,
    /// as the message toolbar sends it: the event being edited does not
    /// have to be among the timeline's loaded items.
    pub async fn edit(
        &self,
        event_id: OwnedEventId,
        content: RoomMessageEventContentWithoutRelation,
    ) -> Result<(), TimelineError> {
        let matrix_room = self.inner.matrix_room.clone();

        let handle = spawn_tokio!(async move {
            let full_content = matrix_room
                .make_edit_event(&event_id, EditedContent::RoomMessage(content))
                .await
                .map_err(matrix_sdk_ui::timeline::EditError::from)?;
            matrix_room.send_queue().send(full_content).await?;
            Ok::<(), matrix_sdk_ui::timeline::Error>(())
        });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|edit_error| {
                error!("Could not send edit: {edit_error}");
                TimelineError::from(edit_error)
            })
    }

    /// Toggle the given reaction key on the given event.
    ///
    /// The SDK can only react to an event it knows about, so the event
    /// has to be in this timeline. The application also records an added
    /// emoji among the recently used ones; the core has no account-data
    /// object to record it in yet.
    pub async fn toggle_reaction(
        &self,
        event_id: OwnedEventId,
        key: &str,
    ) -> Result<(), TimelineError> {
        let matrix_timeline = self
            .matrix_timeline()
            .await
            .ok_or(TimelineError::NoTimeline)?;

        let key_clone = key.to_owned();
        let handle = spawn_tokio!(async move {
            matrix_timeline
                .toggle_reaction(&TimelineEventItemId::EventId(event_id), &key_clone)
                .await
        });

        let was_added = handle
            .await
            .expect("task was not aborted")
            .map_err(|toggle_error| {
                error!("Could not toggle reaction: {toggle_error}");
                TimelineError::from(toggle_error)
            })?;

        // Adding a reaction is a use of the emoji; taking one back is not.
        if was_added && let Some(session) = self.inner.session.upgrade() {
            session.global_account_data().record_emoji_use(key).await;
        }

        Ok(())
    }

    /// Send the file at the given path as an attachment.
    ///
    /// The kind of message follows the MIME type, as the message toolbar
    /// decides it: an image, a video, an audio file or a plain file. The
    /// toolbar also measures images, videos and audio — dimensions,
    /// durations, thumbnails — with the desktop's media stack; the core
    /// measures the size alone, and the embedder that has such a stack
    /// hands the rest over as the `measure`, which goes on the wire as the
    /// toolbar's `load_image_info` and `load_video_info` results do.
    pub async fn send_attachment(
        &self,
        path: PathBuf,
        mime: mime::Mime,
        measure: MediaMeasure,
    ) -> Result<(), TimelineError> {
        let size = std::fs::metadata(&path)
            .ok()
            .and_then(|metadata| UInt::new(metadata.len()));
        let MediaMeasure {
            width,
            height,
            duration,
            blurhash,
            thumbnail,
        } = measure;
        let width = width.map(UInt::from);
        let height = height.map(UInt::from);
        let info = match mime.type_() {
            mime::IMAGE => AttachmentInfo::Image(BaseImageInfo {
                width,
                height,
                size,
                blurhash,
                ..Default::default()
            }),
            mime::VIDEO => AttachmentInfo::Video(BaseVideoInfo {
                duration,
                width,
                height,
                size,
                blurhash,
            }),
            mime::AUDIO => AttachmentInfo::Audio(BaseAudioInfo {
                duration,
                size,
                ..Default::default()
            }),
            _ => AttachmentInfo::File(BaseFileInfo { size }),
        };
        // A thumbnail belongs to a picture or a video alone: the toolbar
        // generates one for nothing else, and the SDK would put it on a
        // file message's info where nothing reads it.
        let thumbnail = match mime.type_() {
            mime::IMAGE | mime::VIDEO => thumbnail,
            _ => None,
        };

        self.send_attachment_with(AttachmentSource::File(path), mime, info, thumbnail)
            .await
    }

    /// Send the recording at the given path as a voice message, under the
    /// given file name.
    ///
    /// The recording is read and its file removed, as the message toolbar
    /// does with its own recorder's file; the bytes are sent under the
    /// given name — the application's translated "Voice message" — since
    /// that is what other clients show. The waveform the toolbar measures
    /// is the embedder's to add.
    pub async fn send_voice(
        &self,
        path: PathBuf,
        mime: mime::Mime,
        duration_ms: u64,
        filename: String,
    ) -> Result<(), TimelineError> {
        let bytes = std::fs::read(&path);
        if let Err(remove_error) = std::fs::remove_file(&path) {
            warn!("Could not remove the voice recording file: {remove_error}");
        }
        let bytes = bytes.inspect_err(|read_error| {
            error!("Could not read the voice recording: {read_error}");
        })?;

        let info = AttachmentInfo::Voice(BaseAudioInfo {
            duration: Some(Duration::from_millis(duration_ms)),
            size: u64::try_from(bytes.len()).ok().and_then(UInt::new),
            waveform: None,
        });

        self.send_attachment_with(AttachmentSource::Data { bytes, filename }, mime, info, None)
            .await
    }

    /// Send the given attachment, after asking the homeserver's upload
    /// limit.
    async fn send_attachment_with(
        &self,
        source: AttachmentSource,
        mime: mime::Mime,
        info: AttachmentInfo,
        thumbnail: Option<Thumbnail>,
    ) -> Result<(), TimelineError> {
        let matrix_timeline = self
            .matrix_timeline()
            .await
            .ok_or(TimelineError::NoTimeline)?;

        let size = match &source {
            AttachmentSource::Data { bytes, .. } => u64::try_from(bytes.len()).ok(),
            AttachmentSource::File(path) => {
                std::fs::metadata(path).ok().map(|metadata| metadata.len())
            }
        };
        check_upload_size(&self.inner.matrix_room.client(), size).await?;

        let config = AttachmentConfig {
            info: Some(info),
            thumbnail,
            ..Default::default()
        };

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .send_attachment(source, mime, config)
                .use_send_queue()
                .await
        });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|send_error| {
                error!("Could not send file: {send_error}");
                TimelineError::from(send_error)
            })
    }

    /// Send the given sticker, through the timeline so it gets a local
    /// echo and the send queue like every other message.
    pub async fn send_sticker(&self, content: StickerEventContent) -> Result<(), TimelineError> {
        let matrix_timeline = self
            .matrix_timeline()
            .await
            .ok_or(TimelineError::NoTimeline)?;

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .send(AnyMessageLikeEventContent::Sticker(content))
                .await
        });

        handle
            .await
            .expect("task was not aborted")
            .map(|_send_handle| ())
            .map_err(|send_error| {
                error!("Could not send sticker: {send_error}");
                TimelineError::from(send_error)
            })
    }

    /// Send the given GIF as a sticker, then tell the service it was shared.
    ///
    /// The GIF is downloaded from the service and uploaded to the
    /// homeserver, rather than linked: a link would leak the IP address of
    /// everyone in the room to the service, would rot when the service
    /// drops the file, and could not be end-to-end encrypted. The service
    /// is told afterwards because that is how it counts, and it is why the
    /// API is free to use.
    pub async fn send_gif(&self, gif: SelectedGif, is_encrypted: bool) -> Result<(), SendGifError> {
        use ruma::events::{
            AnyMessageLikeEventContent, room::ImageInfo, sticker::StickerEventContent,
        };

        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(SendGifError::NoTimeline);
        };
        let client = self.inner.matrix_room.client();

        let SelectedGif {
            url,
            width,
            height,
            size,
            slug,
            title,
        } = gif;

        let source = spawn_tokio!(async move {
            let data = crate::http::fetch(&url, MAX_GIF_SIZE).await?;
            upload_gif(&client, is_encrypted, data).await
        })
        .await
        .expect("task was not aborted")
        .inspect_err(|send_error| error!("Could not send GIF: {send_error}"))?;

        let mut info = ImageInfo::new();
        info.width = Some(width.into());
        info.height = Some(height.into());
        info.size = size.try_into().ok();
        info.mimetype = Some(mime::IMAGE_GIF.to_string());
        // Without this the receiving client asks its homeserver for a
        // thumbnail, which is a still frame.
        info.is_animated = Some(true);

        let content = StickerEventContent::with_source(title, info, source);

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .send(AnyMessageLikeEventContent::Sticker(content))
                .await
        });

        if let Err(send_error) = handle.await.expect("task was not aborted") {
            error!("Could not send GIF: {send_error}");
            return Err(SendGifError::Send(Box::new(send_error)));
        }

        spawn_tokio!(async move { crate::klipy::report_share(&slug).await });

        Ok(())
    }

    /// Send the user's location: the application's exact `m.location`
    /// content, the geo URI with the given body naming it and the
    /// always-present mentions.
    ///
    /// The body is the embedder's: the application's is a translated
    /// sentence naming the URI and a local timestamp.
    pub async fn send_location(&self, geo_uri: String, body: String) -> Result<(), TimelineError> {
        let matrix_timeline = self
            .matrix_timeline()
            .await
            .ok_or(TimelineError::NoTimeline)?;

        let content = RoomMessageEventContent::new(MessageType::Location(
            LocationMessageEventContent::new(body, geo_uri),
        ))
        // To avoid triggering legacy pushrules, we must always include the
        // mentions, even if they are empty.
        .add_mentions(Mentions::default());

        let handle = spawn_tokio!(async move { matrix_timeline.send(content.into()).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_send_handle| ())
            .map_err(|send_error| {
                error!("Could not send location: {send_error}");
                TimelineError::from(send_error)
            })
    }

    /// Discard the local echo with the given unique ID: redact it through
    /// the timeline, which for an unsent message aborts the send, as the
    /// application's cancel-send action does.
    pub async fn discard_local_echo(&self, unique_id: &str) -> Result<(), TimelineError> {
        let matrix_timeline = self
            .matrix_timeline()
            .await
            .ok_or(TimelineError::NoTimeline)?;

        let unique_id = unique_id.to_owned();
        let handle = spawn_tokio!(async move {
            let identifier = matrix_timeline
                .items()
                .await
                .iter()
                .find(|item| item.unique_id().0 == unique_id)
                .and_then(|item| item.as_event())
                .map(EventTimelineItem::identifier)
                .ok_or(TimelineError::UnknownEvent)?;

            matrix_timeline
                .redact(&identifier, None)
                .await
                .map_err(TimelineError::from)
        });

        handle
            .await
            .expect("task was not aborted")
            .inspect_err(|discard_error| error!("Could not discard local event: {discard_error}"))
    }

    /// The pretty-printed JSON source of the given event, if it is in
    /// this timeline and the server echoed it back.
    ///
    /// The application's `Event::source()`: what the properties dialog
    /// shows, read from the loaded item rather than fetched.
    pub async fn event_source(&self, event_id: &EventId) -> Option<String> {
        let matrix_timeline = self.matrix_timeline().await?;

        let items = spawn_tokio!(async move { matrix_timeline.items().await })
            .await
            .expect("task was not aborted");

        let raw = items
            .iter()
            .filter_map(|item| item.as_event())
            .find(|event| event.event_id() == Some(event_id))
            .and_then(|event| event.original_json().cloned())?;

        // The raw value has to become a `Value`, because a `RawValue`
        // cannot be pretty-printed.
        let json = serde_json::to_value(&raw).ok()?;
        serde_json::to_string_pretty(&json).ok().into_clean_string()
    }
}

/// Refuse a file of the given size that the homeserver would not take.
///
/// The message toolbar asks the homeserver's upload limit before sending,
/// rather than uploading the whole file to be told no at the end. The SDK
/// caches the answer after the first ask; when it cannot be had, the
/// upload proceeds and the server stays the judge. A size that is not
/// known is not checked.
pub async fn check_upload_size(
    client: &matrix_sdk::Client,
    size: Option<u64>,
) -> Result<(), TimelineError> {
    let Some(size) = size else {
        return Ok(());
    };

    let client = client.clone();
    let handle = spawn_tokio!(async move { client.load_or_fetch_max_upload_size().await });
    if let Ok(max_upload_size) = handle.await.expect("task was not aborted")
        && size > u64::from(max_upload_size)
    {
        return Err(TimelineError::UploadTooLarge {
            max_bytes: u64::from(max_upload_size),
        });
    }

    Ok(())
}

/// Build the SDK timeline for the given room, with the application's
/// event filter and the given focus.
async fn build_sdk_timeline(
    matrix_room: matrix_sdk::room::Room,
    focus: TimelineFocusKind,
    is_server_notice_room: Arc<AtomicBool>,
) -> Result<SdkTimeline, matrix_sdk_ui::timeline::Error> {
    let own_user_id = matrix_room.own_user_id().to_owned();

    // The category of the room might not have been loaded yet, and the
    // filter cannot wait for it, so ask the store directly. The
    // `m.server_notice` tag is what identifies the room.
    {
        let matrix_room = matrix_room.clone();
        let is_server_notice_room = is_server_notice_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.tags().await });

        if let Ok(Some(tags)) = handle.await.expect("task was not aborted") {
            is_server_notice_room
                .store(tags.contains_key(&TagName::ServerNotice), Ordering::Relaxed);
        }
    }

    let filter = move |any: &AnySyncTimelineEvent, rules: &RoomVersionRules| -> bool {
        show_in_timeline(
            any,
            rules,
            &own_user_id,
            is_server_notice_room.load(Ordering::Relaxed),
        )
    };

    let handle = spawn_tokio!(async move {
        // Unparsable events are requested from the SDK because one of them
        // means something: an invalid or empty `m.room.policy` content
        // unsets the room's policy server, per the spec, and deserves its
        // sentence. The UI-side filter hides the rest.
        let sdk_focus = match focus {
            // Threaded events are hidden from the live timeline: since a
            // thread can be opened from its root, they have somewhere
            // better to be read.
            TimelineFocusKind::Live => TimelineFocus::Live {
                hide_threaded_events: true,
            },
            TimelineFocusKind::Pinned => TimelineFocus::PinnedEvents,
            TimelineFocusKind::Thread { root } => TimelineFocus::Thread {
                root_event_id: root,
            },
            TimelineFocusKind::Event { target } => TimelineFocus::Event {
                target,
                num_context_events: MAX_BATCH_SIZE,
                thread_mode: TimelineEventFocusThreadMode::Automatic {
                    hide_threaded_events: true,
                },
            },
        };

        matrix_room
            .timeline_builder()
            .event_filter(filter)
            .add_failed_to_parse(true)
            .with_focus(sdk_focus)
            .build()
            .await
    });

    handle.await.expect("task was not aborted")
}

/// Whether the given event should be shown in the timeline.
///
/// The application's `show_in_timeline`, verbatim.
fn show_in_timeline(
    any: &AnySyncTimelineEvent,
    rules: &RoomVersionRules,
    own_user_id: &UserId,
    is_server_notice_room: bool,
) -> bool {
    // Make sure we do not show events that cannot be shown.
    if !default_event_filter(any, rules) {
        return false;
    }

    // Only show events we want.
    match any {
        AnySyncTimelineEvent::MessageLike(msg) => match msg {
            AnySyncMessageLikeEvent::RoomMessage(SyncMessageLikeEvent::Original(ev)) => {
                match ev.content.msgtype {
                    // "Events with a `m.server_notice` `msgtype` outside of the
                    // server notice room must be ignored by clients." Anybody
                    // can send one, and we present them as coming from the
                    // homeserver, so this is the whole of the protection.
                    MessageType::ServerNotice(_) => is_server_notice_room,
                    MessageType::Audio(_)
                    | MessageType::Emote(_)
                    | MessageType::File(_)
                    | MessageType::Image(_)
                    | MessageType::Location(_)
                    | MessageType::Notice(_)
                    | MessageType::Text(_)
                    | MessageType::Video(_) => true,
                    _ => false,
                }
            }
            AnySyncMessageLikeEvent::Sticker(SyncMessageLikeEvent::Original(_))
            | AnySyncMessageLikeEvent::RoomEncrypted(SyncMessageLikeEvent::Original(_))
            // A call leaves a row where it happened. The rest of the module's
            // events are signalling and would be a dozen rows for one call;
            // the invite is the one that says a call took place, and the row
            // it draws says what became of it.
            //
            // Shown whether or not it rang: "when clients suppress ringing for
            // an incoming call invite, they SHOULD still display the call
            // invite in the room and annotate that it was ignored".
            | AnySyncMessageLikeEvent::CallInvite(SyncMessageLikeEvent::Original(_)) => true,
            AnySyncMessageLikeEvent::RtcNotification(SyncMessageLikeEvent::Original(ev)) => {
                ev.sender == own_user_id
                    || ev.content.mentions.as_ref().is_some_and(|mentions| {
                        mentions.room || mentions.user_ids.contains(own_user_id)
                    })
            }
            _ => false,
        },
        AnySyncTimelineEvent::State(AnySyncStateEvent::RoomMember(SyncStateEvent::Original(
            member_event,
        ))) => {
            // Do not show member events if the content that we support has not
            // changed. This avoids duplicate "user has joined" events in the
            // timeline which are confusing and wrong.
            !member_event
                .unsigned
                .prev_content
                .as_ref()
                .is_some_and(|prev_content| {
                    prev_content.membership == member_event.content.membership
                        && prev_content.displayname == member_event.content.displayname
                        && prev_content.avatar_url == member_event.content.avatar_url
                })
        }
        AnySyncTimelineEvent::State(state) => matches!(
            state,
            AnySyncStateEvent::RoomMember(_)
                | AnySyncStateEvent::RoomCreate(_)
                | AnySyncStateEvent::RoomEncryption(_)
                | AnySyncStateEvent::RoomThirdPartyInvite(_)
                // Pinning is an act of moderation and the pinned messages view
                // does not say who did it, so the room says so instead.
                | AnySyncStateEvent::RoomPinnedEvents(_)
                // `update_with_other_state` has written the sentence for this
                // one since the ACL editor landed, and this list is what kept
                // it from ever being drawn.
                | AnySyncStateEvent::RoomServerAcl(_)
                // Which server checks this room's messages is an act of
                // moderation too, and one worth a sentence.
                | AnySyncStateEvent::RoomPolicy(_)
                // The moderation policy rules: who wrote which rule, about
                // whom, and why, is the whole history of a policy room.
                | AnySyncStateEvent::PolicyRuleUser(_)
                | AnySyncStateEvent::PolicyRuleRoom(_)
                | AnySyncStateEvent::PolicyRuleServer(_)
        ),
    }
}

/// The position of the receipt to send.
#[derive(Debug, Clone)]
pub enum ReceiptPosition {
    /// We are at the end of the timeline (bottom of the view).
    End,
    /// We are at the event with the given ID.
    Event(ruma::OwnedEventId),
}

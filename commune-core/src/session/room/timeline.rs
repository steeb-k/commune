//! The timeline of a room, headless.
//!
//! Version 1 of the chunk-5 extraction: the live timeline only, as a thin
//! orchestration of [`matrix_sdk_ui::timeline::Timeline`] whose items and
//! diffs pass straight through to the subscriber — the application's
//! `GListModel` splicing, and therefore the diff minimizer, have nothing to
//! translate for and are not needed here. The event filter is the
//! application's `show_in_timeline`, verbatim. Focused, pinned and thread
//! timelines, the read-change trigger and receipts follow with the rest of
//! the chunk.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use eyeball::{SharedObservable, Subscriber};
use eyeball_im::{Vector, VectorDiff};
use futures_util::Stream;
use matrix_sdk_ui::timeline::{
    RoomExt, Timeline as SdkTimeline, TimelineFocus, TimelineItem as SdkTimelineItem,
    default_event_filter,
};
use ruma::{
    UserId,
    api::client::receipt::create_receipt::v3::ReceiptType as ApiReceiptType,
    events::{
        AnySyncMessageLikeEvent, AnySyncStateEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
        SyncStateEvent,
        room::message::{MessageType, RoomMessageEventContent},
        tag::TagName,
    },
    room_version_rules::RoomVersionRules,
};
use tracing::error;

use crate::{matrix::ext_traits::TimelineItemContentExt, spawn_tokio, utils::LoadingState};

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
}

#[derive(Debug)]
struct TimelineInner {
    /// The room API of the SDK.
    matrix_room: matrix_sdk::room::Room,
    /// What this timeline shows.
    focus: TimelineFocusKind,
    /// The underlying SDK timeline.
    matrix_timeline: tokio::sync::OnceCell<Arc<SdkTimeline>>,
    /// The loading state of the timeline.
    state: SharedObservable<LoadingState>,
    /// Whether the start of the room's history has been reached.
    has_reached_start: SharedObservable<bool>,
}

impl Timeline {
    /// Create the live timeline of the given room.
    pub(crate) fn new(matrix_room: matrix_sdk::room::Room) -> Self {
        Self::with_focus(matrix_room, TimelineFocusKind::Live)
    }

    /// Create a timeline of the given room with the given focus.
    pub(crate) fn with_focus(
        matrix_room: matrix_sdk::room::Room,
        focus: TimelineFocusKind,
    ) -> Self {
        Self {
            inner: Arc::new(TimelineInner {
                matrix_room,
                focus,
                matrix_timeline: tokio::sync::OnceCell::new(),
                state: SharedObservable::new(LoadingState::Initial),
                has_reached_start: SharedObservable::new(false),
            }),
        }
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

                match build_sdk_timeline(inner.matrix_room.clone(), inner.focus.clone()).await {
                    Ok(timeline) => {
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

        Some(handle.await.expect("task was not aborted"))
    }

    /// Paginate backwards by the given number of events.
    ///
    /// Returns whether the start of the timeline was reached, or `None` if
    /// the pagination failed.
    pub async fn paginate_backwards(&self, count: u16) -> Option<bool> {
        let matrix_timeline = self.matrix_timeline().await?;

        let handle = spawn_tokio!(async move { matrix_timeline.paginate_backwards(count).await });

        match handle.await.expect("task was not aborted") {
            Ok(reached_start) => {
                if reached_start {
                    self.inner.has_reached_start.set_if_not_eq(true);
                }
                Some(reached_start)
            }
            Err(paginate_error) => {
                error!("Could not paginate timeline: {paginate_error}");
                None
            }
        }
    }

    /// Send the given receipt through this timeline, its type already
    /// resolved against the public-read-receipts setting (the room does
    /// that resolution).
    ///
    /// The SDK scopes the receipt to what the timeline shows: sent through
    /// a thread timeline, it is a receipt for that thread, not for the
    /// room.
    pub(crate) async fn send_receipt_resolved(
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
    pub(crate) async fn subscribe_own_read_receipts(
        &self,
    ) -> Option<impl Stream<Item = ()> + use<>> {
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

    /// Send the given message to the room, rendered from Markdown the
    /// way the application's composer sends by default, mentioning the
    /// given users.
    pub async fn send_text(
        &self,
        body: String,
        plain_body: Option<String>,
        mentions: Vec<ruma::OwnedUserId>,
        room_mention: bool,
        emoticons: Vec<(String, String, String)>,
    ) -> Result<(), ()> {
        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let handle = spawn_tokio!(async move {
            let mut content = RoomMessageEventContent::text_markdown(body);
            // Mention anchors ride in as markdown; the plain body carries
            // the bare names instead of the link syntax.
            if let Some(plain) = plain_body
                && let MessageType::Text(text) = &mut content.msgtype
            {
                text.body = plain;
            }
            // A completed emoticon stays `:shortcode:` in the plain body
            // and becomes the application's exact image tag in the HTML.
            if !emoticons.is_empty()
                && let MessageType::Text(text) = &mut content.msgtype
            {
                use ruma::events::room::message::FormattedBody;

                let mut html = text.formatted.as_ref().map_or_else(
                    || escape_html(&text.body),
                    |formatted| formatted.body.clone(),
                );
                for (shortcode, url, alt) in &emoticons {
                    let tag = format!(
                        r#"<img data-mx-emoticon src="{}" alt="{}" title="{}" height="32">"#,
                        escape_html(url),
                        escape_html(alt),
                        escape_html(shortcode),
                    );
                    html = html.replace(&format!(":{shortcode}:"), &tag);
                }
                text.formatted = Some(FormattedBody::html(html));
            }
            if !mentions.is_empty() || room_mention {
                let mut all = ruma::events::Mentions::with_user_ids(mentions);
                all.room = room_mention;
                content.mentions = Some(all);
            }
            matrix_timeline.send(content.into()).await
        });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(send_error) => {
                error!("Could not send message: {send_error}");
                Err(())
            }
        }
    }

    /// Send the user's location to the room: the application's exact
    /// `m.location` content, a geo URI with a spoken body naming it and
    /// the timestamp, plus the always-present mentions.
    pub async fn send_location(&self, geo_uri: String) -> Result<(), ()> {
        use ruma::events::room::message::LocationMessageEventContent;

        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let handle = spawn_tokio!(async move {
            // The application stamps local time; UTC keeps the core off
            // the platform's timezone database.
            let timestamp = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Iso8601::DEFAULT)
                .unwrap_or_default();
            let body = format!("User Location {geo_uri} at {timestamp}");
            let content = RoomMessageEventContent::new(MessageType::Location(
                LocationMessageEventContent::new(body, geo_uri),
            ))
            // To avoid triggering legacy pushrules, we must always
            // include the mentions, even if they are empty.
            .add_mentions(ruma::events::Mentions::default());

            matrix_timeline.send(content.into()).await
        });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(send_error) => {
                error!("Could not send location: {send_error}");
                Err(())
            }
        }
    }

    /// Send the given plain-text message as a reply to the given event.
    pub async fn send_reply(
        &self,
        in_reply_to: ruma::OwnedEventId,
        body: String,
    ) -> Result<(), ()> {
        use ruma::events::room::message::RoomMessageEventContentWithoutRelation;

        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .send_reply(
                    RoomMessageEventContentWithoutRelation::text_plain(body),
                    in_reply_to,
                )
                .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(()) => Ok(()),
            Err(send_error) => {
                error!("Could not send reply: {send_error}");
                Err(())
            }
        }
    }

    /// Replace the given event's content with the given plain text.
    pub async fn edit(&self, event_id: ruma::OwnedEventId, new_body: String) -> Result<(), ()> {
        use matrix_sdk::room::edit::EditedContent;
        use matrix_sdk_ui::timeline::TimelineEventItemId;
        use ruma::events::room::message::RoomMessageEventContentWithoutRelation;

        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .edit(
                    &TimelineEventItemId::EventId(event_id),
                    EditedContent::RoomMessage(RoomMessageEventContentWithoutRelation::text_plain(
                        new_body,
                    )),
                )
                .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(()) => Ok(()),
            Err(edit_error) => {
                error!("Could not edit message: {edit_error}");
                Err(())
            }
        }
    }

    /// Redact the given event, without a reason.
    pub async fn redact(&self, event_id: ruma::OwnedEventId) -> Result<(), ()> {
        use matrix_sdk_ui::timeline::TimelineEventItemId;

        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .redact(&TimelineEventItemId::EventId(event_id), None)
                .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(()) => Ok(()),
            Err(redact_error) => {
                error!("Could not redact event: {redact_error}");
                Err(())
            }
        }
    }

    /// Toggle the given reaction key on the given event.
    pub async fn toggle_reaction(&self, event_id: ruma::OwnedEventId, key: &str) -> Result<(), ()> {
        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let key = key.to_owned();
        let handle = spawn_tokio!(async move {
            matrix_timeline
                .toggle_reaction(
                    &matrix_sdk_ui::timeline::TimelineEventItemId::EventId(event_id),
                    &key,
                )
                .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(toggle_error) => {
                error!("Could not toggle reaction: {toggle_error}");
                Err(())
            }
        }
    }

    /// Send the file at the given path as an attachment to the room.
    ///
    /// The upload-size preflight lives in the facade, ahead of this;
    /// thumbnails arrive with the composer chunk.
    pub async fn send_attachment(
        &self,
        path: std::path::PathBuf,
        mime: mime::Mime,
    ) -> Result<(), ()> {
        use matrix_sdk::attachment::{AttachmentInfo, BaseFileInfo, BaseImageInfo};
        use matrix_sdk_ui::timeline::{AttachmentConfig, AttachmentSource};

        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let size = std::fs::metadata(&path)
            .ok()
            .and_then(|metadata| metadata.len().try_into().ok());
        let info = if mime.type_() == mime::IMAGE {
            AttachmentInfo::Image(BaseImageInfo {
                size,
                ..Default::default()
            })
        } else {
            AttachmentInfo::File(BaseFileInfo { size })
        };
        let config = AttachmentConfig {
            info: Some(info),
            ..Default::default()
        };

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .send_attachment(AttachmentSource::File(path), mime, config)
                .use_send_queue()
                .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(()) => Ok(()),
            Err(send_error) => {
                error!("Could not send attachment: {send_error}");
                Err(())
            }
        }
    }

    /// Discard the local echo with the given unique ID: redact it
    /// through the timeline, which for an unsent message aborts the
    /// send, as the application's cancel-send action does.
    pub async fn discard_local_echo(&self, unique_id: &str) -> Result<(), ()> {
        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let unique_id = unique_id.to_owned();
        let handle = spawn_tokio!(async move {
            let identifier = matrix_timeline
                .items()
                .await
                .iter()
                .find(|item| item.unique_id().0 == unique_id)
                .and_then(|item| item.as_event())
                .map(matrix_sdk_ui::timeline::EventTimelineItem::identifier);

            match identifier {
                Some(identifier) => matrix_timeline
                    .redact(&identifier, None)
                    .await
                    .map_err(|_| ()),
                None => Err(()),
            }
        });

        handle.await.expect("task was not aborted")
    }

    /// Send a recorded voice message: an audio attachment with its
    /// duration and the voice-message marker.
    pub async fn send_voice(
        &self,
        path: std::path::PathBuf,
        mime: mime::Mime,
        duration_ms: u64,
    ) -> Result<(), ()> {
        use matrix_sdk::attachment::{AttachmentInfo, BaseAudioInfo};
        use matrix_sdk_ui::timeline::{AttachmentConfig, AttachmentSource};

        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let size = std::fs::metadata(&path)
            .ok()
            .and_then(|metadata| metadata.len().try_into().ok());
        let config = AttachmentConfig {
            info: Some(AttachmentInfo::Voice(BaseAudioInfo {
                duration: Some(std::time::Duration::from_millis(duration_ms)),
                size,
                waveform: None,
            })),
            ..Default::default()
        };

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .send_attachment(AttachmentSource::File(path), mime, config)
                .use_send_queue()
                .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(()) => Ok(()),
            Err(send_error) => {
                error!("Could not send voice message: {send_error}");
                Err(())
            }
        }
    }
}

/// Build the SDK timeline for the given room, with the application's
/// event filter and the given focus.
async fn build_sdk_timeline(
    matrix_room: matrix_sdk::room::Room,
    focus: TimelineFocusKind,
) -> Result<SdkTimeline, matrix_sdk_ui::timeline::Error> {
    let own_user_id = matrix_room.own_user_id().to_owned();

    // The category of the room might not have been loaded yet, and the
    // filter cannot wait for it, so ask the store directly. The
    // `m.server_notice` tag is what identifies the room.
    let is_server_notice_room = Arc::new(AtomicBool::new(false));
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

/// Escape a string for HTML attribute and text positions.
fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

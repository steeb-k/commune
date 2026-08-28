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
    events::{
        AnySyncMessageLikeEvent, AnySyncStateEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
        SyncStateEvent,
        room::message::{MessageType, RoomMessageEventContent},
        tag::TagName,
    },
    room_version_rules::RoomVersionRules,
};
use tracing::error;

use crate::{spawn_tokio, utils::LoadingState};

/// The timeline of a room.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct Timeline {
    inner: Arc<TimelineInner>,
}

#[derive(Debug)]
struct TimelineInner {
    /// The room API of the SDK.
    matrix_room: matrix_sdk::room::Room,
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
        Self {
            inner: Arc::new(TimelineInner {
                matrix_room,
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

                match build_sdk_timeline(inner.matrix_room.clone()).await {
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

    /// Send the given plain-text message to the room.
    ///
    /// The application's composer detects mentions and offers Markdown;
    /// that logic arrives with the composer chunk.
    pub async fn send_text(&self, body: String) -> Result<(), ()> {
        let Some(matrix_timeline) = self.matrix_timeline().await else {
            return Err(());
        };

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .send(RoomMessageEventContent::text_plain(body).into())
                .await
        });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(send_error) => {
                error!("Could not send message: {send_error}");
                Err(())
            }
        }
    }
}

/// Build the SDK timeline for the given room, live-focused, with the
/// application's event filter.
async fn build_sdk_timeline(
    matrix_room: matrix_sdk::room::Room,
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
        matrix_room
            .timeline_builder()
            .event_filter(filter)
            .add_failed_to_parse(true)
            // Threaded events are hidden from the live timeline: since a
            // thread can be opened from its root, they have somewhere
            // better to be read.
            .with_focus(TimelineFocus::Live {
                hide_threaded_events: true,
            })
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

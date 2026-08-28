//! A Matrix room, headless.
//!
//! This is the sidebar-relevant slice of the application's `room/mod.rs`:
//! identity, names, category, direct-chat detection, tombstones,
//! read/notification state and the category-changing actions, all as
//! `eyeball` observables over the same update logic in the same order.
//!
//! What is deliberately not here yet, and which chunk brings it:
//! aliases beyond the SDK's own, members and the own-member (so `is_joined`
//! reads the SDK state directly), the live timeline (so `latest_activity`
//! is approximated from sync events and `is_read` moves only with the
//! unread flag until then), typing, permissions, verification, the
//! send-queue watcher, server notices, pinned events, join rule details and
//! the avatar-as-image (the core hands out MXC URIs; the direct member's
//! avatar fallback follows with members).
//!
//! Strings policy: the display name is handed out as the semantic
//! [`RoomDisplayName`] — "Empty Room (was X)" is the UI's sentence to make.

mod category;
mod timeline;

use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

use eyeball::{SharedObservable, Subscriber};
use futures_util::StreamExt;
use matrix_sdk::{
    Result as MatrixResult, RoomDisplayName as SdkRoomDisplayName, RoomInfo, RoomState,
    deserialized_responses::RawSyncOrStrippedState, room::Room as MatrixRoom,
};
use ruma::{
    MilliSecondsSinceUnixEpoch, OwnedMxcUri, OwnedRoomId, OwnedUserId, RoomId,
    events::{
        AnySyncTimelineEvent,
        room::member::{MembershipState, RoomMemberEventContent},
        tag::TagName,
    },
    serde::Raw,
};
use serde::Deserialize;
use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

pub use self::{
    category::{RoomCategory, RoomHighlight, TargetRoomCategory},
    timeline::Timeline,
};
use crate::{
    RUNTIME,
    session::{Session, WeakSession, room_list::RoomMetainfo},
    spawn_tokio,
    utils::{OptionStringExt, StrMutExt},
};

/// The tag order of a room that has none.
///
/// The specification keeps real orders in `[0, 1]` and asks that ordered
/// rooms come first, so anything past 1 sorts a room after all of them.
const NO_TAG_ORDER: f64 = 2.0;

/// The display name of a room, as something the UI still has words to add
/// to.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum RoomDisplayName {
    /// The room has a computed name.
    Named(String),
    /// The room is empty but had another user before.
    EmptyWas(String),
    /// The room is empty and never had another user.
    Empty,
    /// The name is not known yet.
    #[default]
    Unknown,
}

impl RoomDisplayName {
    /// Convert the SDK's display name, cleaning the strings it carries.
    fn from_sdk(name: SdkRoomDisplayName) -> Self {
        let cleaned = |s: String| Some(s).into_clean_string();

        match name {
            SdkRoomDisplayName::Named(s)
            | SdkRoomDisplayName::Calculated(s)
            | SdkRoomDisplayName::Aliased(s) => cleaned(s).map_or(Self::Unknown, Self::Named),
            SdkRoomDisplayName::EmptyWas(s) => cleaned(s).map_or(Self::Empty, Self::EmptyWas),
            SdkRoomDisplayName::Empty => Self::Empty,
        }
    }
}

/// A Matrix room.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct Room {
    inner: Arc<RoomInner>,
}

#[derive(Debug)]
struct RoomInner {
    /// The room API of the SDK.
    matrix_room: MatrixRoom,
    /// The current session.
    session: WeakSession,
    /// The name that is set for this room.
    ///
    /// This can be empty, the display name should be used instead in the
    /// interface.
    name: SharedObservable<Option<String>>,
    /// The display name of this room.
    display_name: SharedObservable<RoomDisplayName>,
    /// Whether this room has an avatar explicitly set.
    has_avatar: SharedObservable<bool>,
    /// The avatar of this room, if it has one explicitly set.
    avatar_url: SharedObservable<Option<OwnedMxcUri>>,
    /// The topic of this room.
    topic: SharedObservable<Option<String>>,
    /// The category of this room.
    category: SharedObservable<RoomCategory>,
    /// The order of this room inside its tag, from the `m.tag` account data.
    ///
    /// The specification orders it in `[0, 1]`, smaller first, and asks that
    /// rooms carrying an order come before rooms without one — so a room
    /// without one reports [`NO_TAG_ORDER`], which sorts after every real
    /// value.
    tag_order: SharedObservable<f64>,
    /// Whether this room is a direct chat.
    is_direct: SharedObservable<bool>,
    /// The other user, if this room is a direct chat with one other user.
    direct_member_user_id: SharedObservable<Option<OwnedUserId>>,
    /// Whether this room has been upgraded.
    is_tombstoned: SharedObservable<bool>,
    /// The ID of the room that was upgraded and that this one replaces.
    predecessor_id: std::sync::OnceLock<OwnedRoomId>,
    /// The ID of the successor of this Room, if this room was upgraded.
    successor_id: std::sync::OnceLock<OwnedRoomId>,
    /// The successor of this Room, if this room was upgraded and the
    /// successor was joined.
    successor: Mutex<Option<Weak<RoomInner>>>,
    /// The number of joined members in the room, according to the
    /// homeserver.
    joined_members_count: SharedObservable<u64>,
    /// Whether this room is a current invite or an invite that was declined
    /// or retracted.
    is_invite: SharedObservable<bool>,
    /// The timestamp of the room's latest activity.
    ///
    /// This is the timestamp of the latest event that counts as possibly
    /// unread. If it is not known, it will return `0`.
    latest_activity: SharedObservable<u64>,
    /// Whether this room is marked as unread.
    is_marked_unread: SharedObservable<bool>,
    /// Whether all messages of this room are read.
    is_read: SharedObservable<bool>,
    /// The number of unread notifications of this room.
    notification_count: SharedObservable<u64>,
    /// Whether this room has unread notifications.
    has_notifications: SharedObservable<bool>,
    /// The highlight state of the room.
    highlight: SharedObservable<RoomHighlight>,
    /// Whether this room is encrypted.
    is_encrypted: SharedObservable<bool>,
    /// Whether this room was forgotten.
    forgotten: SharedObservable<bool>,
    /// Whether the room info is initialized.
    ///
    /// Used to silence logs during initialization.
    is_room_info_initialized: SharedObservable<bool>,
    /// Whether we already attempted an auto-join.
    attempted_auto_join: AtomicBool,
    /// The task watching the SDK's room info.
    room_info_handle: Mutex<Option<AbortHandle>>,
    /// The live timeline of this room.
    live_timeline: std::sync::OnceLock<Timeline>,
}

impl Drop for RoomInner {
    fn drop(&mut self) {
        if let Ok(Some(handle)) = self.room_info_handle.get_mut().map(Option::take) {
            handle.abort();
        }
    }
}

impl Room {
    /// Create a new `Room` for the given session, with the given room API.
    pub(crate) fn new(
        session: &Session,
        matrix_room: MatrixRoom,
        metainfo: Option<RoomMetainfo>,
    ) -> Self {
        let inner = Arc::new(RoomInner {
            matrix_room,
            session: session.downgrade(),
            name: SharedObservable::new(None),
            display_name: SharedObservable::new(RoomDisplayName::default()),
            has_avatar: SharedObservable::new(false),
            avatar_url: SharedObservable::new(None),
            topic: SharedObservable::new(None),
            category: SharedObservable::new(RoomCategory::default()),
            tag_order: SharedObservable::new(NO_TAG_ORDER),
            is_direct: SharedObservable::new(false),
            direct_member_user_id: SharedObservable::new(None),
            is_tombstoned: SharedObservable::new(false),
            predecessor_id: std::sync::OnceLock::new(),
            successor_id: std::sync::OnceLock::new(),
            successor: Mutex::new(None),
            joined_members_count: SharedObservable::new(0),
            is_invite: SharedObservable::new(false),
            latest_activity: SharedObservable::new(0),
            is_marked_unread: SharedObservable::new(false),
            is_read: SharedObservable::new(true),
            notification_count: SharedObservable::new(0),
            has_notifications: SharedObservable::new(false),
            highlight: SharedObservable::new(RoomHighlight::default()),
            is_encrypted: SharedObservable::new(false),
            forgotten: SharedObservable::new(false),
            is_room_info_initialized: SharedObservable::new(false),
            attempted_auto_join: AtomicBool::new(false),
            room_info_handle: Mutex::new(None),
            live_timeline: std::sync::OnceLock::new(),
        });

        let this = Self { inner };
        this.init(metainfo);
        this
    }

    /// Initialize this room.
    fn init(&self, metainfo: Option<RoomMetainfo>) {
        let inner = &self.inner;

        inner.load_predecessor();

        if let Some(RoomMetainfo {
            latest_activity,
            is_read,
        }) = metainfo
        {
            inner.latest_activity.set_if_not_eq(latest_activity);
            inner.is_read.set_if_not_eq(is_read);
            inner.update_highlight();
        }

        // The timeline preload, member watch, typing, join rule, permissions
        // and send-queue watch attach here with their chunks.

        {
            let weak = Arc::downgrade(inner);
            RUNTIME.spawn(async move {
                let Some(inner) = weak.upgrade() else { return };
                inner
                    .update_with_room_info(inner.matrix_room.clone_info())
                    .await;
                RoomInner::watch_room_info(&inner);
                inner.is_room_info_initialized.set_if_not_eq(true);
            });
        }
    }

    /// The room API of the SDK.
    #[must_use]
    pub fn matrix_room(&self) -> &MatrixRoom {
        &self.inner.matrix_room
    }

    /// The current session.
    ///
    /// Unused until the subsystems that reach back into the session
    /// (notifications, verification) are extracted.
    #[allow(dead_code)]
    pub(crate) fn session(&self) -> Option<Session> {
        self.inner.session.upgrade()
    }

    /// The ID of this room.
    #[must_use]
    pub fn room_id(&self) -> &RoomId {
        self.inner.matrix_room.room_id()
    }

    /// Get a human-readable ID for this `Room`.
    ///
    /// This shows the display name and room ID to identify the room easily
    /// in logs.
    #[must_use]
    pub fn human_readable_id(&self) -> String {
        format!("{:?} ({})", self.display_name(), self.room_id())
    }

    /// Whether this room is joined.
    #[must_use]
    pub fn is_joined(&self) -> bool {
        self.inner.matrix_room.state() == RoomState::Joined
    }

    /// The name that is set for this room.
    ///
    /// This can be empty, the display name should be used instead in the
    /// interface.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        self.inner.name.get()
    }

    /// The display name of this room.
    #[must_use]
    pub fn display_name(&self) -> RoomDisplayName {
        self.inner.display_name.get()
    }

    /// Subscribe to the display name of this room.
    pub fn subscribe_display_name(&self) -> Subscriber<RoomDisplayName> {
        self.inner.display_name.subscribe()
    }

    /// Whether this room has an avatar explicitly set.
    #[must_use]
    pub fn has_avatar(&self) -> bool {
        self.inner.has_avatar.get()
    }

    /// The avatar of this room, if it has one explicitly set.
    #[must_use]
    pub fn avatar_url(&self) -> Option<OwnedMxcUri> {
        self.inner.avatar_url.get()
    }

    /// Subscribe to the avatar of this room.
    pub fn subscribe_avatar_url(&self) -> Subscriber<Option<OwnedMxcUri>> {
        self.inner.avatar_url.subscribe()
    }

    /// The topic of this room.
    #[must_use]
    pub fn topic(&self) -> Option<String> {
        self.inner.topic.get()
    }

    /// The category of this room.
    #[must_use]
    pub fn category(&self) -> RoomCategory {
        self.inner.category.get()
    }

    /// Subscribe to the category of this room.
    pub fn subscribe_category(&self) -> Subscriber<RoomCategory> {
        self.inner.category.subscribe()
    }

    /// The order of this room inside its tag.
    #[must_use]
    pub fn tag_order(&self) -> f64 {
        self.inner.tag_order.get()
    }

    /// Whether this room is a direct chat.
    #[must_use]
    pub fn is_direct(&self) -> bool {
        self.inner.is_direct.get()
    }

    /// The other user, if this room is a direct chat with one other user.
    #[must_use]
    pub fn direct_member_user_id(&self) -> Option<OwnedUserId> {
        self.inner.direct_member_user_id.get()
    }

    /// Whether this room has been upgraded.
    #[must_use]
    pub fn is_tombstoned(&self) -> bool {
        self.inner.is_tombstoned.get()
    }

    /// The ID of the room that was upgraded and that this one replaces.
    #[must_use]
    pub fn predecessor_id(&self) -> Option<&OwnedRoomId> {
        self.inner.predecessor_id.get()
    }

    /// The ID of the successor of this Room, if this room was upgraded.
    #[must_use]
    pub fn successor_id(&self) -> Option<&OwnedRoomId> {
        self.inner.successor_id.get()
    }

    /// The number of joined members in the room, according to the
    /// homeserver.
    #[must_use]
    pub fn joined_members_count(&self) -> u64 {
        self.inner.joined_members_count.get()
    }

    /// Whether this room is a current invite or an invite that was declined
    /// or retracted.
    #[must_use]
    pub fn is_invite(&self) -> bool {
        self.inner.is_invite.get()
    }

    /// The timestamp of the room's latest activity.
    #[must_use]
    pub fn latest_activity(&self) -> u64 {
        self.inner.latest_activity.get()
    }

    /// Subscribe to the timestamp of the room's latest activity.
    pub fn subscribe_latest_activity(&self) -> Subscriber<u64> {
        self.inner.latest_activity.subscribe()
    }

    /// Whether all messages of this room are read.
    #[must_use]
    pub fn is_read(&self) -> bool {
        self.inner.is_read.get()
    }

    /// Subscribe to whether all messages of this room are read.
    pub fn subscribe_is_read(&self) -> Subscriber<bool> {
        self.inner.is_read.subscribe()
    }

    /// The number of unread notifications of this room.
    #[must_use]
    pub fn notification_count(&self) -> u64 {
        self.inner.notification_count.get()
    }

    /// Subscribe to the number of unread notifications of this room.
    pub fn subscribe_notification_count(&self) -> Subscriber<u64> {
        self.inner.notification_count.subscribe()
    }

    /// Whether this room has unread notifications.
    #[must_use]
    pub fn has_notifications(&self) -> bool {
        self.inner.has_notifications.get()
    }

    /// The highlight state of the room.
    #[must_use]
    pub fn highlight(&self) -> RoomHighlight {
        self.inner.highlight.get()
    }

    /// Subscribe to the highlight state of the room.
    pub fn subscribe_highlight(&self) -> Subscriber<RoomHighlight> {
        self.inner.highlight.subscribe()
    }

    /// Whether this room is encrypted.
    #[must_use]
    pub fn is_encrypted(&self) -> bool {
        self.inner.is_encrypted.get()
    }

    /// Whether this room was forgotten.
    pub(crate) fn subscribe_forgotten(&self) -> Subscriber<bool> {
        self.inner.forgotten.subscribe()
    }

    /// The live timeline of this room, created on first use.
    #[must_use]
    pub fn live_timeline(&self) -> Timeline {
        self.inner
            .live_timeline
            .get_or_init(|| Timeline::new(self.inner.matrix_room.clone()))
            .clone()
    }

    /// Change the category of this room.
    ///
    /// This makes the necessary to propagate the category to the homeserver.
    ///
    /// This can be used to trigger actions like join or leave, as well as
    /// changing the category in the sidebar.
    ///
    /// Note that rooms cannot change category once they are upgraded.
    pub async fn change_category(&self, category: TargetRoomCategory) -> MatrixResult<()> {
        RoomInner::change_category(&self.inner, category).await
    }

    /// Mark the room as unread.
    pub async fn mark_as_unread(&self) {
        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.set_unread_flag(true).await });

        if let Err(unread_error) = handle.await.expect("task was not aborted") {
            error!("Could not mark room as unread: {unread_error}");
        }
    }

    /// Forget a room that is left.
    pub async fn forget(&self) -> MatrixResult<()> {
        if self.category() != RoomCategory::Left {
            warn!("Cannot forget a room that is not left");
            return Ok(());
        }

        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.forget().await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => {
                self.inner.forgotten.set_if_not_eq(true);
                Ok(())
            }
            Err(forget_error) => {
                error!("Could not forget the room: {forget_error}");
                Err(forget_error)
            }
        }
    }

    /// Update the successor of this room.
    pub(crate) fn update_successor(&self) {
        RoomInner::update_successor(&self.inner);
    }

    /// Take the latest origin server timestamp of the given raw sync events
    /// as this room's latest activity.
    ///
    /// This is an approximation until the timeline is extracted: the
    /// application filters events through `counts_as_activity`, which needs
    /// the event model.
    pub(crate) fn handle_sync_timeline_events<'a>(
        &self,
        events: impl DoubleEndedIterator<Item = &'a Raw<AnySyncTimelineEvent>>,
    ) {
        let mut latest_activity = self.inner.latest_activity.get();

        for event in events.rev() {
            if let Ok(Some(timestamp)) =
                event.get_field::<MilliSecondsSinceUnixEpoch>("origin_server_ts")
            {
                latest_activity = latest_activity.max(timestamp.get().into());
                break;
            }
        }

        self.inner.latest_activity.set_if_not_eq(latest_activity);
    }
}

impl RoomInner {
    /// Update the name of this room.
    fn update_name(&self) {
        let name = self.matrix_room.name().into_clean_string();
        self.name.set_if_not_eq(name);
    }

    /// Load the display name from the SDK.
    async fn update_display_name(&self) {
        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.display_name().await });

        let display_name = handle
            .await
            .expect("task was not aborted")
            .inspect_err(|display_name_error| {
                error!("Could not compute display name: {display_name_error}");
            })
            .ok()
            .map_or(RoomDisplayName::Unknown, RoomDisplayName::from_sdk);

        self.display_name.set_if_not_eq(display_name);
    }

    /// Update the avatar of the room.
    ///
    /// The core hands out the MXC URI; turning it into pixels is the UI's
    /// business, and the direct member's avatar fallback follows with the
    /// member model.
    fn update_avatar(&self) {
        let avatar_url = self.matrix_room.avatar_url();
        self.has_avatar.set_if_not_eq(avatar_url.is_some());
        self.avatar_url.set_if_not_eq(avatar_url);
    }

    /// Update the topic of this room.
    fn update_topic(&self) {
        let topic = self
            .matrix_room
            .topic()
            .map(|mut s| {
                s.strip_nul();
                s.truncate_end_whitespaces();
                s
            })
            .filter(|topic| !topic.is_empty());

        self.topic.set_if_not_eq(topic);
    }

    /// Set the category of this room.
    fn set_category(&self, category: RoomCategory) {
        let old_category = self.category.get();

        if old_category == RoomCategory::Outdated || old_category == category {
            return;
        }

        self.category.set(category);

        // Check if the previous state was different.
        let room_state = self.matrix_room.state();
        if !old_category.is_state(room_state) && self.is_room_info_initialized.get() {
            debug!(room_id = %self.matrix_room.room_id(), ?room_state, "The state of the room changed");
            // The member-list reload and typing setup attach here with
            // their chunks.
        }
    }

    /// Whether this room carries the `m.server_notice` tag.
    ///
    /// The tag is set by the homeserver, and it is the only thing that
    /// identifies the server notices room, per the Server Notices module.
    /// It is not one of the SDK's "notable tags", so it is not cached on
    /// the room info and has to be read from the store.
    async fn is_tagged_server_notice(&self) -> bool {
        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.tags().await });

        match handle.await.expect("task was not aborted") {
            Ok(tags) => tags
                .as_ref()
                .is_some_and(|tags| tags.contains_key(&TagName::ServerNotice)),
            Err(tags_error) => {
                error!("Could not read the tags of the room: {tags_error}");
                false
            }
        }
    }

    /// Update the category from the SDK.
    async fn update_category(self: &Arc<Self>) {
        // Do not load the category if this room was upgraded.
        if self.category.get() == RoomCategory::Outdated {
            return;
        }

        self.update_is_invite().await;

        let state = self.matrix_room.state();

        // The state changed, reset the attempted auto-join.
        if state != RoomState::Invited {
            self.attempted_auto_join.store(false, Ordering::Relaxed);
        }

        let category = match state {
            RoomState::Joined => {
                if self.matrix_room.is_space() {
                    RoomCategory::Space
                } else if self.is_tagged_server_notice().await {
                    RoomCategory::ServerNotice
                } else if self.matrix_room.is_favourite() {
                    RoomCategory::Favorite
                } else if self.matrix_room.is_low_priority() {
                    RoomCategory::LowPriority
                } else {
                    RoomCategory::Normal
                }
            }
            RoomState::Invited => {
                // Automatically accept invite that was after a knock.
                if !self.attempted_auto_join.load(Ordering::Relaxed)
                    && self.was_membership(&MembershipState::Knock).await
                {
                    self.attempted_auto_join.store(true, Ordering::Relaxed);

                    if Self::change_category(self, TargetRoomCategory::Normal)
                        .await
                        .is_ok()
                    {
                        // Wait for the next change to move automatically from knocked to
                        // joined.
                        return;
                    }
                }

                // The invites-from-ignored-users check attaches here with
                // the ignored-users chunk.
                RoomCategory::Invited
            }
            RoomState::Knocked => RoomCategory::Knocked,
            RoomState::Left | RoomState::Banned => RoomCategory::Left,
        };

        self.set_category(category);
        self.update_tag_order(category).await;
    }

    /// Update the order of this room inside its tag.
    ///
    /// Only the two tags that place a room in a sorted section matter;
    /// everything else reads as unordered.
    async fn update_tag_order(&self, category: RoomCategory) {
        let tag_name = match category {
            RoomCategory::Favorite => TagName::Favorite,
            RoomCategory::LowPriority => TagName::LowPriority,
            _ => {
                self.set_tag_order(NO_TAG_ORDER);
                return;
            }
        };

        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.tags().await });

        let tag_order = match handle.await.expect("task was not aborted") {
            Ok(tags) => tags
                .and_then(|tags| tags.get(&tag_name).and_then(|info| info.order))
                .unwrap_or(NO_TAG_ORDER),
            Err(tags_error) => {
                error!("Could not read the tags of the room: {tags_error}");
                NO_TAG_ORDER
            }
        };

        self.set_tag_order(tag_order);
    }

    /// Set the order of this room inside its tag.
    fn set_tag_order(&self, tag_order: f64) {
        if (self.tag_order.get() - tag_order).abs() < f64::EPSILON {
            return;
        }

        self.tag_order.set(tag_order);
    }

    /// Update whether the room is direct or not.
    async fn update_is_direct(&self) {
        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.is_direct().await });

        match handle.await.expect("task was not aborted") {
            Ok(is_direct) => {
                self.is_direct.set_if_not_eq(is_direct);
                self.update_direct_member().await;
            }
            Err(direct_error) => {
                error!(
                    room_id = %self.matrix_room.room_id(),
                    "Could not load whether room is direct: {direct_error}"
                );
            }
        }
    }

    /// The ID of the other user, if this is a direct chat and there is only
    /// one other user.
    async fn direct_user_id(&self) -> Option<OwnedUserId> {
        // Check if the room is direct and if there is only one target.
        let mut direct_targets = self
            .matrix_room
            .direct_targets()
            .into_iter()
            .filter_map(|id| OwnedUserId::try_from(id).ok());

        let direct_target_user_id = direct_targets.next()?;

        if direct_targets.next().is_some() {
            // It is a direct chat with several users.
            return None;
        }

        // Check that there are still at most 2 members.
        let members_count = self.matrix_room.active_members_count();

        if members_count > 2 {
            // We only want a 1-to-1 room. The count might be 1 if the other user left, but
            // we can reinvite them.
            return None;
        }

        // Check that the members count is correct. It might not be correct if the room
        // was just joined, or if it is in an invited state.
        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            matrix_room
                .members(matrix_sdk::RoomMemberships::ACTIVE)
                .await
        });

        let members = match handle.await.expect("task was not aborted") {
            Ok(m) => m,
            Err(members_error) => {
                error!("Could not load room members: {members_error}");
                vec![]
            }
        };

        let members_count = members_count.max(members.len() as u64);
        if members_count > 2 {
            // Same as before.
            return None;
        }

        let own_user_id = self.matrix_room.own_user_id();
        // Get the other member from the list.
        for member in members {
            let user_id = member.user_id();

            if user_id != direct_target_user_id && user_id != own_user_id {
                // There is a non-direct member.
                return None;
            }
        }

        Some(direct_target_user_id)
    }

    /// Update the other member of the room, if this room is a direct chat
    /// and there is only one other member.
    async fn update_direct_member(&self) {
        let direct_user_id = self.direct_user_id().await;
        self.direct_member_user_id.set_if_not_eq(direct_user_id);
    }

    /// Update the tombstone for this room.
    fn update_tombstone(self: &Arc<Self>) {
        if !self.matrix_room.is_tombstoned() || self.successor_id.get().is_some() {
            return;
        }

        if let Some(successor_id) = self
            .matrix_room
            .tombstone_content()
            .and_then(|room_tombstone| room_tombstone.replacement_room)
        {
            let _ = self.successor_id.set(successor_id);
        }

        // Try to get the successor.
        Self::update_successor(self);

        // If the successor was not found, watch for it in the room list.
        if self
            .successor
            .lock()
            .expect("mutex is not poisoned")
            .is_none()
            && let Some(session) = self.session.upgrade()
        {
            session
                .room_list()
                .add_tombstoned_room(self.matrix_room.room_id().to_owned());
        }

        self.is_tombstoned.set_if_not_eq(true);
    }

    /// Update the successor of this room.
    fn update_successor(self: &Arc<Self>) {
        if self.category.get() == RoomCategory::Outdated {
            return;
        }

        let Some(session) = self.session.upgrade() else {
            return;
        };
        let room_list = session.room_list();
        let room_id = self.matrix_room.room_id();

        if let Some(successor) = self
            .successor_id
            .get()
            .and_then(|successor_id| room_list.get(successor_id))
        {
            // The Matrix spec says that we should use the "predecessor" field of the
            // m.room.create event of the successor, not the "successor" field of the
            // m.room.tombstone event, so check it just to be sure.
            if successor
                .predecessor_id()
                .is_some_and(|predecessor_id| predecessor_id == room_id)
            {
                self.set_successor(&successor);
                return;
            }
        }

        // The tombstone event can be redacted and we lose the successor, so search in
        // the room predecessors of other rooms.
        for room in room_list.snapshot() {
            if room
                .predecessor_id()
                .is_some_and(|predecessor_id| predecessor_id == room_id)
            {
                self.set_successor(&room);
                return;
            }
        }
    }

    /// Load the predecessor of this room.
    fn load_predecessor(&self) {
        let Some(event) = self.matrix_room.create_content() else {
            return;
        };
        let Some(predecessor) = event.predecessor else {
            return;
        };

        let _ = self.predecessor_id.set(predecessor.room_id);
    }

    /// Set the successor of this room.
    fn set_successor(&self, successor: &Room) {
        *self.successor.lock().expect("mutex is not poisoned") =
            Some(Arc::downgrade(&successor.inner));

        self.set_category(RoomCategory::Outdated);
    }

    /// Update whether this room is a current invite or an invite that was
    /// declined or retracted.
    async fn update_is_invite(&self) {
        let is_invite = match self.matrix_room.state() {
            RoomState::Invited => true,
            RoomState::Left | RoomState::Banned => {
                self.was_membership(&MembershipState::Invite).await
            }
            _ => false,
        };

        self.is_invite.set_if_not_eq(is_invite);
    }

    /// Check whether the previous membership of our user in this room
    /// matches the one that is given.
    async fn was_membership(&self, membership: &MembershipState) -> bool {
        // To know if this was an invite we need to check in the member event of our own
        // user if the current membership is `invite`, or if the current membership is
        // `leave` or `ban`, and the previous membership was `invite`.
        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            matrix_room
                .get_state_event_static_for_key::<RoomMemberEventContent, _>(
                    matrix_room.own_user_id(),
                )
                .await
        });

        let raw_member_event = match handle.await.expect("task was not aborted") {
            Ok(Some(raw_member_event)) => raw_member_event,
            Ok(None) => {
                return false;
            }
            Err(event_error) => {
                error!("Could not get own member event: {event_error}");
                return false;
            }
        };

        let member_event = match raw_member_event {
            RawSyncOrStrippedState::Sync(raw) => {
                raw.deserialize_as_unchecked::<RoomMemberMembershipEvent>()
            }
            RawSyncOrStrippedState::Stripped(raw) => raw.deserialize_as_unchecked(),
        };

        let member_event = match member_event {
            Ok(member_event) => member_event,
            Err(deserialize_error) => {
                warn!("Could not deserialize room member event: {deserialize_error}");
                return false;
            }
        };

        // Check the current membership event, in case we did not get a state update
        // with the latest change.
        if member_event.content.membership == *membership {
            return true;
        }

        // Check the previous membership, in case we did get a state update with the
        // latest change.
        if let Some(prev_content) = member_event
            .unsigned
            .as_ref()
            .and_then(|unsigned| unsigned.prev_content.as_ref())
        {
            return prev_content.membership == *membership;
        }

        // If we do not have the `prev_content`, we need to fetch the previous state
        // event.
        let Some(replaces_state) = member_event
            .unsigned
            .and_then(|unsigned| unsigned.replaces_state)
        else {
            return false;
        };

        let matrix_room = self.matrix_room.clone();
        let handle =
            spawn_tokio!(
                async move { matrix_room.load_or_fetch_event(&replaces_state, None).await }
            );

        let raw_prev_member_event = match handle.await.expect("task was not aborted") {
            Ok(event) => event,
            Err(fetch_error) => {
                warn!("Could not fetch previous member event: {fetch_error}");
                return false;
            }
        };

        match raw_prev_member_event
            .kind
            .raw()
            .deserialize_as_unchecked::<RoomMemberMembershipEvent>()
        {
            Ok(prev_member_event) => prev_member_event.content.membership == *membership,
            Err(deserialize_error) => {
                warn!("Could not deserialize previous member event: {deserialize_error}");
                false
            }
        }
    }

    /// Set the number of joined members in the room, according to the
    /// homeserver.
    fn set_joined_members_count(&self, count: u64) {
        self.joined_members_count.set_if_not_eq(count);
    }

    /// Update whether this room is marked as unread.
    ///
    /// Kept `async` for shape parity with the application, where the read
    /// state is recomputed from the timeline here.
    #[allow(clippy::unused_async)]
    async fn update_is_marked_unread(&self) {
        let is_marked_unread = self.matrix_room.is_marked_unread();

        if self.is_marked_unread.get() == is_marked_unread {
            return;
        }

        self.is_marked_unread.set(is_marked_unread);

        // Until the timeline is extracted there is no unread-messages
        // computation: the flag is the only thing that moves `is_read`.
        if is_marked_unread {
            self.is_read.set_if_not_eq(false);
        }
        self.update_highlight();
    }

    /// Update the highlight of the room from the current state.
    fn update_highlight(&self) {
        let highlight;

        if matches!(self.category.get(), RoomCategory::Left) {
            // Consider that all left rooms are read.
            highlight = RoomHighlight::None;
            self.set_notification_count(0);
        } else if self.is_read.get() {
            highlight = RoomHighlight::None;
            self.set_notification_count(0);
        } else {
            let counts = self.matrix_room.unread_notification_counts();

            highlight = if counts.highlight_count > 0 {
                RoomHighlight::Highlight
            } else {
                RoomHighlight::Bold
            };
            self.set_notification_count(counts.notification_count);
        }

        self.highlight.set_if_not_eq(highlight);
    }

    /// Set the number of unread notifications of this room.
    fn set_notification_count(&self, count: u64) {
        if self.notification_count.get() == count {
            return;
        }

        self.notification_count.set(count);
        self.has_notifications.set_if_not_eq(count > 0);
    }

    /// Update whether the room is encrypted from the SDK.
    async fn update_is_encrypted(&self) {
        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.latest_encryption_state().await });

        match handle.await.expect("task was not aborted") {
            Ok(state) => {
                if state.is_encrypted() {
                    self.is_encrypted.set_if_not_eq(true);
                }
            }
            Err(encryption_error) => {
                // It can be expected to not be allowed to access the encryption state if the
                // user was never in the room, so do not add noise in the logs.
                if matches!(
                    self.matrix_room.state(),
                    RoomState::Invited | RoomState::Knocked
                ) && encryption_error
                    .as_client_api_error()
                    .is_some_and(|e| e.status_code.is_client_error())
                {
                    debug!("Could not load room encryption state: {encryption_error}");
                } else {
                    error!("Could not load room encryption state: {encryption_error}");
                }
            }
        }
    }

    /// Watch the SDK's room info for changes to the room state.
    fn watch_room_info(self: &Arc<Self>) {
        let subscriber = self.matrix_room.subscribe_info();
        let weak = Arc::downgrade(self);

        let handle = RUNTIME
            .spawn(async move {
                let mut subscriber = std::pin::pin!(subscriber);
                while let Some(room_info) = subscriber.next().await {
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };
                    inner.update_with_room_info(room_info).await;
                }
            })
            .abort_handle();

        *self.room_info_handle.lock().expect("mutex is not poisoned") = Some(handle);
    }

    /// Update this room with the given SDK room info.
    async fn update_with_room_info(self: &Arc<Self>, room_info: RoomInfo) {
        self.update_name();
        self.update_display_name().await;
        self.update_avatar();
        self.update_topic();
        self.update_category().await;
        self.update_is_direct().await;
        self.update_is_marked_unread().await;
        Self::update_tombstone(self);
        self.set_joined_members_count(room_info.joined_members_count());
        self.update_is_encrypted().await;
        // The aliases, server notice, pinned events, join rule, guest access
        // and history visibility updates attach here with their chunks.
    }

    /// Change the category of this room.
    ///
    /// This makes the necessary to propagate the category to the
    /// homeserver.
    ///
    /// This can be used to trigger actions like join or leave, as well as
    /// changing the category in the sidebar.
    ///
    /// Note that rooms cannot change category once they are upgraded.
    async fn change_category(self: &Arc<Self>, category: TargetRoomCategory) -> MatrixResult<()> {
        let previous_category = self.category.get();

        if previous_category == category {
            return Ok(());
        }

        if previous_category == RoomCategory::Outdated {
            warn!("Cannot change the category of an upgraded room");
            return Ok(());
        }

        self.set_category(category.into());

        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let room_state = matrix_room.state();

            match category {
                TargetRoomCategory::Favorite => {
                    if !matrix_room.is_favourite() {
                        // This method handles removing the low priority tag.
                        matrix_room.set_is_favourite(true, None).await?;
                    } else if matrix_room.is_low_priority() {
                        matrix_room.set_is_low_priority(false, None).await?;
                    }

                    if matches!(room_state, RoomState::Invited | RoomState::Left) {
                        matrix_room.join().await?;
                    }
                }
                TargetRoomCategory::Normal => {
                    if matrix_room.is_favourite() {
                        matrix_room.set_is_favourite(false, None).await?;
                    }
                    if matrix_room.is_low_priority() {
                        matrix_room.set_is_low_priority(false, None).await?;
                    }

                    if matches!(room_state, RoomState::Invited | RoomState::Left) {
                        matrix_room.join().await?;
                    }
                }
                TargetRoomCategory::LowPriority => {
                    if !matrix_room.is_low_priority() {
                        // This method handles removing the favourite tag.
                        matrix_room.set_is_low_priority(true, None).await?;
                    } else if matrix_room.is_favourite() {
                        matrix_room.set_is_favourite(false, None).await?;
                    }

                    if matches!(room_state, RoomState::Invited | RoomState::Left) {
                        matrix_room.join().await?;
                    }
                }
                TargetRoomCategory::Left => {
                    if matches!(
                        room_state,
                        RoomState::Knocked | RoomState::Invited | RoomState::Joined
                    ) {
                        matrix_room.leave().await?;
                    }
                }
            }

            Result::<_, matrix_sdk::Error>::Ok(())
        });

        match handle.await.expect("task was not aborted") {
            Ok(()) => Ok(()),
            Err(category_error) => {
                error!("Could not set the room category: {category_error}");

                // Reset the category
                Box::pin(self.update_category()).await;

                Err(category_error)
            }
        }
    }
}

/// Helper type to extract the current and previous memberships from a raw
/// `m.room.member` event.
#[derive(Deserialize)]
struct RoomMemberMembershipEvent {
    content: RoomMemberMembershipContent,
    unsigned: Option<RoomMemberMembershipUnsigned>,
}

/// Helper type to extract the membership of the `unsigned` object of an
/// `m.room.member` event.
#[derive(Deserialize)]
struct RoomMemberMembershipUnsigned {
    replaces_state: Option<ruma::OwnedEventId>,
    prev_content: Option<RoomMemberMembershipContent>,
}

/// Helper type to extract the membership of the `content` object of an
/// `m.room.member` event.
#[derive(Deserialize)]
struct RoomMemberMembershipContent {
    membership: MembershipState,
}

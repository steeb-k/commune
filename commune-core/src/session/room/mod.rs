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

mod aliases;
mod category;
mod composer;
mod join_rule;
mod media_history;
mod member;
mod permissions;
mod search;
mod server_acl;
mod timeline;
mod upgrade;

use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

use eyeball::{SharedObservable, Subscriber};
use futures_util::StreamExt;
use matrix_sdk::{
    Result as MatrixResult, RoomDisplayName as SdkRoomDisplayName, RoomInfo, RoomState,
    deserialized_responses::{AmbiguityChange, RawSyncOrStrippedState},
    room::Room as MatrixRoom,
    send_queue::RoomSendQueueUpdate,
};
use ruma::{
    EventId, MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedMxcUri, OwnedRoomId, OwnedUserId,
    RoomId, UInt,
    api::{
        client::{
            directory::{get_room_visibility, set_room_visibility},
            receipt::create_receipt::v3::ReceiptType as ApiReceiptType,
            room::Visibility,
        },
        error::{ErrorKind, LimitExceededErrorData, RetryAfter},
    },
    events::{
        AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
        room::{
            avatar::ImageInfo as AvatarImageInfo,
            guest_access::GuestAccess,
            history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
            member::{MembershipState, RoomMemberEventContent, SyncRoomMemberEvent},
            message::MessageType,
        },
        tag::TagName,
    },
    matrix_uri::MatrixToUri,
    room_version_rules::RoomVersionRules,
    serde::Raw,
};
use serde::Deserialize;
use tokio::{task::AbortHandle, time::sleep};
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, error, warn};

pub use self::{
    aliases::{AliasError, AliasesState, RoomAliases},
    category::{RoomCategory, RoomHighlight, TargetRoomCategory},
    composer::{ComposerChunk, compose_message, emoticon_plain},
    join_rule::{JoinRule, JoinRuleState, JoinRuleValue, compute_join_rule},
    media_history::{MediaHistoryError, MediaHistoryEvent, MediaHistoryKind, MediaHistoryPage},
    member::{Member, MemberList, MemberRole, Membership},
    permissions::{
        POWER_LEVEL_MAX, Permissions, PermissionsError, PermissionsState, PowerLevelsMatrix,
    },
    search::{RoomSearch, SearchError, SearchResult},
    server_acl::{AclProblem, ServerAclError, acls_are_equal, check_acl, unrestricted_acl},
    timeline::{
        MAX_BATCH_SIZE, ReceiptPosition, Timeline, TimelineError, TimelineFocusKind,
        check_upload_size,
    },
    upgrade::{UpgradeInfo, cmp_room_versions},
};
use crate::{
    RUNTIME, UserFacingError,
    session::{Session, WeakSession, room_list::RoomMetainfo},
    spawn_tokio,
    utils::{OptionStringExt, StrMutExt},
};

/// The tag order of a room that has none.
///
/// The specification keeps real orders in `[0, 1]` and asks that ordered
/// rooms come first, so anything past 1 sorts a room after all of them.
const NO_TAG_ORDER: f64 = 2.0;

/// The default duration in seconds that we wait for before retrying failed
/// sending requests.
const DEFAULT_RETRY_AFTER: u64 = 30;

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

/// A server notice that is still active.
///
/// The homeserver pins the notice it wants shown in the server notices
/// room; this is the most recent one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerNotice {
    /// The body of the notice.
    pub body: String,
    /// The contact method for the administrator of the homeserver, if the
    /// notice gives one.
    pub admin_contact: Option<String>,
}

/// An error encountered while changing the details of a room.
#[derive(Debug, thiserror::Error)]
pub enum RoomDetailsError {
    /// The room is not joined, so its details cannot be changed.
    #[error("the room is not joined")]
    NotJoined,
    /// The name could not be changed.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` here expensive.
    #[error(transparent)]
    Name(Box<matrix_sdk::Error>),
    /// The topic could not be changed.
    #[error(transparent)]
    Topic(Box<matrix_sdk::Error>),
}

impl UserFacingError for RoomDetailsError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NotJoined => "The room is not joined".to_owned(),
            Self::Name(_) => "Could not change room name".to_owned(),
            Self::Topic(_) => "Could not change room description".to_owned(),
        }
    }
}

/// An error encountered while changing the settings of a room.
#[derive(Debug, thiserror::Error)]
pub enum RoomSettingsError {
    /// The room is not joined, so its settings cannot be changed.
    #[error("the room is not joined")]
    NotJoined,
    /// The value cannot be sent: the application never offers it.
    #[error("the value is not supported")]
    Unsupported,
    /// The join rule could not be changed.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` here expensive.
    #[error(transparent)]
    JoinRule(Box<matrix_sdk::Error>),
    /// The history visibility could not be changed.
    #[error(transparent)]
    HistoryVisibility(Box<matrix_sdk::Error>),
    /// Whether the room is published in the directory could not be read.
    #[error(transparent)]
    Directory(Box<matrix_sdk::Error>),
    /// The room could not be published in, or withdrawn from, the
    /// directory.
    #[error("could not {} the room", if *published { "publish" } else { "unpublish" })]
    Publish {
        /// Whether the room was being published.
        published: bool,
        /// The request's error.
        error: Box<matrix_sdk::Error>,
    },
    /// The avatar could not be uploaded.
    #[error(transparent)]
    AvatarUpload(Box<matrix_sdk::Error>),
    /// The avatar could not be changed.
    #[error(transparent)]
    Avatar(Box<matrix_sdk::Error>),
    /// The avatar could not be removed.
    #[error(transparent)]
    AvatarRemove(Box<matrix_sdk::Error>),
}

impl UserFacingError for RoomSettingsError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NotJoined => "The room is not joined".to_owned(),
            Self::Unsupported => "This value is not supported".to_owned(),
            Self::JoinRule(_) => "Could not change who can join".to_owned(),
            Self::HistoryVisibility(_) => "Could not change who can read history".to_owned(),
            Self::Directory(_) => "Could not get directory visibility of room".to_owned(),
            Self::Publish {
                published: true, ..
            } => "Could not publish room in directory".to_owned(),
            Self::Publish {
                published: false, ..
            } => "Could not unpublish room from directory".to_owned(),
            Self::AvatarUpload(_) => "Could not upload avatar".to_owned(),
            Self::Avatar(_) => "Could not change avatar".to_owned(),
            Self::AvatarRemove(_) => "Could not remove avatar".to_owned(),
        }
    }
}

/// Who can read the history of a room.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HistoryVisibilityValue {
    /// Anyone can read.
    WorldReadable,
    /// Members, since this was selected.
    #[default]
    Shared,
    /// Members, since they were invited.
    Invited,
    /// Members, since they joined.
    Joined,
    /// Unsupported value.
    Unsupported,
}

impl From<HistoryVisibility> for HistoryVisibilityValue {
    fn from(value: HistoryVisibility) -> Self {
        match value {
            HistoryVisibility::Invited => Self::Invited,
            HistoryVisibility::Joined => Self::Joined,
            HistoryVisibility::Shared => Self::Shared,
            HistoryVisibility::WorldReadable => Self::WorldReadable,
            _ => Self::Unsupported,
        }
    }
}

impl HistoryVisibilityValue {
    /// The specification's value, if this is one that can be sent.
    ///
    /// The application's conversion panics on the unsupported value, which
    /// its page never selects; the core refuses instead.
    fn to_history_visibility(self) -> Option<HistoryVisibility> {
        match self {
            Self::Invited => Some(HistoryVisibility::Invited),
            Self::Joined => Some(HistoryVisibility::Joined),
            Self::Shared => Some(HistoryVisibility::Shared),
            Self::WorldReadable => Some(HistoryVisibility::WorldReadable),
            Self::Unsupported => None,
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
    /// The display name of the direct member, if it is known.
    direct_member_display_name: SharedObservable<Option<String>>,
    /// Whether this room has been upgraded.
    is_tombstoned: SharedObservable<bool>,
    /// The ID of the room that was upgraded and that this one replaces.
    predecessor_id: std::sync::OnceLock<OwnedRoomId>,
    /// The ID of the successor of this Room, if this room was upgraded.
    successor_id: SharedObservable<Option<OwnedRoomId>>,
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
    /// Whether guests are allowed.
    guests_allowed: SharedObservable<bool>,
    /// The user who invited us, while this room is an invitation.
    inviter_user_id: SharedObservable<Option<OwnedUserId>>,
    /// The event IDs pinned in this room, oldest first.
    ///
    /// Empty in the server notices room: there the pinned events are the
    /// active notices, and the spec asks for them to be shown "through a
    /// special UI, and not the normal pinned events interface".
    pinned_event_ids: SharedObservable<Vec<OwnedEventId>>,
    /// The active server notice of this room, if any.
    ///
    /// A notice is active while it is pinned in the server notices room.
    /// This is always `None` outside of that room.
    active_server_notice: SharedObservable<Option<ServerNotice>>,
    /// The pinned event IDs that `active_server_notice` was computed from.
    server_notice_pinned_ids: Mutex<Vec<OwnedEventId>>,
    /// Whether an embedder reports the read state from a timeline of its
    /// own.
    ///
    /// While one does, the notification-count approximation stays out of
    /// its way.
    read_state_reported: AtomicBool,
    /// The task watching the send queue for recoverable errors.
    send_queue_handle: Mutex<Option<AbortHandle>>,
    /// The task re-reading the category when the ignored users change.
    ignored_users_handle: Mutex<Option<AbortHandle>>,
    /// The aliases of this room.
    aliases: RoomAliases,
    /// The join rule of this room.
    join_rule: JoinRule,
    /// The permissions of our own user in this room.
    permissions: Permissions,
    /// Who can read the history of this room.
    history_visibility: SharedObservable<HistoryVisibilityValue>,
    /// Whether this room was forgotten.
    forgotten: SharedObservable<bool>,
    /// The guard of the handler following the room's member events.
    members_guard: Mutex<Option<matrix_sdk::event_handler::EventHandlerDropGuard>>,
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
    /// The member list of this room, built on first use.
    member_list: std::sync::OnceLock<MemberList>,
    /// The users currently typing in this room, our own user excluded.
    typing: SharedObservable<Vec<OwnedUserId>>,
    /// The typing subscription's event-handler guard and task.
    typing_guard: Mutex<Option<matrix_sdk::event_handler::EventHandlerDropGuard>>,
    typing_handle: Mutex<Option<AbortHandle>>,
}

impl Drop for RoomInner {
    fn drop(&mut self) {
        if let Ok(Some(handle)) = self.room_info_handle.get_mut().map(Option::take) {
            handle.abort();
        }
        if let Ok(Some(handle)) = self.typing_handle.get_mut().map(Option::take) {
            handle.abort();
        }
        if let Ok(Some(handle)) = self.send_queue_handle.get_mut().map(Option::take) {
            handle.abort();
        }
        if let Ok(Some(handle)) = self.ignored_users_handle.get_mut().map(Option::take) {
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
            aliases: RoomAliases::new(matrix_room.clone()),
            join_rule: JoinRule::new(matrix_room.clone(), session.downgrade()),
            permissions: Permissions::new(matrix_room.clone()),
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
            direct_member_display_name: SharedObservable::new(None),
            is_tombstoned: SharedObservable::new(false),
            predecessor_id: std::sync::OnceLock::new(),
            successor_id: SharedObservable::new(None),
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
            guests_allowed: SharedObservable::new(false),
            inviter_user_id: SharedObservable::new(None),
            pinned_event_ids: SharedObservable::new(Vec::new()),
            active_server_notice: SharedObservable::new(None),
            server_notice_pinned_ids: Mutex::new(Vec::new()),
            read_state_reported: AtomicBool::new(false),
            send_queue_handle: Mutex::new(None),
            ignored_users_handle: Mutex::new(None),
            history_visibility: SharedObservable::new(HistoryVisibilityValue::default()),
            forgotten: SharedObservable::new(false),
            members_guard: Mutex::new(None),
            is_room_info_initialized: SharedObservable::new(false),
            attempted_auto_join: AtomicBool::new(false),
            room_info_handle: Mutex::new(None),
            live_timeline: std::sync::OnceLock::new(),
            member_list: std::sync::OnceLock::new(),
            typing: SharedObservable::new(Vec::new()),
            typing_guard: Mutex::new(None),
            typing_handle: Mutex::new(None),
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

        RoomInner::set_up_typing(inner);

        RoomInner::watch_send_queue(inner);
        RoomInner::watch_ignored_users(inner);
        RoomInner::watch_members(inner);

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
    pub fn successor_id(&self) -> Option<OwnedRoomId> {
        self.inner.successor_id.get()
    }

    /// Subscribe to the ID of the successor of this room.
    pub fn subscribe_successor_id(&self) -> Subscriber<Option<OwnedRoomId>> {
        self.inner.successor_id.subscribe()
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

    /// Subscribe to whether this room is encrypted.
    pub fn subscribe_is_encrypted(&self) -> Subscriber<bool> {
        self.inner.is_encrypted.subscribe()
    }

    /// Subscribe to the name that is set for this room.
    pub fn subscribe_name(&self) -> Subscriber<Option<String>> {
        self.inner.name.subscribe()
    }

    /// Subscribe to whether this room has an avatar explicitly set.
    pub fn subscribe_has_avatar(&self) -> Subscriber<bool> {
        self.inner.has_avatar.subscribe()
    }

    /// Subscribe to the topic of this room.
    pub fn subscribe_topic(&self) -> Subscriber<Option<String>> {
        self.inner.topic.subscribe()
    }

    /// Subscribe to the order of this room inside its tag.
    pub fn subscribe_tag_order(&self) -> Subscriber<f64> {
        self.inner.tag_order.subscribe()
    }

    /// Subscribe to whether this room is a direct chat.
    pub fn subscribe_is_direct(&self) -> Subscriber<bool> {
        self.inner.is_direct.subscribe()
    }

    /// Subscribe to the other user of this direct chat.
    pub fn subscribe_direct_member_user_id(&self) -> Subscriber<Option<OwnedUserId>> {
        self.inner.direct_member_user_id.subscribe()
    }

    /// Subscribe to whether this room has been upgraded.
    pub fn subscribe_is_tombstoned(&self) -> Subscriber<bool> {
        self.inner.is_tombstoned.subscribe()
    }

    /// Subscribe to the number of joined members in the room.
    pub fn subscribe_joined_members_count(&self) -> Subscriber<u64> {
        self.inner.joined_members_count.subscribe()
    }

    /// Subscribe to whether this room is a current invite or an invite
    /// that was declined or retracted.
    pub fn subscribe_is_invite(&self) -> Subscriber<bool> {
        self.inner.is_invite.subscribe()
    }

    /// Whether this room is marked as unread.
    #[must_use]
    pub fn is_marked_unread(&self) -> bool {
        self.inner.is_marked_unread.get()
    }

    /// Subscribe to whether this room is marked as unread.
    pub fn subscribe_is_marked_unread(&self) -> Subscriber<bool> {
        self.inner.is_marked_unread.subscribe()
    }

    /// Subscribe to whether this room has unread notifications.
    pub fn subscribe_has_notifications(&self) -> Subscriber<bool> {
        self.inner.has_notifications.subscribe()
    }

    /// Whether the room info is initialized.
    #[must_use]
    pub fn is_room_info_initialized(&self) -> bool {
        self.inner.is_room_info_initialized.get()
    }

    /// Subscribe to whether the room info is initialized.
    pub fn subscribe_is_room_info_initialized(&self) -> Subscriber<bool> {
        self.inner.is_room_info_initialized.subscribe()
    }

    /// Whether we already attempted an auto-join.
    #[must_use]
    pub fn attempted_auto_join(&self) -> bool {
        self.inner.attempted_auto_join.load(Ordering::Relaxed)
    }

    /// Whether guests are allowed.
    #[must_use]
    pub fn guests_allowed(&self) -> bool {
        self.inner.guests_allowed.get()
    }

    /// Subscribe to whether guests are allowed.
    pub fn subscribe_guests_allowed(&self) -> Subscriber<bool> {
        self.inner.guests_allowed.subscribe()
    }

    /// The user who invited us, while this room is an invitation.
    #[must_use]
    pub fn inviter_user_id(&self) -> Option<OwnedUserId> {
        self.inner.inviter_user_id.get()
    }

    /// Subscribe to the user who invited us.
    pub fn subscribe_inviter_user_id(&self) -> Subscriber<Option<OwnedUserId>> {
        self.inner.inviter_user_id.subscribe()
    }

    /// The event IDs pinned in this room, oldest first.
    ///
    /// Empty in the server notices room, whose pinned events are the
    /// active notices.
    #[must_use]
    pub fn pinned_event_ids(&self) -> Vec<OwnedEventId> {
        self.inner.pinned_event_ids.get()
    }

    /// Subscribe to the event IDs pinned in this room.
    pub fn subscribe_pinned_event_ids(&self) -> Subscriber<Vec<OwnedEventId>> {
        self.inner.pinned_event_ids.subscribe()
    }

    /// Whether the event with the given ID is pinned in this room.
    #[must_use]
    pub fn is_pinned(&self, event_id: &EventId) -> bool {
        self.inner
            .pinned_event_ids
            .read()
            .iter()
            .any(|pinned| pinned == event_id)
    }

    /// The active server notice of this room, if any.
    #[must_use]
    pub fn active_server_notice(&self) -> Option<ServerNotice> {
        self.inner.active_server_notice.get()
    }

    /// Subscribe to the active server notice of this room.
    pub fn subscribe_active_server_notice(&self) -> Subscriber<Option<ServerNotice>> {
        self.inner.active_server_notice.subscribe()
    }

    /// Take the read state an embedder computed from a timeline of its own.
    ///
    /// The application's timeline model walks the room's events the way
    /// the specification asks (MSC2654) and knows whether anything is
    /// unread; until that model is the core's, it reports here, and the
    /// observable the metainfo persists is the one it set. From the first
    /// report on, the core's notification-count approximation stands
    /// aside.
    pub fn note_is_read(&self, is_read: bool) {
        self.inner
            .read_state_reported
            .store(true, Ordering::Relaxed);
        self.inner.is_read.set_if_not_eq(is_read);
        self.inner.update_highlight();
    }

    /// Take a latest-activity timestamp an embedder found in a timeline of
    /// its own.
    ///
    /// Only ever moves the activity forward.
    pub fn note_latest_activity(&self, timestamp: u64) {
        let current = self.inner.latest_activity.get();
        self.inner
            .latest_activity
            .set_if_not_eq(current.max(timestamp));
    }

    /// Whether this room was forgotten.
    pub(crate) fn subscribe_forgotten(&self) -> Subscriber<bool> {
        self.inner.forgotten.subscribe()
    }

    /// Handle the members a sync marked ambiguous.
    ///
    /// The application refreshes the members named so that two "Alice"s
    /// are told apart the moment the second joins.
    pub(crate) fn note_ambiguity_changes<'a>(
        &self,
        changes: impl Iterator<Item = &'a AmbiguityChange>,
    ) {
        // Use a set to make sure we update members only once.
        let user_ids = changes
            .flat_map(AmbiguityChange::user_ids)
            .map(ToOwned::to_owned)
            .collect::<HashSet<_>>();

        if let Some(member_list) = self.inner.member_list.get() {
            member_list.update_members(user_ids.into_iter().collect());
        }
    }

    /// The live timeline of this room, created on first use.
    ///
    /// Creating it also starts the read-state watcher: from then on
    /// `is_read`, the highlight and the latest activity follow the
    /// timeline's items and our own read receipts, as the application's
    /// read-change trigger does.
    #[must_use]
    pub fn live_timeline(&self) -> Timeline {
        self.inner
            .live_timeline
            .get_or_init(|| {
                let timeline =
                    Timeline::new(self.inner.matrix_room.clone(), self.inner.session.clone());
                RoomInner::watch_read_state(&self.inner, &timeline);
                timeline
            })
            .clone()
    }

    /// The member list of this room.
    ///
    /// Built on first use; loading starts then, and room-info updates
    /// keep it fresh afterwards.
    #[must_use]
    pub fn member_list(&self) -> MemberList {
        self.inner
            .member_list
            .get_or_init(|| {
                let list = MemberList::new(self.inner.matrix_room.clone());
                let load_list = list.clone();
                RUNTIME.spawn(async move {
                    load_list.load().await;
                });
                list
            })
            .clone()
    }

    /// One page of the media history of this room — its images, videos,
    /// files and audio — going backwards from the given token, or from the
    /// end of the room without one.
    ///
    /// The application's history viewers keep the token between pages;
    /// the homeserver omits it from the last page.
    pub async fn media_history_page(
        &self,
        from: Option<&str>,
    ) -> Result<MediaHistoryPage, MediaHistoryError> {
        media_history::load_page(&self.inner.matrix_room, self.is_encrypted(), from).await
    }

    /// A timeline of the room's pinned events.
    ///
    /// A fresh timeline each call; the caller keeps it as long as the
    /// pinned view is open.
    #[must_use]
    pub fn pinned_timeline(&self) -> Timeline {
        Timeline::with_focus(
            self.inner.matrix_room.clone(),
            TimelineFocusKind::Pinned,
            self.inner.session.clone(),
        )
    }

    /// A timeline of the thread rooted at the given event.
    ///
    /// A fresh timeline each call; the caller keeps it as long as the
    /// thread is open.
    #[must_use]
    pub fn thread_timeline(&self, root: ruma::OwnedEventId) -> Timeline {
        Timeline::with_focus(
            self.inner.matrix_room.clone(),
            TimelineFocusKind::Thread { root },
            self.inner.session.clone(),
        )
    }

    /// Send the given receipt.
    ///
    /// The public-read-receipts setting decides whether a read receipt is
    /// public or private, as in the application.
    pub async fn send_receipt(&self, receipt_type: ApiReceiptType, position: ReceiptPosition) {
        let send_public_receipt = self
            .session()
            .is_none_or(|session| session.settings().public_read_receipts_enabled());

        let receipt_type = match receipt_type {
            ApiReceiptType::Read if !send_public_receipt => ApiReceiptType::ReadPrivate,
            t => t,
        };

        self.live_timeline()
            .send_receipt_resolved(receipt_type, position)
            .await;
    }

    /// Set the name of this room, or remove it with `None`.
    ///
    /// Whitespace around the name is not part of it and a name that is
    /// only whitespace removes it, as the details page trims what was
    /// typed. Refused when the room is not joined. Success is not
    /// reported beyond the request: the change comes back through sync.
    pub async fn set_name(&self, name: Option<&str>) -> Result<(), RoomDetailsError> {
        if !self.is_joined() {
            error!("Cannot change name of room not joined");
            return Err(RoomDetailsError::NotJoined);
        }

        let name = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_default();

        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.set_name(name).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|set_error| {
                error!("Could not change room name: {set_error}");
                RoomDetailsError::Name(Box::new(set_error))
            })
    }

    /// Set the topic of this room, or remove it with `None`.
    ///
    /// It is not possible to remove a topic, so an empty string is what
    /// stands for none. The same trimming and the same refusal as
    /// [`Room::set_name`].
    pub async fn set_topic(&self, topic: Option<&str>) -> Result<(), RoomDetailsError> {
        if !self.is_joined() {
            error!("Cannot change description of room not joined");
            return Err(RoomDetailsError::NotJoined);
        }

        let topic = topic
            .map(str::trim)
            .filter(|topic| !topic.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_default();

        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.set_room_topic(&topic).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|set_error| {
                error!("Could not change room description: {set_error}");
                RoomDetailsError::Topic(Box::new(set_error))
            })
    }

    /// Redact the given events in this room because of the given reason.
    ///
    /// Returns `Ok(())` if all the redactions are successful, otherwise
    /// returns the list of events that could not be redacted. Nothing is
    /// redacted in a room that is not joined.
    pub async fn redact(
        &self,
        events: &[OwnedEventId],
        reason: Option<String>,
    ) -> Result<(), Vec<OwnedEventId>> {
        if !self.is_joined() {
            return Ok(());
        }

        let events = events.to_owned();
        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let mut failed_redactions = Vec::new();

            for event_id in events {
                if let Err(redact_error) =
                    matrix_room.redact(&event_id, reason.as_deref(), None).await
                {
                    error!("Could not redact event with ID {event_id}: {redact_error}");
                    failed_redactions.push(event_id);
                }
            }

            failed_redactions
        });

        let failed_redactions = handle.await.expect("task was not aborted");
        if failed_redactions.is_empty() {
            Ok(())
        } else {
            Err(failed_redactions)
        }
    }

    /// Report the given events in this room.
    ///
    /// The events are a list of `(event_id, reason)` tuples.
    ///
    /// Returns `Ok(())` if all the reports are sent successfully, otherwise
    /// returns the list of event IDs that could not be reported.
    pub async fn report_events(
        &self,
        events: &[(OwnedEventId, Option<String>)],
    ) -> Result<(), Vec<OwnedEventId>> {
        let events = events.to_owned();
        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let futures = events.into_iter().map(|(event_id, reason)| {
                let matrix_room = matrix_room.clone();
                async move {
                    let result = matrix_room.report_content(event_id.clone(), reason).await;
                    (event_id, result)
                }
            });
            futures_util::future::join_all(futures).await
        });

        let mut failed = Vec::new();
        for (event_id, result) in handle.await.expect("task was not aborted") {
            if let Err(report_error) = result {
                error!("Could not report content with event ID {event_id}: {report_error}");
                failed.push(event_id);
            }
        }

        if failed.is_empty() {
            Ok(())
        } else {
            Err(failed)
        }
    }

    /// Invite the given users to this room.
    ///
    /// Returns `Ok(())` if all the invites are sent successfully, otherwise
    /// returns the list of users who could not be invited. Nobody is
    /// invited to a room that is not joined.
    pub async fn invite(&self, user_ids: &[OwnedUserId]) -> Result<(), Vec<OwnedUserId>> {
        if !self.is_joined() {
            error!("Can’t invite users, because this room isn’t a joined room");
            return Ok(());
        }

        let user_ids = user_ids.to_owned();
        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let invitations = user_ids.into_iter().map(|user_id| {
                let matrix_room = matrix_room.clone();
                async move {
                    let result = matrix_room.invite_user_by_id(&user_id).await;
                    (user_id, result)
                }
            });
            futures_util::future::join_all(invitations).await
        });

        let mut failed_invites = Vec::new();
        for (user_id, result) in handle.await.expect("task was not aborted") {
            if let Err(invite_error) = result {
                error!("Could not invite user with ID {user_id}: {invite_error}");
                failed_invites.push(user_id);
            }
        }

        if failed_invites.is_empty() {
            Ok(())
        } else {
            Err(failed_invites)
        }
    }

    /// The `matrix.to` URI representation for the given event in this
    /// room.
    ///
    /// With the routing the SDK computes; falling back to the room ID
    /// alone, without routing, when it cannot.
    pub async fn matrix_to_event_uri(&self, event_id: OwnedEventId) -> MatrixToUri {
        let matrix_room = self.inner.matrix_room.clone();

        let event_id_clone = event_id.clone();
        let handle =
            spawn_tokio!(
                async move { matrix_room.matrix_to_event_permalink(event_id_clone).await }
            );
        match handle.await.expect("task was not aborted") {
            Ok(permalink) => permalink,
            Err(permalink_error) => {
                error!("Could not get room event permalink: {permalink_error}");
                // Fallback to using just the room ID, without routing.
                self.room_id().matrix_to_event_uri(event_id)
            }
        }
    }

    /// The aliases of this room.
    #[must_use]
    pub fn aliases(&self) -> &RoomAliases {
        &self.inner.aliases
    }

    /// The join rule of this room.
    #[must_use]
    pub fn join_rule(&self) -> &JoinRule {
        &self.inner.join_rule
    }

    /// The permissions of our own user in this room.
    ///
    /// Loaded on first use: a reader that wants a current answer calls
    /// `ensure_loaded` first.
    #[must_use]
    pub fn permissions(&self) -> &Permissions {
        &self.inner.permissions
    }

    /// Who can read the history of this room.
    #[must_use]
    pub fn history_visibility(&self) -> HistoryVisibilityValue {
        self.inner.history_visibility.get()
    }

    /// Subscribe to who can read the history of this room.
    pub fn subscribe_history_visibility(&self) -> Subscriber<HistoryVisibilityValue> {
        self.inner.history_visibility.subscribe()
    }

    /// The rules for the version of this room.
    #[must_use]
    pub fn rules(&self) -> RoomVersionRules {
        self.inner
            .matrix_room
            .clone_info()
            .room_version_rules_or_default()
    }

    /// Change who can read the history of this room.
    ///
    /// The unsupported value is refused; the history-visibility page never
    /// offers it.
    pub async fn set_history_visibility(
        &self,
        value: HistoryVisibilityValue,
    ) -> Result<(), RoomSettingsError> {
        let visibility = value
            .to_history_visibility()
            .ok_or(RoomSettingsError::Unsupported)?;
        let content = RoomHistoryVisibilityEventContent::new(visibility);

        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(content).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|send_error| {
                error!("Could not change room history visibility: {send_error}");
                RoomSettingsError::HistoryVisibility(Box::new(send_error))
            })
    }

    /// Whether this room is published in the homeserver's directory.
    pub async fn is_published(&self) -> Result<bool, RoomSettingsError> {
        let client = self.inner.matrix_room.client();
        let request = get_room_visibility::v3::Request::new(self.room_id().to_owned());

        let handle = spawn_tokio!(async move { client.send(request).await });

        match handle.await.expect("task was not aborted") {
            Ok(response) => Ok(response.visibility == Visibility::Public),
            Err(directory_error) => {
                error!("Could not get directory visibility of room: {directory_error}");
                Err(RoomSettingsError::Directory(Box::new(
                    directory_error.into(),
                )))
            }
        }
    }

    /// Publish this room in the homeserver's directory, or withdraw it.
    pub async fn set_published(&self, published: bool) -> Result<(), RoomSettingsError> {
        let visibility = if published {
            Visibility::Public
        } else {
            Visibility::Private
        };

        let client = self.inner.matrix_room.client();
        let request = set_room_visibility::v3::Request::new(self.room_id().to_owned(), visibility);

        let handle = spawn_tokio!(async move { client.send(request).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|publish_error| {
                error!("Could not change directory visibility of room: {publish_error}");
                RoomSettingsError::Publish {
                    published,
                    error: Box::new(publish_error.into()),
                }
            })
    }

    /// Change the avatar of this room: upload the given image, then point
    /// `m.room.avatar` at it, with the dimensions the embedder decoded.
    ///
    /// Refused when the room is not joined. Success is not reported beyond
    /// the request: the change comes back through sync.
    pub async fn set_avatar(
        &self,
        mime: &mime::Mime,
        data: Vec<u8>,
        width: Option<UInt>,
        height: Option<UInt>,
    ) -> Result<(), RoomSettingsError> {
        if !self.is_joined() {
            error!("Cannot change avatar of room not joined");
            return Err(RoomSettingsError::NotJoined);
        }

        let mut image_info = AvatarImageInfo::new();
        image_info.width = width;
        image_info.height = height;
        image_info.size = u64::try_from(data.len()).ok().and_then(UInt::new);
        image_info.mimetype = Some(mime.to_string());

        let client = self.inner.matrix_room.client();
        let mime = mime.clone();
        let handle = spawn_tokio!(async move { client.media().upload(&mime, data, None).await });

        let uri: OwnedMxcUri = match handle.await.expect("task was not aborted") {
            Ok(response) => response.content_uri,
            Err(upload_error) => {
                error!("Could not upload room avatar: {upload_error}");
                return Err(RoomSettingsError::AvatarUpload(Box::new(upload_error)));
            }
        };

        let matrix_room = self.inner.matrix_room.clone();
        let handle =
            spawn_tokio!(async move { matrix_room.set_avatar_url(&uri, Some(image_info)).await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|set_error| {
                error!("Could not change room avatar: {set_error}");
                RoomSettingsError::Avatar(Box::new(set_error))
            })
    }

    /// Remove the avatar of this room.
    ///
    /// Refused when the room is not joined.
    pub async fn remove_avatar(&self) -> Result<(), RoomSettingsError> {
        if !self.is_joined() {
            error!("Cannot remove avatar of room not joined");
            return Err(RoomSettingsError::NotJoined);
        }

        let matrix_room = self.inner.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.remove_avatar().await });

        handle
            .await
            .expect("task was not aborted")
            .map(|_response| ())
            .map_err(|remove_error| {
                error!("Could not remove room avatar: {remove_error}");
                RoomSettingsError::AvatarRemove(Box::new(remove_error))
            })
    }

    /// The display name of the direct member, if this is a direct chat
    /// and it is known.
    #[must_use]
    pub fn direct_member_display_name(&self) -> Option<String> {
        self.inner.direct_member_display_name.get()
    }

    /// The users currently typing in this room, our own user excluded.
    #[must_use]
    pub fn typing_users(&self) -> Vec<OwnedUserId> {
        self.inner.typing.get()
    }

    /// Subscribe to the users currently typing in this room.
    pub fn subscribe_typing(&self) -> Subscriber<Vec<OwnedUserId>> {
        self.inner.typing.subscribe()
    }

    /// Send a typing notification for this room, with the given typing
    /// state.
    ///
    /// Does nothing when typing notifications are disabled in the session
    /// settings.
    pub fn send_typing_notification(&self, is_typing: bool) {
        if self.inner.matrix_room.state() != RoomState::Joined {
            return;
        }
        if self
            .session()
            .is_some_and(|session| !session.settings().typing_enabled())
        {
            return;
        }

        let matrix_room = self.inner.matrix_room.clone();
        RUNTIME.spawn(async move {
            if let Err(typing_error) = matrix_room.typing_notice(is_typing).await {
                error!("Could not send typing notification: {typing_error}");
            }
        });
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
    /// business. When the room has no avatar of its own, the direct
    /// member's avatar fills in — `update_direct_member` owns the
    /// observable in that case, so it is left alone here.
    fn update_avatar(&self) {
        let avatar_url = self.matrix_room.avatar_url();
        self.has_avatar.set_if_not_eq(avatar_url.is_some());

        if avatar_url.is_some() {
            self.avatar_url.set_if_not_eq(avatar_url);
        }
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
    fn set_category(self: &Arc<Self>, category: RoomCategory) {
        let old_category = self.category.get();

        if old_category == RoomCategory::Outdated || old_category == category {
            return;
        }

        self.category.set(category);

        // Check if the previous state was different.
        let room_state = self.matrix_room.state();
        if !old_category.is_state(room_state) {
            if self.is_room_info_initialized.get() {
                debug!(room_id = %self.matrix_room.room_id(), ?room_state, "The state of the room changed");
            }

            match room_state {
                RoomState::Joined => {
                    if let Some(members) = self.member_list.get() {
                        // If we where invited or left before, the list was likely not completed
                        // or might have changed.
                        members.reload();
                    }

                    self.set_up_typing();
                }
                RoomState::Left | RoomState::Knocked | RoomState::Banned | RoomState::Invited => {}
            }
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
        self.update_inviter().await;

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

                if self.is_inviter_ignored() {
                    RoomCategory::Ignored
                } else {
                    RoomCategory::Invited
                }
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
        self.direct_member_user_id
            .set_if_not_eq(direct_user_id.clone());

        let Some(direct_user_id) = direct_user_id else {
            self.direct_member_display_name.set_if_not_eq(None);
            return;
        };

        if let Some(member_list) = self.member_list.get() {
            let _direct_index = member_list.ensure(&direct_user_id);
        }

        // The one bit of member data the sidebar needs today: the person's
        // name and picture. The full member model comes with its chunk.
        let matrix_room = self.matrix_room.clone();
        let handle =
            spawn_tokio!(async move { matrix_room.get_member_no_sync(&direct_user_id).await });

        match handle.await.expect("task was not aborted") {
            Ok(Some(member)) => {
                self.direct_member_display_name
                    .set_if_not_eq(member.display_name().map(ToOwned::to_owned));

                if !self.has_avatar.get() {
                    self.avatar_url
                        .set_if_not_eq(member.avatar_url().map(ToOwned::to_owned));
                }
            }
            Ok(None) => {}
            Err(member_error) => {
                error!("Could not get direct member: {member_error}");
            }
        }
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
            self.successor_id.set_if_not_eq(Some(successor_id));
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
            .and_then(|successor_id| room_list.get(&successor_id))
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
    fn set_successor(self: &Arc<Self>, successor: &Room) {
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

    /// Start listening to typing events.
    ///
    /// Like the application, only joined rooms are listened to; a room
    /// joined later gets its subscription from `set_category()` when its
    /// state becomes joined.
    fn set_up_typing(self: &Arc<Self>) {
        if self
            .typing_guard
            .lock()
            .expect("mutex is not poisoned")
            .is_some()
        {
            // The event handler is already set up.
            return;
        }
        if self.matrix_room.state() != RoomState::Joined {
            return;
        }

        let (guard, receiver) = self.matrix_room.subscribe_to_typing_notifications();
        let own_user_id = self.matrix_room.own_user_id().to_owned();
        let weak = Arc::downgrade(self);

        let handle = RUNTIME
            .spawn(async move {
                let mut stream = BroadcastStream::new(receiver);

                while let Some(typing_user_ids) = stream.next().await {
                    let Ok(typing_user_ids) = typing_user_ids else {
                        continue;
                    };
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };

                    let typing: Vec<OwnedUserId> = typing_user_ids
                        .into_iter()
                        .filter(|user_id| *user_id != own_user_id)
                        .collect();
                    inner.typing.set_if_not_eq(typing);
                }
            })
            .abort_handle();

        *self.typing_guard.lock().expect("mutex is not poisoned") = Some(guard);
        *self.typing_handle.lock().expect("mutex is not poisoned") = Some(handle);
    }

    /// Follow the timeline's items and our own read receipts, keeping
    /// `is_read`, the highlight and the latest activity current — the
    /// application's read-change trigger, headless.
    fn watch_read_state(self: &Arc<Self>, timeline: &Timeline) {
        let weak = Arc::downgrade(self);
        let timeline = timeline.clone();

        RUNTIME.spawn(async move {
            let Some(mut receipts) = timeline.subscribe_own_read_receipts().await else {
                return;
            };
            let Some((_, mut items)) = timeline.subscribe_items().await else {
                return;
            };

            loop {
                if let Some(inner) = weak.upgrade() {
                    inner.update_read_state(&timeline).await;
                } else {
                    break;
                }

                tokio::select! {
                    changed = receipts.next() => if changed.is_none() { break },
                    changed = items.next() => if changed.is_none() { break },
                }
            }
        });
    }

    /// Recompute the read state from the timeline.
    async fn update_read_state(&self, timeline: &Timeline) {
        if self.is_marked_unread.get() {
            self.is_read.set_if_not_eq(false);
        } else if let Some(has_unread) = timeline.has_unread_messages().await {
            self.is_read.set_if_not_eq(!has_unread);
        }

        if let Some(latest_activity) = timeline.latest_activity().await {
            let current = self.latest_activity.get();
            self.latest_activity
                .set_if_not_eq(current.max(latest_activity));
        }

        self.update_highlight();
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
        self.update_active_server_notice().await;
        self.update_pinned_events();
        self.update_is_direct().await;
        self.update_is_marked_unread().await;
        Self::update_tombstone(self);
        self.set_joined_members_count(room_info.joined_members_count());
        self.update_is_encrypted().await;

        // Memberships or power levels may be what changed; a built member
        // list follows along.
        if let Some(member_list) = self.member_list.get() {
            member_list.refresh().await;
        }

        // Without a built timeline there is no MSC2654 walk to decide
        // `is_read`, so for rooms that were never opened the server's own
        // notification accounting is the signal: notifications pending
        // means unread. The application reaches the same states through
        // its preloaded timelines; the read-state watcher takes over here
        // the moment the room is opened.
        if !self.read_state_reported.load(Ordering::Relaxed)
            && self
                .matrix_room
                .unread_notification_counts()
                .notification_count
                > 0
        {
            self.is_read.set_if_not_eq(false);
        }
        self.update_highlight();
        self.aliases.update();
        self.join_rule.update(room_info.join_rule());
        self.permissions.update_is_joined();
        self.update_history_visibility();
        self.update_guests_allowed();
    }

    /// Watch the room's member events.
    fn watch_members(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let handle = self
            .matrix_room
            .add_event_handler(move |event: SyncRoomMemberEvent| {
                let weak = weak.clone();
                async move {
                    if let Some(inner) = weak.upgrade() {
                        inner.handle_member_event(&event).await;
                    }
                }
            });

        let guard = self.matrix_room.client().event_handler_drop_guard(handle);
        *self.members_guard.lock().expect("mutex is not poisoned") = Some(guard);
    }

    /// Handle a member event received via sync.
    async fn handle_member_event(self: &Arc<Self>, event: &SyncRoomMemberEvent) {
        let user_id = event.state_key();

        // "If the client sees the user it is in a call with leave the
        // room, the client should treat this as a hangup event for any
        // calls that are in progress." They cannot send one from outside
        // the room, so nothing else is coming.
        if matches!(
            event.membership(),
            MembershipState::Leave | MembershipState::Ban
        ) && let Some(session) = self.session.upgrade()
        {
            let room = Room {
                inner: self.clone(),
            };
            session.calls().handle_member_left(&room, user_id);
        }

        if let Some(member_list) = self.member_list.get() {
            member_list.update_member(user_id.to_owned());
        }

        // It might change the direct member if the number of members
        // changed.
        self.update_direct_member().await;
    }

    /// Update whether guests are allowed.
    fn update_guests_allowed(&self) {
        let guests_allowed = self.matrix_room.guest_access() == GuestAccess::CanJoin;
        self.guests_allowed.set_if_not_eq(guests_allowed);
    }

    /// Update the user who invited us to this room.
    ///
    /// Only a current invite has one.
    async fn update_inviter(&self) {
        if self.matrix_room.state() != RoomState::Invited {
            self.inviter_user_id.set_if_not_eq(None);
            return;
        }

        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.invite_details().await });

        match handle.await.expect("task was not aborted") {
            Ok(invite) => {
                self.inviter_user_id
                    .set_if_not_eq(invite.inviter.map(|member| member.user_id().to_owned()));
            }
            Err(invite_error) => {
                error!("Could not get invite: {invite_error}");
            }
        }
    }

    /// Whether the user who invited us is one this account ignores.
    ///
    /// The specification says invites from ignored users are to be ignored.
    fn is_inviter_ignored(&self) -> bool {
        let Some(inviter) = self.inviter_user_id.get() else {
            return false;
        };

        self.session
            .upgrade()
            .is_some_and(|session| session.ignored_users().contains(&inviter))
    }

    /// Re-read the category when the ignored users change, because an
    /// invite from somebody just ignored stops being one.
    fn watch_ignored_users(self: &Arc<Self>) {
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let mut subscriber = session.ignored_users().subscribe();
        let weak = Arc::downgrade(self);

        let handle = RUNTIME
            .spawn(async move {
                while subscriber.next().await.is_some() {
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };

                    if inner.matrix_room.state() == RoomState::Invited {
                        inner.update_category().await;
                    }
                }
            })
            .abort_handle();

        *self
            .ignored_users_handle
            .lock()
            .expect("mutex is not poisoned") = Some(handle);
    }

    /// Update the active server notice of this room.
    ///
    /// The spec represents the notices that are still active as the pinned
    /// events of the server notices room, and asks that they be shown
    /// through a UI of their own rather than through the usual pinned
    /// events interface. The most recent one is the notice.
    async fn update_active_server_notice(&self) {
        let pinned_ids = if self.category.get() == RoomCategory::ServerNotice {
            self.matrix_room.pinned_event_ids().unwrap_or_default()
        } else {
            // Outside the server notices room, a pinned `m.server_notice`
            // means nothing: the spec says such an event must be ignored.
            Vec::new()
        };

        {
            let mut computed_from = self
                .server_notice_pinned_ids
                .lock()
                .expect("mutex is not poisoned");
            if *computed_from == pinned_ids {
                return;
            }
            computed_from.clone_from(&pinned_ids);
        }

        let matrix_room = self.matrix_room.clone();
        let handle = spawn_tokio!(async move {
            // The server pins the notice it wants shown; when several are
            // pinned, the last one is the most recent.
            for event_id in pinned_ids.iter().rev() {
                let event = match matrix_room.load_or_fetch_event(event_id, None).await {
                    Ok(event) => event,
                    Err(load_error) => {
                        warn!("Could not load pinned event {event_id}: {load_error}");
                        continue;
                    }
                };

                let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
                    SyncMessageLikeEvent::Original(event),
                ))) = event.raw().deserialize()
                else {
                    continue;
                };

                if let MessageType::ServerNotice(content) = event.content.msgtype {
                    return Some(ServerNotice {
                        body: content.body,
                        admin_contact: content.admin_contact,
                    });
                }
            }

            None
        });

        let notice = handle.await.expect("task was not aborted");
        self.active_server_notice.set_if_not_eq(notice);
    }

    /// Update the events pinned in this room.
    ///
    /// The server notices room is excluded on purpose: there the pinned
    /// events are the notices that are still active, which the spec asks
    /// to be shown "through a special UI, and not the normal pinned
    /// events interface". `update_active_server_notice` is that UI.
    fn update_pinned_events(&self) {
        let pinned_event_ids = if self.category.get() == RoomCategory::ServerNotice {
            Vec::new()
        } else {
            self.matrix_room.pinned_event_ids().unwrap_or_default()
        };

        self.pinned_event_ids.set_if_not_eq(pinned_event_ids);
    }

    /// Watch errors in the send queue to try to handle them.
    ///
    /// A recoverable error stops the queue; it is started again after the
    /// delay the homeserver asked for, or a default, unless the session is
    /// offline — then coming back online restarts it.
    fn watch_send_queue(self: &Arc<Self>) {
        let matrix_room = self.matrix_room.clone();
        let weak = Arc::downgrade(self);

        let handle = RUNTIME
            .spawn(async move {
                let send_queue = matrix_room.send_queue();
                let mut subscriber = match send_queue.subscribe().await {
                    Ok((_, subscriber)) => BroadcastStream::new(subscriber),
                    Err(subscribe_error) => {
                        warn!("Failed to listen to room send queue: {subscribe_error}");
                        return;
                    }
                };

                while let Some(update) = subscriber.next().await {
                    let Ok(RoomSendQueueUpdate::SendError {
                        error,
                        is_recoverable: true,
                        ..
                    }) = update
                    else {
                        continue;
                    };
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };
                    let Some(session) = inner.session.upgrade() else {
                        break;
                    };

                    if session.is_offline() {
                        // The queue will be restarted when the session is
                        // back online.
                        continue;
                    }

                    let duration = match error.client_api_error_kind() {
                        Some(ErrorKind::LimitExceeded(LimitExceededErrorData {
                            retry_after: Some(retry_after),
                            ..
                        })) => match retry_after {
                            RetryAfter::Delay(duration) => Some(*duration),
                            RetryAfter::DateTime(time) => {
                                time.duration_since(SystemTime::now()).ok()
                            }
                        },
                        _ => None,
                    };
                    let retry_after = duration.unwrap_or(Duration::from_secs(DEFAULT_RETRY_AFTER));

                    let matrix_room = inner.matrix_room.clone();
                    RUNTIME.spawn(async move {
                        sleep(retry_after).await;
                        matrix_room.send_queue().set_enabled(true);
                    });
                }
            })
            .abort_handle();

        *self
            .send_queue_handle
            .lock()
            .expect("mutex is not poisoned") = Some(handle);
    }

    /// Update the visibility of the history.
    fn update_history_visibility(&self) {
        let visibility = self.matrix_room.history_visibility_or_default().into();
        self.history_visibility.set_if_not_eq(visibility);
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

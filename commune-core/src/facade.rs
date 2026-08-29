//! The `UniFFI` facade — what the Kotlin side sees.
//!
//! **This is facade v1 and provisional.** It exists to prove the reactive
//! bridge end to end (core observables → foreign listener) and to give the
//! walking skeleton a real sidebar; the real API pass comes with its own
//! chunk. Two shortcuts are deliberate: listeners receive full snapshots
//! rather than diffs, and only the first ready session is exposed.
//!
//! Every future handed across the FFI is a `JoinHandle` onto the core's
//! own runtime, so uniffi's executor never has to be a tokio one.

use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::{
    RUNTIME, config,
    session::{Room, RoomCategory, RoomDisplayName, RoomHighlight, Session},
    session_list::SessionList,
};

/// What the embedder tells the core about itself, over the FFI.
#[derive(uniffi::Record)]
pub struct FfiCoreConfig {
    /// The application id.
    pub app_id: String,
    /// The build profile name.
    pub profile: String,
    /// The directory persistent data lives under.
    pub data_dir: String,
    /// The directory cached data lives under.
    pub cache_dir: String,
}

/// Provide the core with the embedder's configuration.
///
/// Must be called once, before anything else. Settings go to the JSON-file
/// store under the data directory.
#[uniffi::export]
pub fn init_core(ffi_config: FfiCoreConfig) {
    // Rust logs would otherwise vanish: an Android process has no stdout.
    // The same subscriber and panic hook the application installs, so
    // `adb logcat -s Commune` reads the core too.
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::prelude::*;

        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let env_filter = tracing_subscriber::EnvFilter::new("commune_core=debug,warn");
            tracing_subscriber::registry()
                .with(paranoid_android::layer("Commune").with_filter(env_filter))
                .init();

            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                tracing::error!("PANIC: {info}");
                previous(info);
            }));
        });
    }

    config::init(config::CoreConfig {
        app_id: ffi_config.app_id,
        profile: ffi_config.profile,
        data_dir: ffi_config.data_dir.into(),
        cache_dir: ffi_config.cache_dir.into(),
        settings_store: None,
    });
}

/// An error handed across the FFI.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    /// The operation failed; the message is displayable.
    #[error("{msg}")]
    Failed {
        /// The displayable message.
        msg: String,
    },
}

impl From<String> for CoreError {
    fn from(msg: String) -> Self {
        Self::Failed { msg }
    }
}

/// A room's display name, semantically: the Empty variants are the UI's
/// sentences to make.
#[derive(uniffi::Enum)]
pub enum FfiRoomDisplayName {
    /// The room has a computed name.
    Named {
        /// The name.
        name: String,
    },
    /// The room is empty but had another user before.
    EmptyWas {
        /// The user that was in the room.
        user: String,
    },
    /// The room is empty and never had another user.
    Empty,
    /// The name is not known yet.
    Unknown,
}

impl From<RoomDisplayName> for FfiRoomDisplayName {
    fn from(value: RoomDisplayName) -> Self {
        match value {
            RoomDisplayName::Named(name) => Self::Named { name },
            RoomDisplayName::EmptyWas(user) => Self::EmptyWas { user },
            RoomDisplayName::Empty => Self::Empty,
            RoomDisplayName::Unknown => Self::Unknown,
        }
    }
}

/// The category of a room.
#[derive(uniffi::Enum)]
pub enum FfiRoomCategory {
    Knocked,
    Invited,
    ServerNotice,
    Favorite,
    Normal,
    LowPriority,
    Left,
    Outdated,
    Space,
    Ignored,
}

impl From<RoomCategory> for FfiRoomCategory {
    fn from(value: RoomCategory) -> Self {
        match value {
            RoomCategory::Knocked => Self::Knocked,
            RoomCategory::Invited => Self::Invited,
            RoomCategory::ServerNotice => Self::ServerNotice,
            RoomCategory::Favorite => Self::Favorite,
            RoomCategory::Normal => Self::Normal,
            RoomCategory::LowPriority => Self::LowPriority,
            RoomCategory::Left => Self::Left,
            RoomCategory::Outdated => Self::Outdated,
            RoomCategory::Space => Self::Space,
            RoomCategory::Ignored => Self::Ignored,
        }
    }
}

/// The highlight state of a room in the sidebar.
#[derive(uniffi::Enum)]
pub enum FfiRoomHighlight {
    None,
    Bold,
    Highlight,
}

impl From<RoomHighlight> for FfiRoomHighlight {
    fn from(value: RoomHighlight) -> Self {
        match value {
            RoomHighlight::None => Self::None,
            RoomHighlight::Bold => Self::Bold,
            RoomHighlight::Highlight => Self::Highlight,
        }
    }
}

/// A room, as the sidebar needs it.
#[derive(uniffi::Record)]
pub struct FfiRoom {
    /// The ID of the room.
    pub room_id: String,
    /// The display name of the room.
    pub display_name: FfiRoomDisplayName,
    /// The category of the room.
    pub category: FfiRoomCategory,
    /// The highlight state of the room.
    pub highlight: FfiRoomHighlight,
    /// The number of unread notifications.
    pub notification_count: u64,
    /// Whether the room is a direct chat.
    pub is_direct: bool,
    /// Whether all messages are read.
    pub is_read: bool,
    /// The timestamp of the latest activity, in milliseconds since the
    /// Unix epoch.
    pub latest_activity: u64,
    /// The avatar of the room, as an `mxc:` URI.
    pub avatar_url: Option<String>,
    /// The number of joined members.
    pub joined_members_count: u64,
    /// The topic of the room, if any.
    pub topic: Option<String>,
    /// Who sent the latest message, when one is known and readable.
    pub latest_event_sender: Option<String>,
    /// The body of the latest message, when one is known and readable.
    pub latest_event_body: Option<String>,
}

impl From<&Room> for FfiRoom {
    fn from(room: &Room) -> Self {
        Self {
            room_id: room.room_id().to_string(),
            display_name: room.display_name().into(),
            category: room.category().into(),
            highlight: room.highlight().into(),
            notification_count: room.notification_count(),
            is_direct: room.is_direct(),
            is_read: room.is_read(),
            latest_activity: room.latest_activity(),
            avatar_url: room.avatar_url().map(|uri| uri.to_string()),
            joined_members_count: room.joined_members_count(),
            topic: room.topic(),
            latest_event_sender: latest_preview(room).map(|(sender, _)| sender),
            latest_event_body: latest_preview(room).map(|(_, body)| body),
        }
    }
}

/// The latest message of the room as a (sender, body) pair, when the
/// stored latest event is a readable message.
fn latest_preview(room: &Room) -> Option<(String, String)> {
    let matrix_sdk::latest_events::LatestEventValue::Remote(event) =
        room.matrix_room().latest_event()
    else {
        return None;
    };
    let message = crate::matrix::original_message_event_from_raw(event.raw())?;
    Some((
        message.sender.to_string(),
        message.content.msgtype.body().to_owned(),
    ))
}

/// Something on the foreign side that wants to know when the room list
/// changes.
#[uniffi::export(with_foreign)]
pub trait RoomListListener: Send + Sync {
    /// The room list changed; here is all of it.
    fn on_update(&self, rooms: Vec<FfiRoom>);
}

/// The membership state of a room member.
#[derive(uniffi::Enum)]
pub enum FfiMembership {
    /// The user left the room, or was never in the room.
    Leave,
    /// The user is currently in the room.
    Join,
    /// The user was invited to the room.
    Invite,
    /// The user was banned from the room.
    Ban,
    /// The user knocked on the room.
    Knock,
    /// The user is in an unsupported membership state.
    Unsupported,
}

impl From<crate::session::Membership> for FfiMembership {
    fn from(value: crate::session::Membership) -> Self {
        use crate::session::Membership;
        match value {
            Membership::Leave => Self::Leave,
            Membership::Join => Self::Join,
            Membership::Invite => Self::Invite,
            Membership::Ban => Self::Ban,
            Membership::Knock => Self::Knock,
            Membership::Unsupported => Self::Unsupported,
        }
    }
}

/// The role of a room member, derived from their power level.
#[derive(uniffi::Enum)]
pub enum FfiMemberRole {
    /// A room creator, with infinite power level.
    Creator,
    /// An administrator.
    Administrator,
    /// A moderator.
    Moderator,
    /// A member with the room's default power level.
    Default,
    /// A member without enough power to send messages.
    Muted,
    /// A member with a power level that matches no other role.
    Custom,
}

impl From<crate::session::MemberRole> for FfiMemberRole {
    fn from(value: crate::session::MemberRole) -> Self {
        use crate::session::MemberRole;
        match value {
            MemberRole::Creator => Self::Creator,
            MemberRole::Administrator => Self::Administrator,
            MemberRole::Moderator => Self::Moderator,
            MemberRole::Default => Self::Default,
            MemberRole::Muted => Self::Muted,
            MemberRole::Custom => Self::Custom,
        }
    }
}

/// A member of a room.
#[derive(uniffi::Record)]
pub struct FfiMember {
    /// The Matrix ID of the member.
    pub user_id: String,
    /// The name the member displays as.
    pub display_name: String,
    /// Whether the display name is shared with another member.
    pub is_name_ambiguous: bool,
    /// The avatar of the member, if any.
    pub avatar_url: Option<String>,
    /// The power level of the member; `i64::MAX` stands for infinite.
    pub power_level: i64,
    /// The role of the member.
    pub role: FfiMemberRole,
    /// The membership state of the member.
    pub membership: FfiMembership,
}

impl From<&crate::session::Member> for FfiMember {
    fn from(member: &crate::session::Member) -> Self {
        use ruma::events::room::power_levels::UserPowerLevel;

        Self {
            user_id: member.user_id.to_string(),
            display_name: member.display_name_or_localpart(),
            is_name_ambiguous: member.is_name_ambiguous,
            avatar_url: member.avatar_url.as_ref().map(ToString::to_string),
            power_level: if let UserPowerLevel::Int(level) = member.power_level {
                level.into()
            } else {
                i64::MAX
            },
            role: member.role.into(),
            membership: member.membership.into(),
        }
    }
}

/// Something on the foreign side that wants to know when a room's member
/// list changes.
#[uniffi::export(with_foreign)]
pub trait MemberListListener: Send + Sync {
    /// The member list changed; here is all of it.
    fn on_update(&self, members: Vec<FfiMember>);
}

/// Where a room can be moved: the sidebar's category actions.
#[derive(uniffi::Enum)]
pub enum FfiTargetRoomCategory {
    /// Join or move the room into the favorite category.
    Favorite,
    /// Join or move the room into the normal category.
    Normal,
    /// Join or move the room into the low priority category.
    LowPriority,
    /// Leave the room.
    Left,
}

impl From<FfiTargetRoomCategory> for crate::session::TargetRoomCategory {
    fn from(value: FfiTargetRoomCategory) -> Self {
        match value {
            FfiTargetRoomCategory::Favorite => Self::Favorite,
            FfiTargetRoomCategory::Normal => Self::Normal,
            FfiTargetRoomCategory::LowPriority => Self::LowPriority,
            FfiTargetRoomCategory::Left => Self::Left,
        }
    }
}

/// The reply context of an event: what it replies to.
#[derive(uniffi::Record)]
pub struct FfiInReplyTo {
    /// The ID of the replied-to event.
    pub event_id: String,
    /// The sender of the replied-to event, when its details are loaded.
    pub sender: Option<String>,
    /// The body of the replied-to event, when its details are loaded.
    pub body: Option<String>,
}

/// What a state event changed — the ones the timeline words, with the
/// strings the sentence needs.
#[derive(uniffi::Enum)]
pub enum FfiStateChange {
    /// The room name changed.
    Name {
        /// The new name; unset when it was removed.
        name: Option<String>,
    },
    /// The room topic changed.
    Topic {
        /// The new topic; unset when it was removed.
        topic: Option<String>,
    },
    /// The room avatar changed.
    Avatar,
    /// The room was created.
    Create,
    /// Encryption was enabled.
    Encryption,
    /// The join rules changed.
    JoinRules,
    /// The history visibility changed.
    HistoryVisibility,
    /// The canonical alias changed.
    CanonicalAlias,
    /// The pinned events changed.
    PinnedEvents,
    /// Something the timeline has no words for yet.
    Other,
}

/// One message found by an in-room search.
#[derive(uniffi::Record)]
pub struct FfiSearchResult {
    /// The ID of the found event.
    pub event_id: String,
    /// The user that sent it.
    pub sender: String,
    /// The body of the message.
    pub body: String,
    /// The timestamp, in milliseconds since the Unix epoch.
    pub timestamp: u64,
}

/// One room inside a space, as its hierarchy reports it.
#[derive(uniffi::Record)]
pub struct FfiSpaceChild {
    /// The ID of the room.
    pub room_id: String,
    /// The name of the room, if it has one.
    pub name: Option<String>,
    /// The topic of the room, if it has one.
    pub topic: Option<String>,
    /// How many members have joined it.
    pub num_joined_members: u64,
    /// Whether our own user has joined it.
    pub is_joined: bool,
    /// Whether this child is itself a space.
    pub is_space: bool,
}

/// One emoji of the short auth string.
#[derive(uniffi::Record)]
pub struct FfiSasEmoji {
    /// The emoji symbol.
    pub symbol: String,
    /// The word naming it.
    pub description: String,
}

/// Something on the foreign side that wants to follow device
/// verifications.
#[uniffi::export(with_foreign)]
pub trait VerificationListener: Send + Sync {
    /// Another session asked to verify with this one.
    fn on_request(&self, flow_id: String, user_id: String);
    /// The short auth string is ready to compare.
    fn on_emojis(&self, flow_id: String, emojis: Vec<FfiSasEmoji>);
    /// The verification finished on both sides.
    fn on_done(&self, flow_id: String);
    /// The verification was cancelled.
    fn on_cancelled(&self, flow_id: String, reason: String);
}

/// Where account recovery stands.
#[derive(uniffi::Enum)]
pub enum FfiRecoveryState {
    /// The state is not known yet.
    Unknown,
    /// Recovery is set up and every secret is here.
    Enabled,
    /// Recovery is not set up.
    Disabled,
    /// Recovery is set up elsewhere and this session misses secrets —
    /// entering the recovery key completes it.
    Incomplete,
}

/// What a media event carries.
#[derive(uniffi::Enum)]
pub enum FfiMediaKind {
    /// An image.
    Image,
    /// A video.
    Video,
    /// An audio message, voice or otherwise.
    Audio,
    /// Any other file.
    File,
}

/// One reaction key on an event, aggregated over its senders.
#[derive(uniffi::Record)]
pub struct FfiReaction {
    /// The reaction key — usually an emoji.
    pub key: String,
    /// How many users sent this reaction.
    pub count: u64,
    /// Whether our own user is among them.
    pub is_own: bool,
}

/// A timeline item, as the message list needs it.
///
/// The event variant is big and the virtual variants are tiny; uniffi
/// lowers enums by value either way, so boxing would only move the cost.
#[allow(clippy::large_enum_variant)]
#[derive(uniffi::Enum)]
pub enum FfiTimelineItem {
    /// A message-like event.
    Event {
        /// The unique ID of the item within its timeline.
        unique_id: String,
        /// The globally unique event ID, once the server assigned one.
        event_id: Option<String>,
        /// The number of replies in the thread rooted here, if any.
        thread_replies: u64,
        /// The reactions on the event.
        reactions: Vec<FfiReaction>,
        /// The reply context of the event, if it is a reply.
        in_reply_to: Option<FfiInReplyTo>,
        /// Whether the event was edited.
        is_edited: bool,
        /// The users whose read receipts sit on this event, ourselves
        /// excluded by the SDK's own accounting.
        receipts: Vec<String>,
        /// The user that sent the event.
        sender: String,
        /// The display name of the sender, if it is known.
        sender_display_name: Option<String>,
        /// The timestamp of the event, in milliseconds since the Unix
        /// epoch.
        timestamp: u64,
        /// Whether our own user sent the event.
        is_own: bool,
        /// What kind of event this is.
        kind: FfiEventKind,
        /// The text of the event, as far as it has one.
        body: String,
        /// How far a locally sent event got, `None` for remote echoes.
        send_state: Option<FfiSendState>,
    },
    /// A divider between two days.
    DateDivider {
        /// The timestamp of the day, in milliseconds since the Unix epoch.
        timestamp: u64,
    },
    /// The position of our own user's read marker.
    ReadMarker,
    /// The start of the timeline.
    TimelineStart,
}

/// Decode a blurhash into raw RGBA bytes at the given size.
///
/// Rendering at a couple dozen pixels a side and letting the UI scale it
/// up is the intended use — a blurhash holds no more detail than that.
#[uniffi::export]
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "UniFFI hands arguments over by value"
)]
pub fn decode_blurhash(blurhash: String, width: u32, height: u32) -> Option<Vec<u8>> {
    blurhash::decode(&blurhash, width, height, 1.0).ok()
}

/// How far a locally sent event has got.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum FfiSendState {
    /// Waiting in the send queue.
    Sending,
    /// The send failed, and re-enabling the queue can retry it.
    RecoverableError,
    /// The send failed for good (for example, too large).
    PermanentError,
    /// The server acknowledged the event.
    Sent,
}

/// What kind of event a timeline item is.
#[derive(uniffi::Enum)]
pub enum FfiEventKind {
    /// A text-like message (`m.text`, `m.notice`, `m.emote`).
    Text,
    /// A media message; the body is the caption or filename.
    Media {
        /// Whether the media is an image the timeline can show inline
        /// (fetch it with `get_timeline_media`).
        kind: FfiMediaKind,
        /// The blurhash of the media, to present until it arrives
        /// (decode it with `decode_blurhash`).
        blurhash: Option<String>,
    },
    /// A shared location; the body describes it.
    Location {
        /// The `geo:` URI of the spot.
        geo_uri: String,
    },
    /// A message that could not be decrypted.
    UnableToDecrypt,
    /// A redacted message.
    Redacted,
    /// A membership change; the UI words the sentence.
    Membership {
        /// The user whose membership changed.
        user: String,
        /// What happened.
        change: FfiMembershipChange,
    },
    /// A member changed their profile.
    ProfileChange {
        /// The user whose profile changed.
        user: String,
    },
    /// Another state event.
    OtherState {
        /// What changed.
        change: FfiStateChange,
    },
    /// Something not handled yet.
    Unsupported,
}

/// What happened to a user's membership — semantic, the UI's sentence to
/// make.
#[derive(uniffi::Enum)]
pub enum FfiMembershipChange {
    Joined,
    Left,
    Banned,
    Unbanned,
    Kicked,
    Invited,
    KickedAndBanned,
    InvitationAccepted,
    InvitationRejected,
    InvitationRevoked,
    Knocked,
    KnockAccepted,
    KnockRetracted,
    KnockDenied,
    /// The change could not be computed (first event, redaction, or a kind
    /// this version does not know).
    Unknown,
}

impl From<Option<matrix_sdk_ui::timeline::MembershipChange>> for FfiMembershipChange {
    fn from(value: Option<matrix_sdk_ui::timeline::MembershipChange>) -> Self {
        use matrix_sdk_ui::timeline::MembershipChange;

        match value {
            Some(MembershipChange::Joined) => Self::Joined,
            Some(MembershipChange::Left) => Self::Left,
            Some(MembershipChange::Banned) => Self::Banned,
            Some(MembershipChange::Unbanned) => Self::Unbanned,
            Some(MembershipChange::Kicked) => Self::Kicked,
            Some(MembershipChange::Invited) => Self::Invited,
            Some(MembershipChange::KickedAndBanned) => Self::KickedAndBanned,
            Some(MembershipChange::InvitationAccepted) => Self::InvitationAccepted,
            Some(MembershipChange::InvitationRejected) => Self::InvitationRejected,
            Some(MembershipChange::InvitationRevoked) => Self::InvitationRevoked,
            Some(MembershipChange::Knocked) => Self::Knocked,
            Some(MembershipChange::KnockAccepted) => Self::KnockAccepted,
            Some(MembershipChange::KnockRetracted) => Self::KnockRetracted,
            Some(MembershipChange::KnockDenied) => Self::KnockDenied,
            _ => Self::Unknown,
        }
    }
}

/// Something on the foreign side that wants to know when a room's timeline
/// changes.
#[uniffi::export(with_foreign)]
pub trait TimelineListener: Send + Sync {
    /// The timeline changed; here is all of it.
    fn on_update(&self, items: Vec<FfiTimelineItem>);
}

/// Something on the foreign side that wants to know who is typing in a
/// room.
#[uniffi::export(with_foreign)]
pub trait TypingListener: Send + Sync {
    /// The set of typing users changed; our own user is never included.
    fn on_update(&self, user_ids: Vec<String>);
}

/// The toggleable per-session settings, as the settings screen needs
/// them.
#[derive(uniffi::Record)]
pub struct FfiSessionSettings {
    /// Whether notifications are enabled for this session.
    pub notifications_enabled: bool,
    /// Whether read receipts are public.
    pub public_read_receipts_enabled: bool,
    /// Whether typing notifications are sent.
    pub typing_enabled: bool,
}

/// The core, as one object the foreign side holds.
#[derive(uniffi::Object)]
pub struct CoreApp {
    /// The list of logged-in sessions.
    session_list: SessionList,
    /// The task pushing room updates to the foreign listener.
    listener_handle: Mutex<Option<tokio::task::AbortHandle>>,
    /// The task pushing timeline updates to the foreign listener.
    timeline_listener_handle: Mutex<Option<tokio::task::AbortHandle>>,
    /// The task pushing typing updates to the foreign listener.
    typing_listener_handle: Mutex<Option<tokio::task::AbortHandle>>,
    /// The task pushing thread-timeline updates to the foreign listener.
    thread_listener_handle: Mutex<Option<tokio::task::AbortHandle>>,
    /// The task feeding the pinned-events listener.
    pinned_listener_handle: Mutex<Option<tokio::task::AbortHandle>>,
    /// The verification listener and the flows in progress.
    verification: Arc<VerificationFlows>,
    /// The task feeding the member-list listener.
    member_list_listener_handle: Mutex<Option<tokio::task::AbortHandle>>,
    /// The client a login flow built at discovery, kept for the flow's
    /// next step (SSO, OAuth, registration, password reset).
    pending_login: Mutex<Option<matrix_sdk::Client>>,
}

#[uniffi::export]
impl CoreApp {
    /// Create the core. [`init_core()`] must have been called.
    #[uniffi::constructor]
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            session_list: SessionList::new(),
            listener_handle: Mutex::new(None),
            timeline_listener_handle: Mutex::new(None),
            typing_listener_handle: Mutex::new(None),
            thread_listener_handle: Mutex::new(None),
            pinned_listener_handle: Mutex::new(None),
            verification: Arc::new(VerificationFlows::default()),
            member_list_listener_handle: Mutex::new(None),
            pending_login: Mutex::new(None),
        })
    }

    /// Restore the sessions stored on this device.
    pub async fn restore_sessions(&self) {
        let list = self.session_list.clone();
        RUNTIME
            .spawn(async move { list.restore_sessions().await })
            .await
            .expect("task was not aborted");
    }

    /// Whether a session is logged in and running.
    ///
    /// Restoration is asynchronous: after [`Self::restore_sessions()`] this
    /// turns true once the stored session has finished coming up.
    #[must_use]
    pub fn has_ready_session(&self) -> bool {
        self.first_ready_session().is_some()
    }

    /// Whether there are sessions on this device, in any state.
    #[must_use]
    pub fn has_sessions(&self) -> bool {
        !self.session_list.is_empty()
    }

    /// The Matrix user ID of the first ready session, if any.
    #[must_use]
    pub fn session_user_id(&self) -> Option<String> {
        self.first_ready_session()
            .map(|session| session.user_id().to_string())
    }

    /// The display name of the first ready session's user, if it is known.
    #[must_use]
    pub fn session_display_name(&self) -> Option<String> {
        self.first_ready_session()
            .and_then(|session| session.profile().display_name)
    }

    /// Log in with a password on the given homeserver.
    pub async fn login_with_password(
        &self,
        homeserver: String,
        username: String,
        password: String,
    ) -> Result<(), CoreError> {
        let homeserver: url::Url = homeserver.parse().map_err(|_| CoreError::Failed {
            msg: "Invalid homeserver URL".to_owned(),
        })?;

        let list = self.session_list.clone();
        RUNTIME
            .spawn(async move {
                list.login_with_password(homeserver, username, password)
                    .await
            })
            .await
            .expect("task was not aborted")?;

        Ok(())
    }

    /// What the given homeserver offers for logging in, per the
    /// application's discovery: the OAuth 2.0 API when its metadata
    /// resolves, the Matrix native flows otherwise. The client built
    /// here is kept for the flow's next step.
    pub async fn discover_login(&self, homeserver: String) -> Result<FfiLoginMethods, CoreError> {
        use ruma::api::client::session::get_login_types::v3::LoginType;

        let (client, methods) = RUNTIME
            .spawn(async move {
                let client = matrix_sdk::Client::builder()
                    .request_config(matrix_sdk::config::RequestConfig::new().retry_limit(2))
                    .http_client(crate::tls::matrix_client())
                    .server_name_or_homeserver_url(homeserver)
                    .build()
                    .await
                    .map_err(|build_error| CoreError::Failed {
                        msg: format!("Could not reach the homeserver: {build_error}"),
                    })?;

                let supports_oauth = client.oauth().server_metadata().await.is_ok();

                let (supports_password, supports_sso) = if supports_oauth {
                    (false, false)
                } else {
                    let flows = client
                        .matrix_auth()
                        .get_login_types()
                        .await
                        .map_err(|types_error| CoreError::Failed {
                            msg: format!("Could not ask how to log in: {types_error}"),
                        })?
                        .flows;
                    (
                        flows
                            .iter()
                            .any(|login_type| matches!(login_type, LoginType::Password(_))),
                        flows
                            .iter()
                            .any(|login_type| matches!(login_type, LoginType::Sso(_))),
                    )
                };

                let methods = FfiLoginMethods {
                    homeserver_url: client.homeserver().to_string(),
                    supports_password,
                    supports_sso,
                    supports_oauth,
                };
                Ok::<_, CoreError>((client, methods))
            })
            .await
            .expect("task was not aborted")?;

        *self.pending_login.lock().expect("mutex is not poisoned") = Some(client);
        Ok(methods)
    }

    /// The Matrix SSO URL to open in the browser; the redirect carries
    /// the login token back on the application's fixed Android scheme.
    pub async fn sso_login_url(&self) -> Result<String, CoreError> {
        let client = self.pending_login_client()?;

        RUNTIME
            .spawn(async move {
                client
                    .matrix_auth()
                    .get_sso_login_url(ANDROID_REDIRECT_URI, None)
                    .await
                    .map_err(|url_error| CoreError::Failed {
                        msg: format!("Could not build the SSO URL: {url_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Finish a Matrix SSO login with the token the redirect carried.
    pub async fn finish_sso_login(&self, login_token: String) -> Result<(), CoreError> {
        let client = self.pending_login_client()?;
        let list = self.session_list.clone();

        RUNTIME
            .spawn(async move {
                client
                    .matrix_auth()
                    .login_token(&login_token)
                    .initial_device_display_name("Commune")
                    .send()
                    .await
                    .map_err(|login_error| CoreError::Failed {
                        msg: format!("Could not log in: {login_error}"),
                    })?;
                list.adopt_logged_in_client(client)
                    .await
                    .map(|_| ())
                    .map_err(|adopt_error| CoreError::Failed { msg: adopt_error })
            })
            .await
            .expect("task was not aborted")
    }

    /// The OAuth 2.0 authorization URL to open in the browser, with the
    /// application's exact client registration.
    pub async fn oauth_login_url(&self) -> Result<String, CoreError> {
        let client = self.pending_login_client()?;

        RUNTIME
            .spawn(async move {
                let redirect =
                    url::Url::parse(ANDROID_REDIRECT_URI).expect("redirect URI is valid");
                let oauth = client.oauth();
                let data = oauth
                    .login(redirect, None, Some(oauth_client_registration_data()), None)
                    .build()
                    .await
                    .map_err(|url_error| CoreError::Failed {
                        msg: format!("Could not set up login: {url_error}"),
                    })?;
                Ok(data.url.to_string())
            })
            .await
            .expect("task was not aborted")
    }

    /// Finish an OAuth 2.0 login with the query string the redirect
    /// carried.
    pub async fn finish_oauth_login(&self, redirect_query: String) -> Result<(), CoreError> {
        use matrix_sdk::utils::UrlOrQuery;

        let client = self.pending_login_client()?;
        let list = self.session_list.clone();

        RUNTIME
            .spawn(async move {
                client
                    .oauth()
                    .finish_login(UrlOrQuery::Query(redirect_query))
                    .await
                    .map_err(|login_error| CoreError::Failed {
                        msg: format!("Could not log in: {login_error}"),
                    })?;
                list.adopt_logged_in_client(client)
                    .await
                    .map(|_| ())
                    .map_err(|adopt_error| CoreError::Failed { msg: adopt_error })
            })
            .await
            .expect("task was not aborted")
    }

    /// Create an account on the discovered homeserver, walking the
    /// stages a headless client can answer (dummy, and terms — creating
    /// the account is accepting them). Anything more wants a browser.
    pub async fn register_user(&self, username: String, password: String) -> Result<(), CoreError> {
        use ruma::api::client::{
            account::register,
            uiaa::{AuthData, Dummy},
        };

        let client = self.pending_login_client()?;
        let list = self.session_list.clone();

        RUNTIME
            .spawn(async move {
                let build_request = |auth: Option<AuthData>| {
                    let mut request = register::v3::Request::new();
                    request.username = Some(username.clone());
                    request.password = Some(password.clone());
                    request.initial_device_display_name = Some("Commune".to_owned());
                    request.auth = auth;
                    request
                };

                let mut auth = None;
                for _attempt in 0..4 {
                    match client.matrix_auth().register(build_request(auth.take())).await {
                        Ok(_) => {
                            return list
                                .adopt_logged_in_client(client)
                                .await
                                .map(|_| ())
                                .map_err(|adopt_error| CoreError::Failed { msg: adopt_error });
                        }
                        Err(register_error) => {
                            let Some(uiaa) = register_error.as_uiaa_response() else {
                                return Err(CoreError::Failed {
                                    msg: format!("Could not create account: {register_error}"),
                                });
                            };
                            let next_stage = uiaa
                                .flows
                                .iter()
                                .filter_map(|flow| {
                                    flow.stages
                                        .iter()
                                        .find(|stage| !uiaa.completed.contains(stage))
                                })
                                .find(|stage| {
                                    stage.as_str() == "m.login.dummy"
                                        || stage.as_str() == "m.login.terms"
                                });
                            auth = match next_stage.map(|stage| stage.as_str()) {
                                Some("m.login.dummy") => {
                                    let mut dummy = Dummy::new();
                                    dummy.session = uiaa.session.clone();
                                    Some(AuthData::Dummy(dummy))
                                }
                                Some("m.login.terms") => AuthData::new(
                                    "m.login.terms",
                                    uiaa.session.clone(),
                                    serde_json::Map::new(),
                                )
                                .ok(),
                                _ => {
                                    return Err(CoreError::Failed {
                                        msg: "This homeserver asks for steps this app cannot answer yet"
                                            .to_owned(),
                                    });
                                }
                            };
                        }
                    }
                }
                Err(CoreError::Failed {
                    msg: "Could not create account".to_owned(),
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Ask the homeserver to email a password-reset link.
    pub async fn request_password_reset(&self, email: String) -> Result<FfiResetHandle, CoreError> {
        use ruma::api::client::account::request_password_change_token_via_email;

        let client = self.pending_login_client()?;

        RUNTIME
            .spawn(async move {
                let client_secret = ruma::ClientSecret::new();
                let request = request_password_change_token_via_email::v3::Request::new(
                    client_secret.clone(),
                    email,
                    1u32.into(),
                );
                let response =
                    client
                        .send(request)
                        .await
                        .map_err(|send_error| CoreError::Failed {
                            msg: format!("Could not send the email: {send_error}"),
                        })?;
                Ok(FfiResetHandle {
                    sid: response.sid.to_string(),
                    client_secret: client_secret.to_string(),
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Set the new password once the emailed link was opened, signing
    /// every other session out, as the application does.
    pub async fn reset_password(
        &self,
        new_password: String,
        handle: FfiResetHandle,
    ) -> Result<(), CoreError> {
        use ruma::api::client::{
            account::change_password,
            uiaa::{AuthData, ThirdpartyIdCredentials},
        };

        let client = self.pending_login_client()?;

        RUNTIME
            .spawn(async move {
                let sid = ruma::SessionId::parse(handle.sid).map_err(|_| CoreError::Failed {
                    msg: "Invalid reset session".to_owned(),
                })?;
                let client_secret =
                    ruma::ClientSecret::parse(handle.client_secret).map_err(|_| {
                        CoreError::Failed {
                            msg: "Invalid reset secret".to_owned(),
                        }
                    })?;
                let credentials = ThirdpartyIdCredentials::new(sid, client_secret);
                let credentials =
                    serde_json::to_value(&credentials).map_err(|_| CoreError::Failed {
                        msg: "Could not build the reset".to_owned(),
                    })?;
                let mut data = serde_json::Map::new();
                data.insert("threepid_creds".to_owned(), credentials);
                let auth = AuthData::new("m.login.email.identity", None, data).map_err(|_| {
                    CoreError::Failed {
                        msg: "Could not build the reset".to_owned(),
                    }
                })?;

                let mut request = change_password::v3::Request::new(new_password);
                // The account has been out of the owner's hands for as
                // long as the password was unknown; every other session
                // goes.
                request.logout_devices = true;
                request.auth = Some(auth);

                client
                    .send(request)
                    .await
                    .map(|_| ())
                    .map_err(|send_error| CoreError::Failed {
                        msg: if send_error.as_uiaa_response().is_some() {
                            "Open the link in the email first, then try again".to_owned()
                        } else {
                            format!("Could not reset the password: {send_error}")
                        },
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// The rooms of the first ready session, as of now.
    #[must_use]
    pub fn rooms(&self) -> Vec<FfiRoom> {
        self.first_ready_session().map_or_else(Vec::new, |session| {
            session
                .room_list()
                .snapshot()
                .iter()
                .map(FfiRoom::from)
                .collect()
        })
    }

    /// Give the room list of the first ready session to the given listener,
    /// now and on every change.
    ///
    /// Replaces any previous listener.
    pub fn set_room_list_listener(&self, listener: Arc<dyn RoomListListener>) {
        let list = self.session_list.clone();

        let handle = RUNTIME
            .spawn(async move {
                let session = wait_for_ready_session(&list).await;
                push_room_updates(&session, listener).await;
            })
            .abort_handle();

        if let Some(previous) = self
            .listener_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// Give the room's pinned events to the given listener, now and on
    /// every change.
    ///
    /// Replaces any previous pinned listener.
    pub fn set_pinned_listener(&self, room_id: String, listener: Arc<dyn TimelineListener>) {
        let session = self.first_ready_session();

        let handle = RUNTIME
            .spawn(async move {
                let Some(session) = session else { return };
                let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
                    return;
                };
                let Some(room) = session.room_list().get(&room_id) else {
                    return;
                };

                let timeline = room.pinned_timeline();
                let Some((items, mut stream)) = timeline.subscribe_items().await else {
                    return;
                };
                let own_user_id = session.user_id().clone();
                listener.on_update(
                    items
                        .iter()
                        .map(|item| ffi_timeline_item(item, Some(&own_user_id)))
                        .collect(),
                );

                let mut items = items;
                while let Some(diffs) = stream.next().await {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                    listener.on_update(
                        items
                            .iter()
                            .map(|item| ffi_timeline_item(item, Some(&own_user_id)))
                            .collect(),
                    );
                }
            })
            .abort_handle();

        if let Some(previous) = self
            .pinned_listener_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// Stop feeding the pinned-events listener.
    pub fn clear_pinned_listener(&self) {
        if let Some(previous) = self
            .pinned_listener_handle
            .lock()
            .expect("mutex is not poisoned")
            .take()
        {
            previous.abort();
        }
    }

    /// Give the member list of the given room to the given listener, now
    /// and on every change.
    ///
    /// Replaces any previous member-list listener; v1 watches one room at
    /// a time, which is what one screen shows.
    pub fn set_member_list_listener(&self, room_id: String, listener: Arc<dyn MemberListListener>) {
        let session = self.first_ready_session();

        let handle = RUNTIME
            .spawn(async move {
                let Some(session) = session else { return };
                let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
                    return;
                };
                let Some(room) = session.room_list().get(&room_id) else {
                    return;
                };

                let member_list = room.member_list();
                let (members, stream) = member_list.subscribe();

                listener.on_update(members.iter().map(FfiMember::from).collect());

                let mut members = members;
                let mut stream = std::pin::pin!(stream);
                while let Some(diffs) = stream.next().await {
                    for diff in diffs {
                        diff.apply(&mut members);
                    }
                    listener.on_update(members.iter().map(FfiMember::from).collect());
                }
            })
            .abort_handle();

        if let Some(previous) = self
            .member_list_listener_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// Stop feeding the member-list listener.
    pub fn clear_member_list_listener(&self) {
        if let Some(previous) = self
            .member_list_listener_handle
            .lock()
            .expect("mutex is not poisoned")
            .take()
        {
            previous.abort();
        }
    }

    /// Fetch the avatar at the given MXC URI into a file, returning its
    /// path.
    pub async fn get_avatar(&self, mxc_uri: String, size: u32) -> Option<String> {
        let session = self.first_ready_session()?;

        RUNTIME
            .spawn(async move {
                let avatar_url: ruma::OwnedMxcUri = mxc_uri.into();

                crate::matrix::media::get_avatar_file(&session.client(), &avatar_url, size)
                    .await
                    .map(|path| path.to_string_lossy().into_owned())
            })
            .await
            .expect("task was not aborted")
    }

    /// Give the timeline of the given room to the given listener, now and
    /// on every change.
    ///
    /// Replaces any previous timeline listener; v1 watches one room at a
    /// time, which is what one screen shows.
    pub fn set_timeline_listener(&self, room_id: String, listener: Arc<dyn TimelineListener>) {
        let session = self.first_ready_session();

        let handle = RUNTIME
            .spawn(async move {
                let Some(session) = session else { return };
                let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
                    return;
                };
                let Some(room) = session.room_list().get(&room_id) else {
                    return;
                };

                let timeline = room.live_timeline();
                let Some((items, mut stream)) = timeline.subscribe_items().await else {
                    return;
                };
                let own_user_id = session.user_id().clone();

                listener.on_update(
                    items
                        .iter()
                        .map(|item| ffi_timeline_item(item, Some(&own_user_id)))
                        .collect(),
                );

                let mut items = items;
                while let Some(diffs) = stream.next().await {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                    listener.on_update(
                        items
                            .iter()
                            .map(|item| ffi_timeline_item(item, Some(&own_user_id)))
                            .collect(),
                    );
                }
            })
            .abort_handle();

        if let Some(previous) = self
            .timeline_listener_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// The current session's settings.
    #[must_use]
    pub fn session_settings(&self) -> Option<FfiSessionSettings> {
        let session = self.first_ready_session()?;
        let settings = session.settings();

        Some(FfiSessionSettings {
            notifications_enabled: settings.notifications_enabled(),
            public_read_receipts_enabled: settings.public_read_receipts_enabled(),
            typing_enabled: settings.typing_enabled(),
        })
    }

    /// Set whether notifications are enabled for this session.
    pub fn set_notifications_enabled(&self, enabled: bool) {
        if let Some(session) = self.first_ready_session() {
            session.settings().set_notifications_enabled(enabled);
        }
    }

    /// Set whether read receipts are public for this session.
    pub fn set_public_read_receipts_enabled(&self, enabled: bool) {
        if let Some(session) = self.first_ready_session() {
            session.settings().set_public_read_receipts_enabled(enabled);
        }
    }

    /// Set whether typing notifications are sent for this session.
    pub fn set_typing_enabled(&self, enabled: bool) {
        if let Some(session) = self.first_ready_session() {
            session.settings().set_typing_enabled(enabled);
        }
    }

    /// Give the typing users of the given room to the given listener, now
    /// and on every change. Replaces any previous typing listener.
    pub fn set_typing_listener(&self, room_id: String, listener: Arc<dyn TypingListener>) {
        let session = self.first_ready_session();

        let handle = RUNTIME
            .spawn(async move {
                let Some(session) = session else { return };
                let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
                    return;
                };
                let Some(room) = session.room_list().get(&room_id) else {
                    return;
                };

                let mut subscriber = room.subscribe_typing();
                listener.on_update(
                    room.typing_users()
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                );

                while let Some(typing) = subscriber.next().await {
                    listener.on_update(typing.iter().map(ToString::to_string).collect());
                }
            })
            .abort_handle();

        if let Some(previous) = self
            .typing_listener_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// Send a typing notification for the given room.
    ///
    /// Owned `String` because the FFI hands one over.
    #[allow(clippy::needless_pass_by_value)]
    pub fn send_typing(&self, room_id: String, is_typing: bool) {
        let Some(session) = self.first_ready_session() else {
            return;
        };
        let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
            return;
        };
        let Some(room) = session.room_list().get(&room_id) else {
            return;
        };

        room.send_typing_notification(is_typing);
    }

    /// Fetch the image of the given timeline item into a file, returning
    /// its path.
    ///
    /// The item is looked up in the room's timeline so that encrypted
    /// sources come with their keys; only image messages are handled for
    /// now.
    pub async fn get_timeline_media(&self, room_id: String, unique_id: String) -> Option<String> {
        use matrix_sdk_ui::timeline::{MsgLikeKind, TimelineItemContent};
        use ruma::events::room::message::MessageType;

        let session = self.first_ready_session()?;

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).ok()?;
                let room = session.room_list().get(&room_id)?;
                let matrix_timeline = room.live_timeline().matrix_timeline().await?;

                let items = matrix_timeline.items().await;
                let item = items.iter().find(|item| item.unique_id().0 == unique_id)?;
                let event = item.as_event()?;

                let TimelineItemContent::MsgLike(msg_like) = event.content() else {
                    return None;
                };
                let source = match &msg_like.kind {
                    MsgLikeKind::Message(message) => match message.msgtype() {
                        MessageType::Image(image) => image.source.clone(),
                        MessageType::Video(video) => video.source.clone(),
                        MessageType::Audio(audio) => audio.source.clone(),
                        MessageType::File(file) => file.source.clone(),
                        _ => return None,
                    },
                    MsgLikeKind::Sticker(sticker) => match &sticker.content().source {
                        ruma::events::sticker::StickerMediaSource::Plain(url) => {
                            ruma::events::room::MediaSource::Plain(url.clone())
                        }
                        ruma::events::sticker::StickerMediaSource::Encrypted(file) => {
                            ruma::events::room::MediaSource::Encrypted(file.clone())
                        }
                        _ => return None,
                    },
                    _ => return None,
                };

                let request = matrix_sdk::media::MediaRequestParameters {
                    source,
                    format: matrix_sdk::media::MediaFormat::File,
                };

                crate::matrix::media::get_media_file(&session.client(), request)
                    .await
                    .map(|path| path.to_string_lossy().into_owned())
            })
            .await
            .expect("task was not aborted")
    }

    /// Fetch the avatar of the given room into a file, returning its path.
    pub async fn get_room_avatar(&self, room_id: String, size: u32) -> Option<String> {
        let session = self.first_ready_session()?;

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).ok()?;
                let room = session.room_list().get(&room_id)?;
                let avatar_url = room.avatar_url()?;

                crate::matrix::media::get_avatar_file(&session.client(), &avatar_url, size)
                    .await
                    .map(|path| path.to_string_lossy().into_owned())
            })
            .await
            .expect("task was not aborted")
    }

    /// Mark the given room as read, sending a read receipt and moving the
    /// fully-read marker to the end of its timeline, as the application's
    /// room history does when the newest message is looked at.
    pub async fn mark_room_read(&self, room_id: String) {
        let Some(session) = self.first_ready_session() else {
            return;
        };

        RUNTIME
            .spawn(async move {
                let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
                    return;
                };
                let Some(room) = session.room_list().get(&room_id) else {
                    return;
                };
                room.send_receipt(
                    ruma::api::client::receipt::create_receipt::v3::ReceiptType::Read,
                    crate::session::ReceiptPosition::End,
                )
                .await;
                room.send_receipt(
                    ruma::api::client::receipt::create_receipt::v3::ReceiptType::FullyRead,
                    crate::session::ReceiptPosition::End,
                )
                .await;
            })
            .await
            .expect("task was not aborted");
    }

    /// Give the thread rooted at the given event to the given listener,
    /// now and on every change. Replaces any previous thread listener.
    pub fn set_thread_listener(
        &self,
        room_id: String,
        root_event_id: String,
        listener: Arc<dyn TimelineListener>,
    ) {
        let session = self.first_ready_session();

        let handle = RUNTIME
            .spawn(async move {
                let Some(session) = session else { return };
                let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
                    return;
                };
                let Ok(thread_root) = ruma::EventId::parse(&root_event_id) else {
                    return;
                };
                let Some(room) = session.room_list().get(&room_id) else {
                    return;
                };

                let timeline = room.thread_timeline(thread_root);
                let Some((items, mut stream)) = timeline.subscribe_items().await else {
                    return;
                };

                let own_user_id = session.user_id().clone();
                listener.on_update(
                    items
                        .iter()
                        .map(|item| ffi_timeline_item(item, Some(&own_user_id)))
                        .collect(),
                );

                let mut items = items;
                while let Some(diffs) = stream.next().await {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                    listener.on_update(
                        items
                            .iter()
                            .map(|item| ffi_timeline_item(item, Some(&own_user_id)))
                            .collect(),
                    );
                }
            })
            .abort_handle();

        if let Some(previous) = self
            .thread_listener_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// Stop pushing thread updates.
    pub fn clear_thread_listener(&self) {
        if let Some(handle) = self
            .thread_listener_handle
            .lock()
            .expect("mutex is not poisoned")
            .take()
        {
            handle.abort();
        }
    }

    /// Send a plain-text message into the thread rooted at the given
    /// event.
    pub async fn send_thread_message(
        &self,
        room_id: String,
        root_event_id: String,
        body: String,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let thread_root =
                    ruma::EventId::parse(&root_event_id).map_err(|_| CoreError::Failed {
                        msg: "Invalid event ID".to_owned(),
                    })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                room.thread_timeline(thread_root)
                    .send_text(body, None, Vec::new(), false, Vec::new())
                    .await
                    .map_err(|()| CoreError::Failed {
                        msg: "Could not send the message".to_owned(),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Paginate the given room's timeline backwards.
    pub async fn paginate_backwards(&self, room_id: String) {
        let Some(session) = self.first_ready_session() else {
            return;
        };

        RUNTIME
            .spawn(async move {
                let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
                    return;
                };
                let Some(room) = session.room_list().get(&room_id) else {
                    return;
                };
                room.live_timeline().paginate_backwards(20).await;
            })
            .await
            .expect("task was not aborted");
    }

    /// A snapshot of the given room's members, loading the list on first
    /// use — the composer's mention completion reads this.
    pub async fn room_members(&self, room_id: String) -> Vec<FfiMember> {
        let Some(session) = self.first_ready_session() else {
            return Vec::new();
        };

        RUNTIME
            .spawn(async move {
                let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
                    return Vec::new();
                };
                let Some(room) = session.room_list().get(&room_id) else {
                    return Vec::new();
                };
                let member_list = room.member_list();

                // The first call starts the load; wait (bounded) for it.
                for _ in 0..50 {
                    if member_list.state() == crate::utils::LoadingState::Ready {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }

                member_list.snapshot().iter().map(FfiMember::from).collect()
            })
            .await
            .expect("task was not aborted")
    }

    /// Send a message to the given room — Markdown, as the composer
    /// writes it — mentioning the given users.
    pub async fn send_message(
        &self,
        room_id: String,
        body: String,
        mentions: Vec<FfiMention>,
        emoticons: Vec<FfiSticker>,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                // A mention becomes a matrix.to link in the body — the
                // application's composer produces the same anchor — plus
                // its entry in m.mentions.
                let mut markdown = body.clone();
                let mut plain = body;
                let mut user_ids = Vec::new();
                for mention in &mentions {
                    let Ok(user_id) = ruma::UserId::parse(&mention.user_id) else {
                        continue;
                    };
                    let at_name = format!("@{}", mention.display_name);
                    let anchor = format!(
                        "[{}](https://matrix.to/#/{})",
                        mention.display_name, mention.user_id,
                    );
                    markdown = markdown.replace(&at_name, &anchor);
                    // The plain body carries the bare name, as the
                    // application's composer writes it.
                    plain = plain.replace(&at_name, &mention.display_name);
                    user_ids.push(user_id);
                }
                let room_mention = plain.split_whitespace().any(|word| word == "@room");
                let plain = if user_ids.is_empty() {
                    None
                } else {
                    Some(plain)
                };

                let emoticons = emoticons
                    .into_iter()
                    .map(|emoticon| (emoticon.shortcode, emoticon.url, emoticon.body))
                    .collect();

                room.live_timeline()
                    .send_text(markdown, plain, user_ids, room_mention, emoticons)
                    .await
                    .map_err(|()| CoreError::Failed {
                        msg: "Could not send the message".to_owned(),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Toggle the given reaction on the given event in the given room.
    pub async fn toggle_reaction(
        &self,
        room_id: String,
        event_id: String,
        key: String,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let event_id = ruma::EventId::parse(&event_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid event ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                room.live_timeline()
                    .toggle_reaction(event_id, &key)
                    .await
                    .map_err(|()| CoreError::Failed {
                        msg: "Could not toggle the reaction".to_owned(),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Send a plain-text reply to the given event in the given room.
    pub async fn send_reply(
        &self,
        room_id: String,
        in_reply_to: String,
        body: String,
    ) -> Result<(), CoreError> {
        self.with_room_event(room_id, in_reply_to, move |room, event_id| async move {
            room.live_timeline().send_reply(event_id, body).await
        })
        .await
        .map_err(|()| CoreError::Failed {
            msg: "Could not send the reply".to_owned(),
        })
    }

    /// Replace the given event's content with the given plain text.
    pub async fn edit_message(
        &self,
        room_id: String,
        event_id: String,
        new_body: String,
    ) -> Result<(), CoreError> {
        self.with_room_event(room_id, event_id, move |room, event_id| async move {
            room.live_timeline().edit(event_id, new_body).await
        })
        .await
        .map_err(|()| CoreError::Failed {
            msg: "Could not edit the message".to_owned(),
        })
    }

    /// Redact the given event in the given room.
    pub async fn redact_event(&self, room_id: String, event_id: String) -> Result<(), CoreError> {
        self.with_room_event(room_id, event_id, move |room, event_id| async move {
            room.live_timeline().redact(event_id).await
        })
        .await
        .map_err(|()| CoreError::Failed {
            msg: "Could not redact the event".to_owned(),
        })
    }

    /// Move the given room to the given category: accepting an invite is a
    /// move to Normal, declining it (or leaving) a move to Left.
    pub async fn change_room_category(
        &self,
        room_id: String,
        category: FfiTargetRoomCategory,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                room.change_category(category.into())
                    .await
                    .map_err(|change_error| CoreError::Failed {
                        msg: format!("Could not move the room: {change_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Open a direct chat with the given user: the existing one when
    /// there is one, a newly created encrypted DM otherwise. Returns the
    /// room ID.
    pub async fn create_direct_chat(&self, user_id: String) -> Result<String, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let user_id = ruma::UserId::parse(&user_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid user ID".to_owned(),
                })?;

                // The application reuses an existing direct chat rather
                // than opening a second one.
                let existing = session.room_list().snapshot().iter().find_map(|room| {
                    (room.is_joined() && room.direct_member_user_id().as_deref() == Some(&user_id))
                        .then(|| room.room_id().to_string())
                });
                if let Some(room_id) = existing {
                    return Ok(room_id);
                }

                let client = session.client();
                client
                    .create_dm(&user_id)
                    .await
                    .map(|room| room.room_id().to_string())
                    .map_err(|create_error| CoreError::Failed {
                        msg: format!("Could not create the direct chat: {create_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Join the room with the given ID or alias. Returns the room ID.
    pub async fn join_room(&self, room_id_or_alias: String) -> Result<String, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let id_or_alias =
                    ruma::RoomOrAliasId::parse(room_id_or_alias.trim()).map_err(|_| {
                        CoreError::Failed {
                            msg: "Not a room ID or alias".to_owned(),
                        }
                    })?;

                let client = session.client();
                match client.join_room_by_id_or_alias(&id_or_alias, &[]).await {
                    Ok(room) => Ok(room.room_id().to_string()),
                    Err(join_error) => {
                        // A room that cannot be joined may still take a
                        // knock; the sidebar's knock machinery handles the
                        // approval from there.
                        client
                            .knock(id_or_alias.clone(), None, Vec::new())
                            .await
                            .map(|room| room.room_id().to_string())
                            .map_err(|_| CoreError::Failed {
                                msg: format!("Could not join the room: {join_error}"),
                            })
                    }
                }
            })
            .await
            .expect("task was not aborted")
    }

    /// Follow device verifications with the given listener, accepting
    /// the flows the listener's side approves.
    ///
    /// Replaces any previous listener; registering starts watching for
    /// incoming requests.
    pub fn set_verification_listener(&self, listener: Arc<dyn VerificationListener>) {
        let flows = self.verification.clone();

        *flows.listener.lock().expect("mutex is not poisoned") = Some(listener);

        if flows
            .handlers_installed
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }

        let list = self.session_list.clone();
        RUNTIME.spawn(async move {
            use ruma::events::key::verification::{
                request::ToDeviceKeyVerificationRequestEvent,
                start::ToDeviceKeyVerificationStartEvent,
            };

            let session = wait_for_ready_session(&list).await;
            let client = session.client();

            let flows_for_request = flows.clone();
            client.add_event_handler(
                move |event: ToDeviceKeyVerificationRequestEvent, client: matrix_sdk::Client| {
                    let flows = flows_for_request.clone();
                    async move {
                        let flow_id = event.content.transaction_id.to_string();
                        tracing::info!(
                            "Verification request event from {}: {flow_id}",
                            event.sender
                        );
                        let Some(request) = client
                            .encryption()
                            .get_verification_request(&event.sender, &event.content.transaction_id)
                            .await
                        else {
                            tracing::warn!("Request {flow_id} not found in the SDK");
                            return;
                        };
                        flows.insert_request(flow_id.clone(), request);
                        flows.emit(|listener| {
                            listener.on_request(flow_id.clone(), event.sender.to_string());
                        });
                    }
                },
            );

            // A verification request from another user arrives in the
            // direct chat as a message, per the application's
            // identity_verification_view.
            let flows_for_room = flows.clone();
            client.add_event_handler(
                move |event: ruma::events::room::message::OriginalSyncRoomMessageEvent,
                      client: matrix_sdk::Client| {
                    let flows = flows_for_room.clone();
                    async move {
                        use ruma::events::room::message::MessageType;

                        if !matches!(event.content.msgtype, MessageType::VerificationRequest(_)) {
                            return;
                        }
                        if client.user_id().is_some_and(|own| own == event.sender) {
                            return;
                        }
                        let flow_id = event.event_id.to_string();
                        tracing::info!(
                            "In-room verification request from {}: {flow_id}",
                            event.sender
                        );
                        let Some(request) = client
                            .encryption()
                            .get_verification_request(&event.sender, &event.event_id)
                            .await
                        else {
                            tracing::warn!("In-room request {flow_id} not found in the SDK");
                            return;
                        };
                        flows.insert_request(flow_id.clone(), request);
                        flows.emit(|listener| {
                            listener.on_request(flow_id.clone(), event.sender.to_string());
                        });
                    }
                },
            );

            let flows_for_start = flows.clone();
            client.add_event_handler(
                move |event: ToDeviceKeyVerificationStartEvent, client: matrix_sdk::Client| {
                    let flows = flows_for_start.clone();
                    async move {
                        use matrix_sdk::encryption::verification::Verification;

                        let flow_id = event.content.transaction_id.to_string();
                        // A start belonging to a request flow is handled by
                        // that flow; only a bare legacy start arrives alone.
                        if flows.has(&flow_id) {
                            return;
                        }
                        let Some(Verification::SasV1(sas)) = client
                            .encryption()
                            .get_verification(&event.sender, flow_id.as_str())
                            .await
                        else {
                            return;
                        };
                        flows.insert_sas(flow_id.clone(), sas);
                        flows.emit(|listener| {
                            listener.on_request(flow_id.clone(), event.sender.to_string());
                        });
                    }
                },
            );
        });
    }

    /// Search the given room's messages on the server — the application's
    /// search criteria: message bodies, most recent first. Encrypted
    /// rooms cannot be searched by the server.
    pub async fn search_room(
        &self,
        room_id: String,
        search_term: String,
    ) -> Result<Vec<FfiSearchResult>, CoreError> {
        use ruma::{
            api::client::{
                filter::RoomEventFilter,
                search::search_events::{
                    self,
                    v3::{Categories, Criteria, EventContext, OrderBy, SearchKeys},
                },
            },
            assign,
            events::{AnySyncMessageLikeEvent, AnySyncTimelineEvent, MessageLikeEventType},
            serde::Raw,
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let client = session.client();

                let filter = assign!(RoomEventFilter::default(), {
                    rooms: Some(vec![room_id]),
                    types: Some(vec![MessageLikeEventType::RoomMessage.to_string()]),
                    limit: Some(ruma::UInt::from(30u32)),
                });
                let criteria = assign!(Criteria::new(search_term), {
                    keys: Some(vec![SearchKeys::ContentBody]),
                    filter,
                    order_by: Some(OrderBy::Recent),
                    event_context: assign!(EventContext::new(), {
                        before_limit: ruma::UInt::default(),
                        after_limit: ruma::UInt::default(),
                    }),
                });
                let categories = assign!(Categories::new(), { room_events: Some(criteria) });
                let request = search_events::v3::Request::new(categories);

                let response =
                    client
                        .send(request)
                        .await
                        .map_err(|search_error| CoreError::Failed {
                            msg: format!("Could not search: {search_error}"),
                        })?;

                Ok(response
                    .search_categories
                    .room_events
                    .results
                    .iter()
                    .filter_map(|result| result.result.as_ref())
                    .map(Raw::cast_ref_unchecked::<AnySyncTimelineEvent>)
                    .filter_map(|raw| raw.deserialize().ok())
                    .filter_map(|event| match event {
                        AnySyncTimelineEvent::MessageLike(
                            AnySyncMessageLikeEvent::RoomMessage(message),
                        ) => {
                            let original = message.as_original()?;
                            Some(FfiSearchResult {
                                event_id: original.event_id.to_string(),
                                sender: original.sender.to_string(),
                                body: original.content.msgtype.body().to_owned(),
                                timestamp: original.origin_server_ts.get().into(),
                            })
                        }
                        _ => None,
                    })
                    .collect())
            })
            .await
            .expect("task was not aborted")
    }

    /// Set the given room's name and topic.
    pub async fn set_room_details(
        &self,
        room_id: String,
        name: String,
        topic: String,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();

                if room.name().unwrap_or_default() != name {
                    matrix_room
                        .set_name(name)
                        .await
                        .map_err(|set_error| CoreError::Failed {
                            msg: format!("Could not set the name: {set_error}"),
                        })?;
                }
                if room.topic().unwrap_or_default() != topic {
                    matrix_room
                        .set_room_topic(&topic)
                        .await
                        .map_err(|set_error| CoreError::Failed {
                            msg: format!("Could not set the topic: {set_error}"),
                        })?;
                }
                Ok(())
            })
            .await
            .expect("task was not aborted")
    }

    /// The rooms inside the given space, from the server's hierarchy.
    pub async fn space_children(&self, space_id: String) -> Result<Vec<FfiSpaceChild>, CoreError> {
        use ruma::api::client::space::get_hierarchy;

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let space_id = ruma::RoomId::parse(&space_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;

                let client = session.client();
                let request = get_hierarchy::v1::Request::new(space_id.clone());
                let response =
                    client
                        .send(request)
                        .await
                        .map_err(|hierarchy_error| CoreError::Failed {
                            msg: format!("Could not load the space: {hierarchy_error}"),
                        })?;

                Ok(response
                    .rooms
                    .into_iter()
                    .filter(|chunk| chunk.summary.room_id != space_id)
                    .map(|chunk| {
                        let summary = &chunk.summary;
                        FfiSpaceChild {
                            room_id: summary.room_id.to_string(),
                            name: summary.name.clone(),
                            topic: summary.topic.clone(),
                            num_joined_members: summary.num_joined_members.into(),
                            is_joined: session
                                .room_list()
                                .get(&summary.room_id)
                                .is_some_and(|room| room.is_joined()),
                            is_space: summary.room_type == Some(ruma::room::RoomType::Space),
                        }
                    })
                    .collect())
            })
            .await
            .expect("task was not aborted")
    }

    /// Feed a scanned QR code into the verification with the given flow
    /// ID. The outcome arrives through the listener: done, or cancelled.
    pub async fn scan_qr(&self, flow_id: String, data: Vec<u8>) -> Result<(), CoreError> {
        use matrix_sdk::encryption::verification::QrVerificationData;

        let flows = self.verification.clone();

        RUNTIME
            .spawn(async move {
                let request = {
                    let map = flows.flows.lock().expect("mutex is not poisoned");
                    match map.get(&flow_id) {
                        Some(VerificationFlow::Request(request)) => request.clone(),
                        _ => {
                            return Err(CoreError::Failed {
                                msg: "No verification in progress".to_owned(),
                            });
                        }
                    }
                };

                let data =
                    QrVerificationData::from_bytes(&data).map_err(|_| CoreError::Failed {
                        msg: "Not a verification QR code".to_owned(),
                    })?;
                let qr = request
                    .scan_qr_code(data)
                    .await
                    .map_err(|scan_error| CoreError::Failed {
                        msg: format!("Could not scan the code: {scan_error}"),
                    })?
                    .ok_or_else(|| CoreError::Failed {
                        msg: "The code belongs to another verification".to_owned(),
                    })?;

                let follow_flows = flows.clone();
                RUNTIME.spawn(async move {
                    use futures_util::StreamExt;
                    use matrix_sdk::encryption::verification::QrVerificationState;

                    let mut changes = qr.changes();
                    while let Some(state) = changes.next().await {
                        match state {
                            QrVerificationState::Done { .. } => {
                                let flow_id = flow_id.clone();
                                follow_flows.emit(move |listener| listener.on_done(flow_id));
                                break;
                            }
                            QrVerificationState::Cancelled(info) => {
                                let flow_id = flow_id.clone();
                                let reason = info.reason().to_owned();
                                follow_flows
                                    .emit(move |listener| listener.on_cancelled(flow_id, reason));
                                break;
                            }
                            _ => {}
                        }
                    }
                });

                Ok(())
            })
            .await
            .expect("task was not aborted")
    }

    /// Ask the account's verified sessions to verify this one. The flow
    /// then arrives through the listener like an incoming one: emojis,
    /// then done.
    pub async fn request_verification(&self) -> Result<String, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };
        let flows = self.verification.clone();

        RUNTIME
            .spawn(async move {
                let client = session.client();
                let user_id = client.user_id().expect("logged in").to_owned();
                let identity = client
                    .encryption()
                    .get_user_identity(&user_id)
                    .await
                    .ok()
                    .flatten()
                    .ok_or_else(|| CoreError::Failed {
                        msg: "No identity to verify against".to_owned(),
                    })?;

                let request = identity
                    .request_verification()
                    .await
                    .map_err(|request_error| CoreError::Failed {
                        msg: format!("Could not request verification: {request_error}"),
                    })?;
                let flow_id = request.flow_id().to_owned();
                flows.insert_request(flow_id.clone(), request.clone());

                // As the requester we wait for the other side to accept and
                // start; the SAS is then followed like any other.
                let follow_flows = flows.clone();
                let follow_flow_id = flow_id.clone();
                RUNTIME.spawn(async move {
                    use futures_util::StreamExt;
                    use matrix_sdk::encryption::verification::{
                        Verification, VerificationRequestState,
                    };

                    let mut changes = request.changes();
                    while let Some(state) = changes.next().await {
                        match state {
                            VerificationRequestState::Transitioned {
                                verification: Verification::SasV1(sas),
                            } => {
                                follow_flows.insert_sas(follow_flow_id.clone(), sas.clone());
                                if let Err(sas_error) = sas.accept().await {
                                    tracing::error!("Could not accept SAS: {sas_error}");
                                    return;
                                }
                                follow_flows.clone().follow_sas(follow_flow_id, sas);
                                return;
                            }
                            VerificationRequestState::Cancelled(info) => {
                                let reason = info.reason().to_owned();
                                follow_flows.emit(move |listener| {
                                    listener.on_cancelled(follow_flow_id, reason);
                                });
                                return;
                            }
                            _ => {}
                        }
                    }
                });

                Ok(flow_id)
            })
            .await
            .expect("task was not aborted")
    }

    /// Ask another user to verify: the request goes into the direct
    /// chat as a message, and the flow then runs like any other SAS.
    pub async fn request_user_verification(&self, user_id: String) -> Result<String, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };
        let flows = self.verification.clone();

        RUNTIME
            .spawn(async move {
                let user_id = ruma::UserId::parse(&user_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid user ID".to_owned(),
                })?;
                let client = session.client();
                let identity = client
                    .encryption()
                    .get_user_identity(&user_id)
                    .await
                    .ok()
                    .flatten()
                    .ok_or_else(|| CoreError::Failed {
                        msg: "This user has no cross-signing identity yet".to_owned(),
                    })?;

                let request = identity
                    .request_verification()
                    .await
                    .map_err(|request_error| CoreError::Failed {
                        msg: format!("Could not request verification: {request_error}"),
                    })?;
                let flow_id = request.flow_id().to_owned();
                flows.insert_request(flow_id.clone(), request.clone());

                // As the requester we wait for the other side to accept and
                // start; the SAS is then followed like any other.
                let follow_flows = flows.clone();
                let follow_flow_id = flow_id.clone();
                RUNTIME.spawn(async move {
                    use futures_util::StreamExt;
                    use matrix_sdk::encryption::verification::{
                        Verification, VerificationRequestState,
                    };

                    let mut changes = request.changes();
                    while let Some(state) = changes.next().await {
                        match state {
                            VerificationRequestState::Transitioned {
                                verification: Verification::SasV1(sas),
                            } => {
                                follow_flows.insert_sas(follow_flow_id.clone(), sas.clone());
                                if let Err(sas_error) = sas.accept().await {
                                    tracing::error!("Could not accept SAS: {sas_error}");
                                    return;
                                }
                                follow_flows.clone().follow_sas(follow_flow_id, sas);
                                return;
                            }
                            VerificationRequestState::Cancelled(info) => {
                                let reason = info.reason().to_owned();
                                follow_flows.emit(move |listener| {
                                    listener.on_cancelled(follow_flow_id, reason);
                                });
                                return;
                            }
                            _ => {}
                        }
                    }
                });

                Ok(flow_id)
            })
            .await
            .expect("task was not aborted")
    }

    /// Accept the verification with the given flow ID; the emojis arrive
    /// through the listener when both sides are ready.
    pub async fn accept_verification(&self, flow_id: String) {
        let flows = self.verification.clone();
        RUNTIME
            .spawn(async move {
                flows.accept(&flow_id).await;
            })
            .await
            .expect("task was not aborted");
    }

    /// Confirm that the emojis matched.
    pub async fn confirm_verification(&self, flow_id: String) {
        let flows = self.verification.clone();
        RUNTIME
            .spawn(async move {
                flows.confirm(&flow_id).await;
            })
            .await
            .expect("task was not aborted");
    }

    /// Cancel the verification — the emojis did not match, or the user
    /// declined.
    pub async fn cancel_verification(&self, flow_id: String) {
        let flows = self.verification.clone();
        RUNTIME
            .spawn(async move {
                flows.cancel(&flow_id).await;
            })
            .await
            .expect("task was not aborted");
    }

    /// Where account recovery stands for the first ready session.
    pub async fn recovery_state(&self) -> FfiRecoveryState {
        use matrix_sdk::encryption::recovery::RecoveryState;

        let Some(session) = self.first_ready_session() else {
            return FfiRecoveryState::Unknown;
        };

        RUNTIME
            .spawn(async move {
                match session.client().encryption().recovery().state() {
                    RecoveryState::Unknown => FfiRecoveryState::Unknown,
                    RecoveryState::Enabled => FfiRecoveryState::Enabled,
                    RecoveryState::Disabled => FfiRecoveryState::Disabled,
                    RecoveryState::Incomplete => FfiRecoveryState::Incomplete,
                }
            })
            .await
            .expect("task was not aborted")
    }

    /// Set up recovery, returning the recovery key to write down.
    pub async fn enable_recovery(&self) -> Result<String, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                session
                    .client()
                    .encryption()
                    .recovery()
                    .enable()
                    .await
                    .map_err(|enable_error| CoreError::Failed {
                        msg: format!("Could not set up recovery: {enable_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Recover the account's secrets with the given recovery key.
    pub async fn recover(&self, recovery_key: String) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                session
                    .client()
                    .encryption()
                    .recovery()
                    .recover(recovery_key.trim())
                    .await
                    .map_err(|recover_error| CoreError::Failed {
                        msg: format!("Could not recover: {recover_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Send the file at the given path as an attachment to the given room.
    pub async fn send_attachment(
        &self,
        room_id: String,
        file_path: String,
        mime_type: String,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let mime = mime_type
                    .parse::<mime::Mime>()
                    .unwrap_or(mime::APPLICATION_OCTET_STREAM);

                let size = std::fs::metadata(&file_path)
                    .ok()
                    .map(|metadata| metadata.len());
                check_upload_size(&session.client(), size).await?;

                room.live_timeline()
                    .send_attachment(file_path.into(), mime)
                    .await
                    .map_err(|()| CoreError::Failed {
                        msg: "Could not send the attachment".to_owned(),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Search the GIF service.
    ///
    /// Pages are 1-indexed. GIFs without both a preview and a sendable
    /// variant are dropped, so every entry of the result can be presented
    /// and sent.
    pub async fn search_gifs(&self, query: String, page: u32) -> Result<FfiGifPage, CoreError> {
        RUNTIME
            .spawn(async move {
                let result = crate::klipy::search(&query, page)
                    .await
                    .map_err(|search_error| CoreError::Failed {
                        msg: format!("{search_error}"),
                    })?;

                let gifs = result
                    .gifs
                    .iter()
                    .filter_map(|gif| {
                        let preview = gif.preview()?;
                        let selection = gif.to_selection()?;
                        Some(FfiGif {
                            id: gif.id,
                            slug: selection.slug,
                            title: selection.title,
                            preview_url: preview.url.clone(),
                            preview_width: preview.width,
                            preview_height: preview.height,
                            send_url: selection.url,
                            send_width: selection.width,
                            send_height: selection.height,
                            send_size: selection.size,
                        })
                    })
                    .collect();

                Ok(FfiGifPage {
                    gifs,
                    has_next: result.has_next,
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Download the preview of a GIF, so the picker can present it.
    ///
    /// Downloading through the core keeps one HTTP stack, one TLS
    /// configuration and one size guard for everything the app fetches.
    pub async fn fetch_gif_preview(&self, url: String) -> Result<Vec<u8>, CoreError> {
        RUNTIME
            .spawn(async move {
                // A preview is tens of kilobytes; 2 MB is only a guard.
                crate::http::fetch(&url, 2 * 1024 * 1024)
                    .await
                    .map_err(|fetch_error| CoreError::Failed {
                        msg: format!("{fetch_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Download the given GIF and send it to the given room, then report the
    /// share to the GIF service.
    pub async fn send_gif(&self, room_id: String, gif: FfiGif) -> Result<(), CoreError> {
        use ruma::events::{
            AnyMessageLikeEventContent,
            room::ImageInfo,
            sticker::{StickerEventContent, StickerMediaSource},
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                // `to_send` can fall back to a variant above the preferred
                // send size, so the guard here is looser than that limit.
                let bytes = crate::http::fetch(&gif.send_url, 20 * 1024 * 1024)
                    .await
                    .map_err(|fetch_error| CoreError::Failed {
                        msg: format!("Could not download the GIF: {fetch_error}"),
                    })?;

                // The application sends a GIF as a sticker: uploaded, never
                // linked, encrypted where the room is.
                let client = session.client();
                check_upload_size(&client, Some(bytes.len() as u64)).await?;
                let source = if room.is_encrypted() {
                    let mut cursor = std::io::Cursor::new(bytes.clone());
                    let file = client.upload_encrypted_file(&mut cursor).await.map_err(
                        |upload_error| CoreError::Failed {
                            msg: format!("Could not upload the GIF: {upload_error}"),
                        },
                    )?;
                    StickerMediaSource::Encrypted(Box::new(file))
                } else {
                    let response = client
                        .media()
                        .upload(&mime::IMAGE_GIF, bytes.clone(), None)
                        .await
                        .map_err(|upload_error| CoreError::Failed {
                            msg: format!("Could not upload the GIF: {upload_error}"),
                        })?;
                    StickerMediaSource::Plain(response.content_uri)
                };

                let mut info = ImageInfo::new();
                info.width = Some(gif.send_width.into());
                info.height = Some(gif.send_height.into());
                info.size = bytes.len().try_into().ok();
                info.mimetype = Some(mime::IMAGE_GIF.to_string());
                // Without this the receiving client asks its homeserver for
                // a thumbnail, which is a still frame.
                info.is_animated = Some(true);

                let content = StickerEventContent::with_source(gif.title, info, source);
                room.matrix_room()
                    .send(AnyMessageLikeEventContent::Sticker(content))
                    .await
                    .map_err(|send_error| CoreError::Failed {
                        msg: format!("Could not send the GIF: {send_error}"),
                    })?;

                crate::klipy::report_share(&gif.slug).await;
                Ok(())
            })
            .await
            .expect("task was not aborted")
    }

    /// One page of the room's media history, walking backward from
    /// `from`, or from the end of the room when it is `None`.
    ///
    /// This is the application's history viewer pagination: `/messages`
    /// filtered to message events — with a URL filter where the server can
    /// see the content, without one in encrypted rooms — then classified
    /// by message type on our side.
    pub async fn room_media_history(
        &self,
        room_id: String,
        from: Option<String>,
    ) -> Result<FfiHistoryPage, CoreError> {
        use ruma::{
            api::client::filter::{RoomEventFilter, UrlFilter},
            assign,
            events::MessageLikeEventType,
            uint,
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                // In an encrypted room the server cannot see the content, so
                // the URL filter would drop everything.
                let filter = if room.is_encrypted() {
                    assign!(RoomEventFilter::default(), {
                        types: Some(vec![
                            MessageLikeEventType::RoomEncrypted.to_string(),
                            MessageLikeEventType::RoomMessage.to_string(),
                        ]),
                    })
                } else {
                    assign!(RoomEventFilter::default(), {
                        types: Some(vec![MessageLikeEventType::RoomMessage.to_string()]),
                        url_filter: Some(UrlFilter::EventsWithUrl),
                    })
                };
                let options = assign!(
                    matrix_sdk::room::MessagesOptions::backward().from(from.as_deref()),
                    {
                        limit: uint!(20),
                        filter,
                    }
                );

                let response =
                    room.matrix_room()
                        .messages(options)
                        .await
                        .map_err(|messages_error| CoreError::Failed {
                            msg: format!("Could not load the media history: {messages_error}"),
                        })?;

                let events = response
                    .chunk
                    .iter()
                    .filter_map(|event| ffi_history_event(event.raw()))
                    .collect();

                Ok(FfiHistoryPage {
                    events,
                    next_token: response.end,
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Log the session out and remove it from the app.
    ///
    /// The pusher, if one was set, must be removed before this: logging
    /// out invalidates the access token that could remove it.
    pub async fn logout(&self) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        let session_id = session.session_id().to_owned();
        RUNTIME
            .spawn(async move {
                session
                    .log_out()
                    .await
                    .map_err(|logout_error| CoreError::Failed { msg: logout_error })
            })
            .await
            .expect("task was not aborted")?;

        self.session_list.remove(&session_id);
        Ok(())
    }

    /// The account's current profile.
    pub async fn account_profile(&self) -> Option<FfiProfile> {
        let session = self.first_ready_session()?;

        RUNTIME
            .spawn(async move {
                let profile = session.client().account().fetch_user_profile().await.ok()?;
                Some(FfiProfile {
                    display_name: profile
                        .get_static::<ruma::api::client::profile::DisplayName>()
                        .ok()
                        .flatten(),
                    avatar_url: profile
                        .get_static::<ruma::api::client::profile::AvatarUrl>()
                        .ok()
                        .flatten()
                        .map(|url| url.to_string()),
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Change the account's display name.
    pub async fn set_display_name(&self, name: String) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                session
                    .client()
                    .account()
                    .set_display_name(Some(name.trim()))
                    .await
                    .map_err(|profile_error| CoreError::Failed {
                        msg: format!("Could not change the display name: {profile_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Upload the file at the given path as the account's avatar.
    pub async fn set_account_avatar(
        &self,
        file_path: String,
        mime_type: String,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let bytes = std::fs::read(&file_path).map_err(|read_error| CoreError::Failed {
                    msg: format!("Could not read the image: {read_error}"),
                })?;
                let mime = mime_type.parse::<mime::Mime>().unwrap_or(mime::IMAGE_JPEG);

                session
                    .client()
                    .account()
                    .upload_avatar(&mime, bytes)
                    .await
                    .map(|_| ())
                    .map_err(|avatar_error| CoreError::Failed {
                        msg: format!("Could not upload the avatar: {avatar_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Invite the given user to the given room.
    pub async fn invite_user(&self, room_id: String, user_id: String) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let user_id =
                    ruma::UserId::parse(user_id.trim()).map_err(|_| CoreError::Failed {
                        msg: "That is not a valid user ID".to_owned(),
                    })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                room.matrix_room()
                    .invite_user_by_id(&user_id)
                    .await
                    .map_err(|invite_error| CoreError::Failed {
                        msg: format!("Could not invite: {invite_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Create a room, as the application's create dialog does: private
    /// rooms can be encrypted from birth, public rooms get an alias.
    ///
    /// Returns the new room's ID.
    pub async fn create_room(
        &self,
        name: String,
        topic: Option<String>,
        public: bool,
        encrypted: bool,
        alias: Option<String>,
    ) -> Result<String, CoreError> {
        use ruma::{
            api::client::room::{Visibility, create_room},
            assign,
            events::{InitialStateEvent, room::encryption::RoomEncryptionEventContent},
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let mut request = assign!(create_room::v3::Request::new(), {
                    name: Some(name.trim().to_owned()),
                    topic: topic.filter(|t| !t.trim().is_empty()),
                });

                if public {
                    request.visibility = Visibility::Public;
                    request.room_alias_name =
                        alias.map(|a| a.trim().trim_start_matches('#').to_owned());
                } else {
                    request.visibility = Visibility::Private;
                    if encrypted {
                        let event = InitialStateEvent::with_empty_state_key(
                            RoomEncryptionEventContent::with_recommended_defaults(),
                        );
                        request.initial_state = vec![event.to_raw_any()];
                    }
                }

                let room = session
                    .client()
                    .create_room(request)
                    .await
                    .map_err(|create_error| CoreError::Failed {
                        msg: format!("Could not create the room: {create_error}"),
                    })?;

                Ok(room.room_id().to_string())
            })
            .await
            .expect("task was not aborted")
    }

    /// The account's sessions, ours first.
    pub async fn list_devices(&self) -> Result<Vec<FfiDevice>, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let client = session.client();
                let own_device_id = client.device_id().map(ToOwned::to_owned);
                let own_user_id = client.user_id().map(ToOwned::to_owned);

                let response =
                    client
                        .devices()
                        .await
                        .map_err(|devices_error| CoreError::Failed {
                            msg: format!("Could not load the sessions: {devices_error}"),
                        })?;

                let mut devices = Vec::new();
                for device in response.devices {
                    let is_current = own_device_id.as_deref() == Some(&device.device_id);
                    let is_verified = if let Some(user_id) = &own_user_id {
                        client
                            .encryption()
                            .get_device(user_id, &device.device_id)
                            .await
                            .ok()
                            .flatten()
                            .is_some_and(|crypto_device| crypto_device.is_verified())
                    } else {
                        false
                    };

                    devices.push(FfiDevice {
                        device_id: device.device_id.to_string(),
                        display_name: device.display_name,
                        is_current,
                        is_verified,
                        last_seen_ts: device.last_seen_ts.map(|ts| ts.0.into()),
                        last_seen_ip: device.last_seen_ip,
                    });
                }

                // Ours first, then most recently seen.
                devices.sort_by_key(|device| {
                    (
                        !device.is_current,
                        u64::MAX - device.last_seen_ts.unwrap_or(0),
                    )
                });
                Ok(devices)
            })
            .await
            .expect("task was not aborted")
    }

    /// Rename one of the account's sessions.
    pub async fn rename_device(&self, device_id: String, name: String) -> Result<(), CoreError> {
        use ruma::api::client::device::update_device;

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let device_id: ruma::OwnedDeviceId = device_id.into();
                let request = ruma::assign!(update_device::v3::Request::new(device_id), {
                    display_name: Some(name.trim().to_owned()),
                });
                session
                    .client()
                    .send(request)
                    .await
                    .map(|_| ())
                    .map_err(|rename_error| CoreError::Failed {
                        msg: format!("Could not rename the session: {rename_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Sign another of the account's sessions out. The server demands the
    /// password again for this.
    pub async fn sign_out_device(
        &self,
        device_id: String,
        password: String,
    ) -> Result<(), CoreError> {
        use ruma::api::client::uiaa::{AuthData, MatrixUserIdentifier, Password};

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let client = session.client();
                let device_id: ruma::OwnedDeviceId = device_id.into();
                let user_id = client.user_id().ok_or_else(|| CoreError::Failed {
                    msg: "No user".to_owned(),
                })?;

                // First pass to learn the auth session, second to answer it.
                let devices = &[device_id];
                match client.delete_devices(devices, None).await {
                    Ok(_) => Ok(()),
                    Err(delete_error) => {
                        let Some(info) = delete_error.as_uiaa_response() else {
                            return Err(CoreError::Failed {
                                msg: format!("Could not sign the session out: {delete_error}"),
                            });
                        };
                        let auth = AuthData::Password(ruma::assign!(
                            Password::new(
                                MatrixUserIdentifier::new(user_id.to_string()).into(),
                                password,
                            ),
                            { session: info.session.clone() }
                        ));
                        client
                            .delete_devices(devices, Some(auth))
                            .await
                            .map(|_| ())
                            .map_err(|retry_error| CoreError::Failed {
                                msg: format!("Could not sign the session out: {retry_error}"),
                            })
                    }
                }
            })
            .await
            .expect("task was not aborted")
    }

    /// How the given room notifies, as far as the user has said.
    pub async fn room_notification_mode(&self, room_id: String) -> FfiRoomNotificationMode {
        use matrix_sdk::notification_settings::RoomNotificationMode;

        let Some(session) = self.first_ready_session() else {
            return FfiRoomNotificationMode::Default;
        };

        RUNTIME
            .spawn(async move {
                let Ok(room_id) = ruma::RoomId::parse(&room_id) else {
                    return FfiRoomNotificationMode::Default;
                };
                let settings = session.client().notification_settings().await;
                match settings
                    .get_user_defined_room_notification_mode(&room_id)
                    .await
                {
                    Some(RoomNotificationMode::AllMessages) => FfiRoomNotificationMode::All,
                    Some(RoomNotificationMode::MentionsAndKeywordsOnly) => {
                        FfiRoomNotificationMode::MentionsOnly
                    }
                    Some(RoomNotificationMode::Mute) => FfiRoomNotificationMode::Mute,
                    None => FfiRoomNotificationMode::Default,
                }
            })
            .await
            .expect("task was not aborted")
    }

    /// Set how the given room notifies, or hand it back to the defaults.
    pub async fn set_room_notification_mode(
        &self,
        room_id: String,
        mode: FfiRoomNotificationMode,
    ) -> Result<(), CoreError> {
        use matrix_sdk::notification_settings::RoomNotificationMode;

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let settings = session.client().notification_settings().await;
                let result = match mode {
                    FfiRoomNotificationMode::Default => {
                        settings.delete_user_defined_room_rules(&room_id).await
                    }
                    FfiRoomNotificationMode::All => {
                        settings
                            .set_room_notification_mode(&room_id, RoomNotificationMode::AllMessages)
                            .await
                    }
                    FfiRoomNotificationMode::MentionsOnly => {
                        settings
                            .set_room_notification_mode(
                                &room_id,
                                RoomNotificationMode::MentionsAndKeywordsOnly,
                            )
                            .await
                    }
                    FfiRoomNotificationMode::Mute => {
                        settings
                            .set_room_notification_mode(&room_id, RoomNotificationMode::Mute)
                            .await
                    }
                };
                result.map_err(|mode_error| CoreError::Failed {
                    msg: format!("Could not change the notification mode: {mode_error}"),
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// The users the account ignores, as `m.ignored_user_list` lists
    /// them — the application's safety page order.
    pub async fn ignored_users(&self) -> Vec<String> {
        use ruma::events::ignored_user_list::IgnoredUserListEventContent;

        let Some(session) = self.first_ready_session() else {
            return Vec::new();
        };

        RUNTIME
            .spawn(async move {
                let Ok(Some(raw)) = session
                    .client()
                    .account()
                    .account_data::<IgnoredUserListEventContent>()
                    .await
                else {
                    return Vec::new();
                };
                raw.deserialize()
                    .map(|content| {
                        content
                            .ignored_users
                            .into_keys()
                            .map(|user_id| user_id.to_string())
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .await
            .expect("task was not aborted")
    }

    /// Ignore the given user: their messages disappear everywhere.
    pub async fn ignore_user(&self, user_id: String) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let user_id = ruma::UserId::parse(&user_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid user ID".to_owned(),
                })?;
                session
                    .client()
                    .account()
                    .ignore_user(&user_id)
                    .await
                    .map_err(|ignore_error| CoreError::Failed {
                        msg: format!("Could not ignore the user: {ignore_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Stop ignoring the given user.
    pub async fn unignore_user(&self, user_id: String) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let user_id = ruma::UserId::parse(&user_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid user ID".to_owned(),
                })?;
                session
                    .client()
                    .account()
                    .unignore_user(&user_id)
                    .await
                    .map_err(|unignore_error| CoreError::Failed {
                        msg: format!("Could not stop ignoring the user: {unignore_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// The keywords that trigger notifications, from the account's
    /// enabled keyword push rules.
    pub async fn notification_keywords(&self) -> Vec<String> {
        let Some(session) = self.first_ready_session() else {
            return Vec::new();
        };

        RUNTIME
            .spawn(async move {
                let settings = session.client().notification_settings().await;
                settings.enabled_keywords().await.into_iter().collect()
            })
            .await
            .expect("task was not aborted")
    }

    /// Add a keyword that triggers notifications, returning the updated
    /// list.
    ///
    /// The updated list comes from the same settings instance that made
    /// the change: a fresh read would race the sync echo of the rules.
    pub async fn add_notification_keyword(
        &self,
        keyword: String,
    ) -> Result<Vec<String>, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let settings = session.client().notification_settings().await;
                settings
                    .add_keyword(keyword)
                    .await
                    .map_err(|keyword_error| CoreError::Failed {
                        msg: format!("Could not add the keyword: {keyword_error}"),
                    })?;
                Ok(settings.enabled_keywords().await.into_iter().collect())
            })
            .await
            .expect("task was not aborted")
    }

    /// Remove a keyword from the notification triggers, returning the
    /// updated list.
    pub async fn remove_notification_keyword(
        &self,
        keyword: String,
    ) -> Result<Vec<String>, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let settings = session.client().notification_settings().await;
                settings
                    .remove_keyword(&keyword)
                    .await
                    .map_err(|keyword_error| CoreError::Failed {
                        msg: format!("Could not remove the keyword: {keyword_error}"),
                    })?;
                Ok(settings.enabled_keywords().await.into_iter().collect())
            })
            .await
            .expect("task was not aborted")
    }

    /// Change the room's avatar: upload the file, then point
    /// `m.room.avatar` at it, as the application's edit-details page
    /// does. Width and height come from the embedder, which decoded the
    /// picture to show it.
    pub async fn set_room_avatar(
        &self,
        room_id: String,
        file_path: String,
        mime_type: String,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Result<(), CoreError> {
        use ruma::events::room::avatar::ImageInfo as AvatarImageInfo;

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let mime = mime_type
                    .parse::<mime::Mime>()
                    .unwrap_or(mime::APPLICATION_OCTET_STREAM);

                let data = std::fs::read(&file_path).map_err(|read_error| CoreError::Failed {
                    msg: format!("Could not read the file: {read_error}"),
                })?;
                let size = u64::try_from(data.len()).ok();
                check_upload_size(&session.client(), size).await?;

                let response = session
                    .client()
                    .media()
                    .upload(&mime, data, None)
                    .await
                    .map_err(|upload_error| CoreError::Failed {
                        msg: format!("Could not upload the avatar: {upload_error}"),
                    })?;

                let mut info = AvatarImageInfo::new();
                info.width = width.map(Into::into);
                info.height = height.map(Into::into);
                info.size = size.and_then(|s| u32::try_from(s).ok()).map(Into::into);
                info.mimetype = Some(mime.to_string());

                room.matrix_room()
                    .set_avatar_url(&response.content_uri, Some(info))
                    .await
                    .map(|_| ())
                    .map_err(|set_error| CoreError::Failed {
                        msg: format!("Could not change the avatar: {set_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Remove the room's avatar.
    pub async fn remove_room_avatar(&self, room_id: String) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                room.matrix_room()
                    .remove_avatar()
                    .await
                    .map(|_| ())
                    .map_err(|remove_error| CoreError::Failed {
                        msg: format!("Could not remove the avatar: {remove_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// The room's join rule, with what this room's version supports and
    /// whether we may change it.
    pub async fn room_join_rule(&self, room_id: String) -> Result<FfiJoinRuleInfo, CoreError> {
        use ruma::events::room::join_rules::{JoinRule, RoomJoinRulesEventContent};

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();

                let rule = read_state_content::<RoomJoinRulesEventContent>(&matrix_room)
                    .await
                    .map(|content| content.join_rule)
                    .unwrap_or(JoinRule::Invite);

                let (value, allow) = match &rule {
                    JoinRule::Public => (FfiJoinRuleValue::Public, Vec::new()),
                    JoinRule::Invite => (FfiJoinRuleValue::Invite, Vec::new()),
                    JoinRule::Knock => (FfiJoinRuleValue::Knock, Vec::new()),
                    JoinRule::Restricted(restricted) => {
                        (FfiJoinRuleValue::Restricted, allow_room_ids(restricted))
                    }
                    JoinRule::KnockRestricted(restricted) => (
                        FfiJoinRuleValue::KnockRestricted,
                        allow_room_ids(restricted),
                    ),
                    _ => (FfiJoinRuleValue::Unsupported, Vec::new()),
                };

                let authorization = matrix_room
                    .clone_info()
                    .room_version_rules_or_default()
                    .authorization;

                Ok(FfiJoinRuleInfo {
                    value,
                    allow_room_ids: allow,
                    supports_knock: authorization.knocking,
                    supports_restricted: authorization.restricted_join_rule,
                    supports_knock_restricted: authorization.knock_restricted_join_rule,
                    can_change: can_send_state(
                        &matrix_room,
                        ruma::events::StateEventType::RoomJoinRules,
                    )
                    .await,
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Change the room's join rule, computed as the application does: a
    /// restricted rule takes the given space, or keeps the saved allow
    /// list when none is given — never an empty one.
    pub async fn set_room_join_rule(
        &self,
        room_id: String,
        value: FfiJoinRuleValue,
        allow_space_id: Option<String>,
    ) -> Result<(), CoreError> {
        use ruma::events::room::join_rules::{
            AllowRule, JoinRule, Restricted, RoomJoinRulesEventContent,
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();

                let restricted = || async {
                    if let Some(space_id) = &allow_space_id {
                        let space_id =
                            ruma::RoomId::parse(space_id).map_err(|_| CoreError::Failed {
                                msg: "Invalid space ID".to_owned(),
                            })?;
                        return Ok::<_, CoreError>(Restricted::new(vec![
                            AllowRule::room_membership(space_id),
                        ]));
                    }
                    // Carry the saved list over verbatim, so changing
                    // whether people may knock never changes who may join.
                    let current = read_state_content::<RoomJoinRulesEventContent>(&matrix_room)
                        .await
                        .map(|content| content.join_rule);
                    match current {
                        Some(
                            JoinRule::Restricted(restricted)
                            | JoinRule::KnockRestricted(restricted),
                        ) => Ok(restricted),
                        _ => Err(CoreError::Failed {
                            msg: "A membership rule needs a space".to_owned(),
                        }),
                    }
                };

                let rule = match value {
                    FfiJoinRuleValue::Public => JoinRule::Public,
                    FfiJoinRuleValue::Invite => JoinRule::Invite,
                    FfiJoinRuleValue::Knock => JoinRule::Knock,
                    FfiJoinRuleValue::Restricted => JoinRule::Restricted(restricted().await?),
                    FfiJoinRuleValue::KnockRestricted => {
                        JoinRule::KnockRestricted(restricted().await?)
                    }
                    FfiJoinRuleValue::Unsupported => {
                        return Err(CoreError::Failed {
                            msg: "Cannot set an unsupported rule".to_owned(),
                        });
                    }
                };

                matrix_room
                    .send_state_event(RoomJoinRulesEventContent::new(rule))
                    .await
                    .map(|_| ())
                    .map_err(|send_error| CoreError::Failed {
                        msg: format!("Could not change who can join: {send_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// The room's history visibility, and whether we may change it.
    pub async fn room_history_visibility(
        &self,
        room_id: String,
    ) -> Result<FfiHistoryVisibilityInfo, CoreError> {
        use ruma::events::room::history_visibility::{
            HistoryVisibility, RoomHistoryVisibilityEventContent,
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();

                let visibility =
                    read_state_content::<RoomHistoryVisibilityEventContent>(&matrix_room)
                        .await
                        .map(|content| content.history_visibility)
                        .unwrap_or(HistoryVisibility::Shared);

                let value = match visibility {
                    HistoryVisibility::WorldReadable => FfiHistoryVisibility::WorldReadable,
                    HistoryVisibility::Shared => FfiHistoryVisibility::Shared,
                    HistoryVisibility::Invited => FfiHistoryVisibility::Invited,
                    _ => FfiHistoryVisibility::Joined,
                };

                Ok(FfiHistoryVisibilityInfo {
                    value,
                    can_change: can_send_state(
                        &matrix_room,
                        ruma::events::StateEventType::RoomHistoryVisibility,
                    )
                    .await,
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Change the room's history visibility.
    pub async fn set_room_history_visibility(
        &self,
        room_id: String,
        value: FfiHistoryVisibility,
    ) -> Result<(), CoreError> {
        use ruma::events::room::history_visibility::{
            HistoryVisibility, RoomHistoryVisibilityEventContent,
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                let visibility = match value {
                    FfiHistoryVisibility::WorldReadable => HistoryVisibility::WorldReadable,
                    FfiHistoryVisibility::Shared => HistoryVisibility::Shared,
                    FfiHistoryVisibility::Invited => HistoryVisibility::Invited,
                    FfiHistoryVisibility::Joined => HistoryVisibility::Joined,
                };

                room.matrix_room()
                    .send_state_event(RoomHistoryVisibilityEventContent::new(visibility))
                    .await
                    .map(|_| ())
                    .map_err(|send_error| CoreError::Failed {
                        msg: format!("Could not change the history visibility: {send_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// The room's addresses: canonical and alternative public aliases,
    /// the aliases registered on this homeserver, and whether the room
    /// is published in the server's directory.
    pub async fn room_addresses(&self, room_id: String) -> Result<FfiRoomAddresses, CoreError> {
        use ruma::{
            api::client::{
                directory::get_room_visibility,
                room::{Visibility, aliases},
            },
            events::room::canonical_alias::RoomCanonicalAliasEventContent,
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();
                let client = session.client();

                let content = read_state_content::<RoomCanonicalAliasEventContent>(&matrix_room)
                    .await
                    .unwrap_or_default();

                let local = client
                    .send(aliases::v3::Request::new(room_id.clone()))
                    .await
                    .map(|response| {
                        response
                            .aliases
                            .into_iter()
                            .map(|alias| alias.to_string())
                            .collect()
                    })
                    .unwrap_or_default();

                let published = client
                    .send(get_room_visibility::v3::Request::new(room_id.clone()))
                    .await
                    .map(|response| response.visibility == Visibility::Public)
                    .unwrap_or(false);

                Ok(FfiRoomAddresses {
                    canonical: content.alias.map(|alias| alias.to_string()),
                    alt: content
                        .alt_aliases
                        .iter()
                        .map(|alias| alias.to_string())
                        .collect(),
                    local,
                    published,
                    can_change: can_send_state(
                        &matrix_room,
                        ruma::events::StateEventType::RoomCanonicalAlias,
                    )
                    .await,
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Change one aspect of the room's addresses. The canonical-alias
    /// event is read, mutated exactly as the application's aliases model
    /// does, and sent back whole; registering and unregistering local
    /// aliases go through the directory endpoints; publishing flips the
    /// room's directory visibility.
    pub async fn set_room_address(
        &self,
        room_id: String,
        action: FfiAddressAction,
    ) -> Result<(), CoreError> {
        use ruma::{
            api::client::{
                alias::{create_alias, delete_alias},
                directory::set_room_visibility,
                room::Visibility,
            },
            events::room::canonical_alias::RoomCanonicalAliasEventContent,
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();
                let client = session.client();

                let parse_alias = |alias: &str| {
                    ruma::RoomAliasId::parse(alias).map_err(|_| CoreError::Failed {
                        msg: "Invalid address".to_owned(),
                    })
                };

                match action {
                    FfiAddressAction::Publish { published } => {
                        let visibility = if published {
                            Visibility::Public
                        } else {
                            Visibility::Private
                        };
                        client
                            .send(set_room_visibility::v3::Request::new(
                                room_id.clone(),
                                visibility,
                            ))
                            .await
                            .map(|_| ())
                            .map_err(|send_error| CoreError::Failed {
                                msg: format!("Could not change the visibility: {send_error}"),
                            })
                    }
                    FfiAddressAction::RegisterLocal { alias } => {
                        let alias = parse_alias(&alias)?;
                        client
                            .send(create_alias::v3::Request::new(alias, room_id.clone()))
                            .await
                            .map(|_| ())
                            .map_err(|send_error| CoreError::Failed {
                                msg: format!("Could not register the address: {send_error}"),
                            })
                    }
                    FfiAddressAction::UnregisterLocal { alias } => {
                        let alias = parse_alias(&alias)?;
                        client
                            .send(delete_alias::v3::Request::new(alias))
                            .await
                            .map(|_| ())
                            .map_err(|send_error| CoreError::Failed {
                                msg: format!("Could not unregister the address: {send_error}"),
                            })
                    }
                    FfiAddressAction::SetCanonical { alias } => {
                        let alias = parse_alias(&alias)?;
                        let mut content =
                            read_state_content::<RoomCanonicalAliasEventContent>(&matrix_room)
                                .await
                                .unwrap_or_default();
                        if content.alias.as_ref() == Some(&alias) {
                            return Ok(());
                        }
                        if let Some(pos) = content.alt_aliases.iter().position(|a| *a == alias) {
                            content.alt_aliases.remove(pos);
                        }
                        if let Some(old) = content.alias.replace(alias)
                            && !content.alt_aliases.contains(&old)
                        {
                            content.alt_aliases.push(old);
                        }
                        send_canonical(&matrix_room, content).await
                    }
                    FfiAddressAction::RemoveCanonical { alias } => {
                        let alias = parse_alias(&alias)?;
                        let mut content =
                            read_state_content::<RoomCanonicalAliasEventContent>(&matrix_room)
                                .await
                                .unwrap_or_default();
                        if content.alias.take().is_none_or(|a| a != alias) {
                            return Ok(());
                        }
                        send_canonical(&matrix_room, content).await
                    }
                    FfiAddressAction::AddAlt { alias } => {
                        let alias = parse_alias(&alias)?;
                        // The alias must exist and point at this room.
                        let resolved =
                            client
                                .resolve_room_alias(&alias)
                                .await
                                .map_err(|resolve_error| CoreError::Failed {
                                    msg: format!("This address is not registered: {resolve_error}"),
                                })?;
                        if resolved.room_id != room_id {
                            return Err(CoreError::Failed {
                                msg: "This address points to another room".to_owned(),
                            });
                        }
                        let mut content =
                            read_state_content::<RoomCanonicalAliasEventContent>(&matrix_room)
                                .await
                                .unwrap_or_default();
                        if content.alias.as_ref() == Some(&alias)
                            || content.alt_aliases.contains(&alias)
                        {
                            return Ok(());
                        }
                        content.alt_aliases.push(alias);
                        send_canonical(&matrix_room, content).await
                    }
                    FfiAddressAction::RemoveAlt { alias } => {
                        let alias = parse_alias(&alias)?;
                        let mut content =
                            read_state_content::<RoomCanonicalAliasEventContent>(&matrix_room)
                                .await
                                .unwrap_or_default();
                        let Some(pos) = content.alt_aliases.iter().position(|a| *a == alias) else {
                            return Ok(());
                        };
                        content.alt_aliases.remove(pos);
                        send_canonical(&matrix_room, content).await
                    }
                }
            })
            .await
            .expect("task was not aborted")
    }

    /// The room's server ACL. An absent event reads as the open default:
    /// every server allowed, IP literals too.
    pub async fn room_server_acl(&self, room_id: String) -> Result<FfiServerAcl, CoreError> {
        use ruma::events::room::server_acl::RoomServerAclEventContent;

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();

                let content = read_state_content::<RoomServerAclEventContent>(&matrix_room)
                    .await
                    .unwrap_or_else(|| {
                        RoomServerAclEventContent::new(true, vec!["*".to_owned()], Vec::new())
                    });

                Ok(FfiServerAcl {
                    allow: content.allow,
                    deny: content.deny,
                    allow_ip_literals: content.allow_ip_literals,
                    can_change: can_send_state(
                        &matrix_room,
                        ruma::events::StateEventType::RoomServerAcl,
                    )
                    .await,
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Replace the room's server ACL.
    pub async fn set_room_server_acl(
        &self,
        room_id: String,
        allow: Vec<String>,
        deny: Vec<String>,
        allow_ip_literals: bool,
    ) -> Result<(), CoreError> {
        use ruma::events::room::server_acl::RoomServerAclEventContent;

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                room.matrix_room()
                    .send_state_event(RoomServerAclEventContent::new(
                        allow_ip_literals,
                        allow,
                        deny,
                    ))
                    .await
                    .map(|_| ())
                    .map_err(|send_error| CoreError::Failed {
                        msg: format!("Could not change the server ACL: {send_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// The room's permission thresholds — the application's permissions
    /// page, flattened: role defaults, action levels, and the per-event
    /// overrides it exposes.
    pub async fn room_permissions_matrix(
        &self,
        room_id: String,
    ) -> Result<FfiPowerLevelsMatrix, CoreError> {
        use ruma::events::TimelineEventType;

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();

                let power_levels =
                    matrix_room
                        .power_levels()
                        .await
                        .map_err(|levels_error| CoreError::Failed {
                            msg: format!("Could not read the permissions: {levels_error}"),
                        })?;

                let events_default: i64 = power_levels.events_default.into();
                let state_default: i64 = power_levels.state_default.into();
                let event_level = |event_type: TimelineEventType, default: i64| -> i64 {
                    power_levels
                        .events
                        .get(&event_type)
                        .map_or(default, |level| (*level).into())
                };

                Ok(FfiPowerLevelsMatrix {
                    users_default: power_levels.users_default.into(),
                    events_default,
                    state_default,
                    invite: power_levels.invite.into(),
                    kick: power_levels.kick.into(),
                    ban: power_levels.ban.into(),
                    redact_others: power_levels.redact.into(),
                    redact_own: event_level(TimelineEventType::RoomRedaction, events_default),
                    notify_room: power_levels.notifications.room.into(),
                    name: event_level(TimelineEventType::RoomName, state_default),
                    topic: event_level(TimelineEventType::RoomTopic, state_default),
                    avatar: event_level(TimelineEventType::RoomAvatar, state_default),
                    aliases: event_level(TimelineEventType::RoomCanonicalAlias, state_default),
                    history_visibility: event_level(
                        TimelineEventType::RoomHistoryVisibility,
                        state_default,
                    ),
                    encryption: event_level(TimelineEventType::RoomEncryption, state_default),
                    power_levels: event_level(TimelineEventType::RoomPowerLevels, state_default),
                    server_acl: event_level(TimelineEventType::RoomServerAcl, state_default),
                    upgrade: event_level(TimelineEventType::RoomTombstone, state_default),
                    can_change: can_send_state(
                        &matrix_room,
                        ruma::events::StateEventType::RoomPowerLevels,
                    )
                    .await,
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Replace the room's permission thresholds, collected exactly as
    /// the application's permissions page collects them: overrides that
    /// match their default are elided, redacting one's own messages can
    /// never need more power than redacting others', and the per-user
    /// levels are carried over untouched.
    pub async fn set_room_permissions_matrix(
        &self,
        room_id: String,
        matrix: FfiPowerLevelsMatrix,
    ) -> Result<(), CoreError> {
        use ruma::{
            Int,
            events::{TimelineEventType, room::power_levels::RoomPowerLevelsEventContent},
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();

                let mut power_levels =
                    matrix_room
                        .power_levels()
                        .await
                        .map_err(|levels_error| CoreError::Failed {
                            msg: format!("Could not read the permissions: {levels_error}"),
                        })?;

                let int = Int::new_saturating;
                power_levels.users_default = int(matrix.users_default);
                power_levels.events_default = int(matrix.events_default);
                power_levels.state_default = int(matrix.state_default);
                power_levels.invite = int(matrix.invite);
                power_levels.kick = int(matrix.kick);
                power_levels.ban = int(matrix.ban);
                power_levels.redact = int(matrix.redact_others);
                power_levels.notifications.room = int(matrix.notify_room);

                let mut set_event = |event_type: TimelineEventType, value: i64, default: i64| {
                    if value == default {
                        power_levels.events.remove(&event_type);
                    } else {
                        power_levels.events.insert(event_type, int(value));
                    }
                };

                // Redacting our own events is sending a redaction event.
                let redact_own = matrix.redact_own.min(matrix.redact_others);
                set_event(
                    TimelineEventType::RoomRedaction,
                    redact_own,
                    matrix.events_default,
                );
                set_event(
                    TimelineEventType::RoomName,
                    matrix.name,
                    matrix.state_default,
                );
                set_event(
                    TimelineEventType::RoomTopic,
                    matrix.topic,
                    matrix.state_default,
                );
                set_event(
                    TimelineEventType::RoomAvatar,
                    matrix.avatar,
                    matrix.state_default,
                );
                set_event(
                    TimelineEventType::RoomCanonicalAlias,
                    matrix.aliases,
                    matrix.state_default,
                );
                set_event(
                    TimelineEventType::RoomHistoryVisibility,
                    matrix.history_visibility,
                    matrix.state_default,
                );
                set_event(
                    TimelineEventType::RoomEncryption,
                    matrix.encryption,
                    matrix.state_default,
                );
                set_event(
                    TimelineEventType::RoomPowerLevels,
                    matrix.power_levels,
                    matrix.state_default,
                );
                set_event(
                    TimelineEventType::RoomServerAcl,
                    matrix.server_acl,
                    matrix.state_default,
                );
                set_event(
                    TimelineEventType::RoomTombstone,
                    matrix.upgrade,
                    matrix.state_default,
                );

                let content =
                    RoomPowerLevelsEventContent::try_from(power_levels).map_err(|error| {
                        CoreError::Failed {
                            msg: format!("Could not build the permissions: {error}"),
                        }
                    })?;

                matrix_room
                    .send_state_event(content)
                    .await
                    .map(|_| ())
                    .map_err(|send_error| CoreError::Failed {
                        msg: format!("Could not save the permissions: {send_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// The room versions an upgrade could go to, per the application's
    /// rules: stable versions at or above both the current and the
    /// server's default, the current and default versions listed as
    /// unstable when they are, sorted, with the suggested pick marked.
    pub async fn room_upgrade_info(&self, room_id: String) -> Result<FfiUpgradeInfo, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let matrix_room = room.matrix_room().clone();

                let room_info = matrix_room.clone_info();
                let current_version = room_info
                    .create()
                    .map(|create| create.room_version.clone())
                    .ok_or_else(|| CoreError::Failed {
                        msg: "The room has no create event".to_owned(),
                    })?;

                let capability = session
                    .client()
                    .homeserver_capabilities()
                    .room_versions()
                    .await
                    .map_err(|capability_error| CoreError::Failed {
                        msg: format!("Could not ask the server: {capability_error}"),
                    })?;

                let can_upgrade = !matrix_room.is_direct().await.unwrap_or(false)
                    && matrix_room.successor_room().is_none()
                    && can_send_state(&matrix_room, ruma::events::StateEventType::RoomTombstone)
                        .await;

                Ok(build_upgrade_info(
                    &current_version,
                    &capability,
                    can_upgrade,
                ))
            })
            .await
            .expect("task was not aborted")
    }

    /// Upgrade the room to the given version.
    pub async fn upgrade_room(
        &self,
        room_id: String,
        new_version: String,
    ) -> Result<(), CoreError> {
        use ruma::api::client::room::upgrade_room;

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let new_version =
                    ruma::RoomVersionId::try_from(new_version.as_str()).map_err(|_| {
                        CoreError::Failed {
                            msg: "Invalid room version".to_owned(),
                        }
                    })?;

                session
                    .client()
                    .send(upgrade_room::v3::Request::new(room_id, new_version))
                    .await
                    .map(|_| ())
                    .map_err(|upgrade_error| CoreError::Failed {
                        msg: format!("Could not upgrade the room: {upgrade_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Kick the given user from the given room.
    pub async fn kick_user(
        &self,
        room_id: String,
        user_id: String,
        reason: Option<String>,
    ) -> Result<(), CoreError> {
        self.with_room_user(room_id, user_id, move |room, user| async move {
            room.kick_user(&user, reason.as_deref())
                .await
                .map_err(|kick_error| CoreError::Failed {
                    msg: format!("Could not kick: {kick_error}"),
                })
        })
        .await
    }

    /// Ban the given user from the given room.
    pub async fn ban_user(
        &self,
        room_id: String,
        user_id: String,
        reason: Option<String>,
    ) -> Result<(), CoreError> {
        self.with_room_user(room_id, user_id, move |room, user| async move {
            room.ban_user(&user, reason.as_deref())
                .await
                .map_err(|ban_error| CoreError::Failed {
                    msg: format!("Could not ban: {ban_error}"),
                })
        })
        .await
    }

    /// Change the given user's power level in the given room.
    pub async fn set_member_power_level(
        &self,
        room_id: String,
        user_id: String,
        level: i64,
    ) -> Result<(), CoreError> {
        self.with_room_user(room_id, user_id, move |room, user| async move {
            let level = ruma::Int::try_from(level).unwrap_or_default();
            room.update_power_levels(vec![(&user, level)])
                .await
                .map(|_| ())
                .map_err(|power_error| CoreError::Failed {
                    msg: format!("Could not change the role: {power_error}"),
                })
        })
        .await
    }

    /// Export the room keys to an encrypted file at the given path.
    pub async fn export_room_keys(
        &self,
        path: String,
        passphrase: String,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                session
                    .client()
                    .encryption()
                    .export_room_keys(path.into(), &passphrase, |_| true)
                    .await
                    .map_err(|export_error| CoreError::Failed {
                        msg: format!("Could not export the keys: {export_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Import room keys from an encrypted export at the given path.
    ///
    /// Returns how many keys came in.
    pub async fn import_room_keys(
        &self,
        path: String,
        passphrase: String,
    ) -> Result<u64, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                session
                    .client()
                    .encryption()
                    .import_room_keys(path.into(), &passphrase)
                    .await
                    .map(|counts| counts.imported_count as u64)
                    .map_err(|import_error| CoreError::Failed {
                        msg: format!("Could not import the keys: {import_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Send a recorded voice message.
    pub async fn send_voice_message(
        &self,
        room_id: String,
        file_path: String,
        mime_type: String,
        duration_ms: u64,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let mime = mime_type
                    .parse::<mime::Mime>()
                    .unwrap_or(mime::APPLICATION_OCTET_STREAM);

                let size = std::fs::metadata(&file_path)
                    .ok()
                    .map(|metadata| metadata.len());
                check_upload_size(&session.client(), size).await?;

                room.live_timeline()
                    .send_voice(file_path.into(), mime, duration_ms)
                    .await
                    .map_err(|()| CoreError::Failed {
                        msg: "Could not send the voice message".to_owned(),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// The sticker packs on the account, as the application resolves
    /// them: the packs of Commune's own packs room first, then every
    /// room pack enabled globally. Stable event names are preferred,
    /// the unstable `im.ponies` names read as fallback, and personal
    /// `im.ponies.user_emotes` packs from other clients come along too.
    pub async fn sticker_packs(&self) -> Vec<FfiStickerPack> {
        let Some(session) = self.first_ready_session() else {
            return Vec::new();
        };

        RUNTIME
            .spawn(collect_image_packs(session, "sticker"))
            .await
            .expect("task was not aborted")
    }

    /// The emoticon images of the same packs, for the composer's
    /// `:shortcode:` completion.
    pub async fn emoticon_packs(&self) -> Vec<FfiStickerPack> {
        let Some(session) = self.first_ready_session() else {
            return Vec::new();
        };

        RUNTIME
            .spawn(collect_image_packs(session, "emoticon"))
            .await
            .expect("task was not aborted")
    }

    /// Send the user's location to the room, as the application's
    /// message toolbar does.
    pub async fn send_location(&self, room_id: String, geo_uri: String) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                room.live_timeline()
                    .send_location(geo_uri)
                    .await
                    .map_err(|()| CoreError::Failed {
                        msg: "Could not send the location".to_owned(),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Send a sticker from a pack.
    pub async fn send_sticker(
        &self,
        room_id: String,
        sticker: FfiSticker,
    ) -> Result<(), CoreError> {
        use ruma::events::{room::ImageInfo, sticker::StickerEventContent};

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let url = ruma::OwnedMxcUri::from(sticker.url);
                // The event carries the pack image's declared `info`
                // whole, as the application's `sticker_content` does.
                let info = sticker
                    .info_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<ImageInfo>(json).ok())
                    .unwrap_or_default();

                room.matrix_room()
                    .send(StickerEventContent::new(sticker.body, info, url))
                    .await
                    .map(|_| ())
                    .map_err(|send_error| CoreError::Failed {
                        msg: format!("Could not send the sticker: {send_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Fetch the media behind a plain `mxc:` URI into a file, returning
    /// its path — sticker previews, mostly.
    pub async fn get_mxc_media(&self, mxc: String) -> Option<String> {
        let session = self.first_ready_session()?;

        RUNTIME
            .spawn(async move {
                let request = matrix_sdk::media::MediaRequestParameters {
                    source: ruma::events::room::MediaSource::Plain(ruma::OwnedMxcUri::from(mxc)),
                    format: matrix_sdk::media::MediaFormat::File,
                };
                crate::matrix::media::get_media_file(&session.client(), request)
                    .await
                    .map(|path| path.to_string_lossy().into_owned())
            })
            .await
            .expect("task was not aborted")
    }

    /// A matrix.to link to the given event, with the routing the SDK
    /// computes — what the application's Copy Message Link puts on the
    /// clipboard.
    pub async fn event_permalink(
        &self,
        room_id: String,
        event_id: String,
    ) -> Result<String, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let event_id = ruma::EventId::parse(&event_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid event ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                room.matrix_room()
                    .matrix_to_event_permalink(event_id)
                    .await
                    .map(|uri| uri.to_string())
                    .map_err(|link_error| CoreError::Failed {
                        msg: format!("Could not build the link: {link_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// The raw JSON of the given event, pretty-printed — the properties
    /// dialog's source view.
    pub async fn event_source(
        &self,
        room_id: String,
        event_id: String,
    ) -> Result<String, CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let event_id = ruma::EventId::parse(&event_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid event ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                let event =
                    room.matrix_room()
                        .event(&event_id, None)
                        .await
                        .map_err(|event_error| CoreError::Failed {
                            msg: format!("Could not fetch the event: {event_error}"),
                        })?;

                let value =
                    event
                        .raw()
                        .deserialize_as::<serde_json::Value>()
                        .map_err(|json_error| CoreError::Failed {
                            msg: format!("Could not read the event: {json_error}"),
                        })?;
                serde_json::to_string_pretty(&value).map_err(|json_error| CoreError::Failed {
                    msg: format!("Could not render the event: {json_error}"),
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Report the given event to the homeserver administrator, as the
    /// application's report action does.
    pub async fn report_event(
        &self,
        room_id: String,
        event_id: String,
        reason: Option<String>,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let event_id = ruma::EventId::parse(&event_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid event ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                room.matrix_room()
                    .report_content(event_id, reason)
                    .await
                    .map(|_| ())
                    .map_err(|report_error| CoreError::Failed {
                        msg: format!("Could not report the event: {report_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Send the given event's content to another room, verbatim.
    ///
    /// NOTE: the application's Forward menu item is a stub (its action
    /// is never registered), so there is no wire precedent to mirror —
    /// re-sending the original content as a fresh event of the same
    /// type is the design here.
    pub async fn forward_event(
        &self,
        room_id: String,
        event_id: String,
        target_room_id: String,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let event_id = ruma::EventId::parse(&event_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid event ID".to_owned(),
                })?;
                let target_room_id =
                    ruma::RoomId::parse(&target_room_id).map_err(|_| CoreError::Failed {
                        msg: "Invalid room ID".to_owned(),
                    })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;
                let target =
                    session
                        .room_list()
                        .get(&target_room_id)
                        .ok_or_else(|| CoreError::Failed {
                            msg: "Unknown room".to_owned(),
                        })?;

                let event =
                    room.matrix_room()
                        .event(&event_id, None)
                        .await
                        .map_err(|event_error| CoreError::Failed {
                            msg: format!("Could not fetch the event: {event_error}"),
                        })?;
                let value =
                    event
                        .raw()
                        .deserialize_as::<serde_json::Value>()
                        .map_err(|json_error| CoreError::Failed {
                            msg: format!("Could not read the event: {json_error}"),
                        })?;
                let event_type = value
                    .get("type")
                    .and_then(|event_type| event_type.as_str())
                    .ok_or_else(|| CoreError::Failed {
                        msg: "The event has no type".to_owned(),
                    })?
                    .to_owned();
                let content = value
                    .get("content")
                    .cloned()
                    .ok_or_else(|| CoreError::Failed {
                        msg: "The event has no content".to_owned(),
                    })?;
                let raw_content = serde_json::from_value::<
                    ruma::serde::Raw<ruma::events::AnyMessageLikeEventContent>,
                >(content)
                .map_err(|json_error| CoreError::Failed {
                    msg: format!("Could not carry the content over: {json_error}"),
                })?;

                target
                    .matrix_room()
                    .send_raw(&event_type, raw_content)
                    .await
                    .map(|_| ())
                    .map_err(|send_error| CoreError::Failed {
                        msg: format!("Could not forward the message: {send_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Discard a message that never sent: redact its local echo, as the
    /// application's cancel-send does.
    pub async fn discard_local_echo(
        &self,
        room_id: String,
        unique_id: String,
    ) -> Result<(), CoreError> {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                room.live_timeline()
                    .discard_local_echo(&unique_id)
                    .await
                    .map_err(|()| CoreError::Failed {
                        msg: "Could not discard the message".to_owned(),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Retry the messages that failed to send, by waking the send queue
    /// back up.
    ///
    /// The queue disables itself on a recoverable error; re-enabling it
    /// respawns the sending tasks for everything still unsent.
    pub async fn retry_sends(&self) {
        let Some(session) = self.first_ready_session() else {
            return;
        };

        RUNTIME
            .spawn(async move {
                let client = session.client();
                client.send_queue().set_enabled(true).await;
                client
                    .send_queue()
                    .respawn_tasks_for_rooms_with_unsent_requests()
                    .await;
            })
            .await
            .expect("task was not aborted");
    }

    /// Point the homeserver's push at the given gateway.
    ///
    /// `gateway_url` is the Matrix push gateway (`.../_matrix/push/v1/notify`)
    /// and `pushkey` the `UnifiedPush` endpoint that identifies this device.
    pub async fn set_push_gateway(
        &self,
        gateway_url: String,
        pushkey: String,
    ) -> Result<(), CoreError> {
        use ruma::{
            api::client::push::{Pusher, PusherIds, PusherInit, PusherKind, set_pusher},
            push::HttpPusherData,
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let pusher: Pusher = PusherInit {
                    ids: PusherIds::new(pushkey, config::app_id().to_owned()),
                    kind: PusherKind::Http(HttpPusherData::new(gateway_url)),
                    app_display_name: "Commune".to_owned(),
                    device_display_name: "Commune on Android".to_owned(),
                    profile_tag: None,
                    lang: "en".to_owned(),
                }
                .into();

                session
                    .client()
                    .send(set_pusher::v3::Request::post(pusher))
                    .await
                    .map(|_| ())
                    .map_err(|pusher_error| CoreError::Failed {
                        msg: format!("Could not set the pusher: {pusher_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// Remove the pusher with the given pushkey, so the homeserver stops
    /// pushing to it.
    pub async fn remove_push_gateway(&self, pushkey: String) -> Result<(), CoreError> {
        use ruma::api::client::push::{PusherIds, set_pusher};

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let ids = PusherIds::new(pushkey, config::app_id().to_owned());
                session
                    .client()
                    .send(set_pusher::v3::Request::delete(ids))
                    .await
                    .map(|_| ())
                    .map_err(|pusher_error| CoreError::Failed {
                        msg: format!("Could not remove the pusher: {pusher_error}"),
                    })
            })
            .await
            .expect("task was not aborted")
    }

    /// One page of the public room directory, optionally filtered by a
    /// search term, continuing from `since` when given.
    pub async fn explore_rooms(
        &self,
        search: Option<String>,
        since: Option<String>,
    ) -> Result<FfiPublicRoomPage, CoreError> {
        use ruma::{
            api::client::directory::get_public_rooms_filtered,
            assign,
            directory::{Filter, RoomNetwork},
            uint,
        };

        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let request = assign!(get_public_rooms_filtered::v3::Request::new(), {
                    limit: Some(uint!(20)),
                    since,
                    room_network: RoomNetwork::Matrix,
                    filter: assign!(Filter::new(), {
                        generic_search_term: search.filter(|term| !term.is_empty()),
                    }),
                });

                let response = session
                    .client()
                    .public_rooms_filtered(request)
                    .await
                    .map_err(|explore_error| CoreError::Failed {
                        msg: format!("Could not load the directory: {explore_error}"),
                    })?;

                let rooms = response
                    .chunk
                    .into_iter()
                    .map(|chunk| {
                        let is_joined = session.room_list().get(&chunk.room_id).is_some();
                        FfiPublicRoom {
                            room_id: chunk.room_id.to_string(),
                            name: chunk.name,
                            topic: chunk.topic,
                            alias: chunk.canonical_alias.map(|alias| alias.to_string()),
                            joined_members: chunk.num_joined_members.into(),
                            is_joined,
                        }
                    })
                    .collect();

                Ok(FfiPublicRoomPage {
                    rooms,
                    next_batch: response.next_batch,
                })
            })
            .await
            .expect("task was not aborted")
    }

    /// Fetch the media of a history event into a file, returning its path.
    pub async fn get_history_media(&self, room_id: String, event_id: String) -> Option<String> {
        use ruma::events::room::message::MessageType;

        let session = self.first_ready_session()?;

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).ok()?;
                let event_id = ruma::EventId::parse(&event_id).ok()?;
                let room = session.room_list().get(&room_id)?;

                let event = room.matrix_room().event(&event_id, None).await.ok()?;
                let message = crate::matrix::original_message_event_from_raw(event.raw())?;
                let source = match &message.content.msgtype {
                    MessageType::Image(image) => image.source.clone(),
                    MessageType::Video(video) => video.source.clone(),
                    MessageType::Audio(audio) => audio.source.clone(),
                    MessageType::File(file) => file.source.clone(),
                    _ => return None,
                };

                let request = matrix_sdk::media::MediaRequestParameters {
                    source,
                    format: matrix_sdk::media::MediaFormat::File,
                };

                crate::matrix::media::get_media_file(&session.client(), request)
                    .await
                    .map(|path| path.to_string_lossy().into_owned())
            })
            .await
            .expect("task was not aborted")
    }
}

/// Build the FFI view of one media-history event, if the raw event is a
/// media message.
fn ffi_history_event(
    raw: &ruma::serde::Raw<ruma::events::AnySyncTimelineEvent>,
) -> Option<FfiHistoryEvent> {
    use ruma::events::room::message::MessageType;

    let message = crate::matrix::original_message_event_from_raw(raw)?;

    let (kind, body, mime_type, size) = match &message.content.msgtype {
        MessageType::Image(image) => (
            FfiHistoryKind::Media,
            image.filename().to_owned(),
            image.info.as_ref().and_then(|info| info.mimetype.clone()),
            image.info.as_ref().and_then(|info| info.size),
        ),
        MessageType::Video(video) => (
            FfiHistoryKind::Media,
            video.filename().to_owned(),
            video.info.as_ref().and_then(|info| info.mimetype.clone()),
            video.info.as_ref().and_then(|info| info.size),
        ),
        MessageType::Audio(audio) => (
            FfiHistoryKind::Audio,
            audio.filename().to_owned(),
            audio.info.as_ref().and_then(|info| info.mimetype.clone()),
            audio.info.as_ref().and_then(|info| info.size),
        ),
        MessageType::File(file) => (
            FfiHistoryKind::File,
            file.filename().to_owned(),
            file.info.as_ref().and_then(|info| info.mimetype.clone()),
            file.info.as_ref().and_then(|info| info.size),
        ),
        _ => return None,
    };

    Some(FfiHistoryEvent {
        event_id: message.event_id.to_string(),
        sender: message.sender.to_string(),
        timestamp: message.origin_server_ts.0.into(),
        kind,
        body,
        mime_type,
        size: size.map(u64::from),
        is_video: matches!(&message.content.msgtype, MessageType::Video(_)),
    })
}

/// What kind of history page an event belongs on.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum FfiHistoryKind {
    /// An image or a video, for the media grid.
    Media,
    /// A generic file.
    File,
    /// An audio file.
    Audio,
}

/// One event of the media history.
#[derive(uniffi::Record)]
pub struct FfiHistoryEvent {
    /// The ID of the event.
    pub event_id: String,
    /// The user that sent it.
    pub sender: String,
    /// When it was sent, in milliseconds since the epoch.
    pub timestamp: u64,
    /// The page it belongs on.
    pub kind: FfiHistoryKind,
    /// The filename, or the body when no filename travelled.
    pub body: String,
    /// The MIME type, when the sender declared one.
    pub mime_type: Option<String>,
    /// The size in bytes, when the sender declared one.
    pub size: Option<u64>,
    /// Whether a Media event is a video rather than an image.
    pub is_video: bool,
}

/// One of the account's sessions.
#[derive(uniffi::Record)]
pub struct FfiDevice {
    /// The ID of the device.
    pub device_id: String,
    /// Its display name, when one is set.
    pub display_name: Option<String>,
    /// Whether it is this session.
    pub is_current: bool,
    /// Whether cross-signing vouches for it.
    pub is_verified: bool,
    /// When it was last seen, in milliseconds since the epoch.
    pub last_seen_ts: Option<u64>,
    /// The IP it was last seen from.
    pub last_seen_ip: Option<String>,
}

/// How a room notifies.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum FfiRoomNotificationMode {
    /// Whatever the account's defaults say.
    Default,
    /// Every message.
    All,
    /// Mentions and keywords only.
    MentionsOnly,
    /// Nothing.
    Mute,
}

/// A user the composer mentions, by the display name that stands for
/// them in the text.
#[derive(uniffi::Record)]
pub struct FfiMention {
    /// The user ID of the mention.
    pub user_id: String,
    /// The display name as it appears in the body, after an `@`.
    pub display_name: String,
}

/// A sticker pack from the account's image packs.
#[derive(uniffi::Record)]
pub struct FfiStickerPack {
    /// The display name of the pack.
    pub name: String,
    /// The stickers of the pack.
    pub stickers: Vec<FfiSticker>,
}

/// One sticker of a pack.
#[derive(uniffi::Record, Clone)]
pub struct FfiSticker {
    /// The shortcode that identifies the image in its pack.
    pub shortcode: String,
    /// The description, sent as the event body.
    pub body: String,
    /// The `mxc:` URI of the image.
    pub url: String,
    /// The width in pixels, when the pack declares one.
    pub width: Option<u32>,
    /// The height in pixels, when the pack declares one.
    pub height: Option<u32>,
    /// The MIME type, when the pack declares one.
    pub mime_type: Option<String>,
    /// The pack image's raw `info` JSON, sent whole with the sticker —
    /// the application sends everything the pack declared.
    pub info_json: Option<String>,
}

/// Walk the account's image packs — Commune's own packs room, the
/// room packs enabled globally, and personal `im.ponies` packs — and
/// keep the images whose usage allows the given one.
async fn collect_image_packs(session: Session, usage: &'static str) -> Vec<FfiStickerPack> {
    let client = session.client();
    let mut packs = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    // Commune's own packs room.
    if let Some(value) =
        read_account_data(&client, "io.github.steeb_k.Commune.image_packs_room").await
        && let Some(room_id) = value
            .pointer("/content/room_id")
            .or_else(|| value.get("room_id"))
            .and_then(|id| id.as_str())
        && let Ok(room_id) = ruma::RoomId::parse(room_id)
        && let Some(room) = client.get_room(&room_id)
        && let Ok(events) = room.get_state_events("m.room.image_pack".into()).await
    {
        for event in events {
            let matrix_sdk::deserialized_responses::RawAnySyncOrStrippedState::Sync(raw) = event
            else {
                continue;
            };
            let Ok(value) = raw.deserialize_as::<serde_json::Value>() else {
                continue;
            };
            let state_key = value
                .get("state_key")
                .and_then(|key| key.as_str())
                .unwrap_or_default()
                .to_owned();
            if let Some(pack) =
                parse_sticker_pack(value.get("content").unwrap_or(&value), &state_key, usage)
            {
                seen.insert((room_id.to_string(), state_key));
                packs.push(pack);
            }
        }
    }

    // Room packs enabled globally, stable name first.
    collect_enabled_packs(&client, &mut seen, &mut packs, usage).await;

    // Personal packs other clients keep in account data.
    if let Some(value) = read_account_data(&client, "im.ponies.user_emotes").await
        && let Some(pack) =
            parse_sticker_pack(value.get("content").unwrap_or(&value), "My Stickers", usage)
    {
        packs.push(pack);
    }

    packs
}

/// Collect the room packs the account enabled globally — the stable
/// `m.image_pack.rooms` account data, the unstable `im.ponies` names as
/// fallback — into `packs`, skipping the (room, key) pairs already seen.
async fn collect_enabled_packs(
    client: &matrix_sdk::Client,
    seen: &mut std::collections::HashSet<(String, String)>,
    packs: &mut Vec<FfiStickerPack>,
    usage: &str,
) {
    let mut enabled = serde_json::Map::new();
    for event_type in ["m.image_pack.rooms", "im.ponies.emote_rooms"] {
        if let Some(value) = read_account_data(client, event_type).await
            && let Some(rooms) = value
                .pointer("/content/rooms")
                .or_else(|| value.get("rooms"))
                .and_then(|rooms| rooms.as_object())
        {
            for (room_id, keys) in rooms {
                enabled
                    .entry(room_id.clone())
                    .or_insert_with(|| keys.clone());
            }
        }
    }

    for (room_id, keys) in &enabled {
        let Ok(room_id) = ruma::RoomId::parse(room_id) else {
            continue;
        };
        let Some(room) = client.get_room(&room_id) else {
            continue;
        };
        let Some(keys) = keys.as_object() else {
            continue;
        };
        for state_key in keys.keys() {
            if seen.contains(&(room_id.to_string(), state_key.clone())) {
                continue;
            }
            for event_type in ["m.room.image_pack", "im.ponies.room_emotes"] {
                if let Ok(Some(
                    matrix_sdk::deserialized_responses::RawAnySyncOrStrippedState::Sync(raw),
                )) = room.get_state_event((*event_type).into(), state_key).await
                    && let Ok(value) = raw.deserialize_as::<serde_json::Value>()
                    && let Some(pack) =
                        parse_sticker_pack(value.get("content").unwrap_or(&value), state_key, usage)
                {
                    seen.insert((room_id.to_string(), state_key.clone()));
                    packs.push(pack);
                    break;
                }
            }
        }
    }
}

/// Ask the homeserver's upload limit before sending, rather than
/// uploading the whole file to be told no at the end, as the
/// application's message toolbar does. The SDK caches the answer after
/// the first ask; when it cannot be had, the upload proceeds and the
/// server stays the judge.
async fn check_upload_size(
    client: &matrix_sdk::Client,
    size: Option<u64>,
) -> Result<(), CoreError> {
    let Some(size) = size else {
        return Ok(());
    };
    if let Ok(max_upload_size) = client.load_or_fetch_max_upload_size().await
        && size > u64::from(max_upload_size)
    {
        return Err(CoreError::Failed {
            msg: format!(
                "This file is too large, the homeserver takes up to {}",
                format_size(u64::from(max_upload_size))
            ),
        });
    }
    Ok(())
}

/// A byte count in decimal units, as the application's toast renders it.
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Read one of the room's state events (empty state key) as its typed
/// content, `None` when it is absent or unreadable.
async fn read_state_content<C>(matrix_room: &matrix_sdk::Room) -> Option<C>
where
    C: ruma::events::StaticEventContent + serde::de::DeserializeOwned,
{
    use matrix_sdk::deserialized_responses::RawAnySyncOrStrippedState;

    let raw = matrix_room
        .get_state_event(C::TYPE.into(), "")
        .await
        .ok()
        .flatten()?;
    let RawAnySyncOrStrippedState::Sync(raw) = raw else {
        return None;
    };
    let value = raw.deserialize_as::<serde_json::Value>().ok()?;
    serde_json::from_value(value.get("content")?.clone()).ok()
}

/// Whether our own user may send the given state event in the room.
async fn can_send_state(
    matrix_room: &matrix_sdk::Room,
    event_type: ruma::events::StateEventType,
) -> bool {
    let own_user_id = matrix_room.own_user_id().to_owned();
    match matrix_room.power_levels().await {
        Ok(power_levels) => power_levels.user_can_send_state(&own_user_id, event_type),
        Err(_) => false,
    }
}

/// The room IDs a restricted join rule allows the members of.
fn allow_room_ids(restricted: &ruma::events::room::join_rules::Restricted) -> Vec<String> {
    restricted
        .allow
        .iter()
        .filter_map(|rule| match rule {
            ruma::events::room::join_rules::AllowRule::RoomMembership(membership) => {
                Some(membership.room_id.to_string())
            }
            _ => None,
        })
        .collect()
}

/// Send the mutated canonical-alias content back whole.
async fn send_canonical(
    matrix_room: &matrix_sdk::Room,
    content: ruma::events::room::canonical_alias::RoomCanonicalAliasEventContent,
) -> Result<(), CoreError> {
    matrix_room
        .send_state_event(content)
        .await
        .map(|_| ())
        .map_err(|send_error| CoreError::Failed {
            msg: format!("Could not update the addresses: {send_error}"),
        })
}

/// Build the upgrade choices from the current version and the server's
/// capabilities, per the application's upgrade dialog: stable versions
/// at or above both the current and the default version, the current
/// and default listed as unstable when they are, sorted, the suggested
/// pick marked.
fn build_upgrade_info(
    current: &ruma::RoomVersionId,
    capability: &ruma::api::client::discovery::get_capabilities::v3::RoomVersionsCapability,
    can_upgrade: bool,
) -> FfiUpgradeInfo {
    use std::cmp::Ordering;

    use ruma::api::client::discovery::get_capabilities::v3::RoomVersionStability;

    let is_stable = |version: &ruma::RoomVersionId| {
        capability
            .available
            .get(version)
            .is_some_and(|stability| *stability == RoomVersionStability::Stable)
    };
    let maximum_stable = capability
        .available
        .iter()
        .filter(|(_, stability)| **stability == RoomVersionStability::Stable)
        .map(|(version, _)| version)
        .max_by(|a, b| cmp_room_versions(a, b));

    let current_is_stable = is_stable(current);
    let default_is_stable = is_stable(&capability.default);
    // The minimum stable version is the highest stable version between
    // the current version and the default version.
    let minimum_stable = match (current_is_stable, default_is_stable) {
        (true, false) => Some(current),
        (false, true) => Some(&capability.default),
        (true, true) => Some(match cmp_room_versions(current, &capability.default) {
            Ordering::Less => &capability.default,
            _ => current,
        }),
        (false, false) => None,
    };
    let selected_version = minimum_stable.unwrap_or(&capability.default).clone();

    let mut stable: Vec<ruma::RoomVersionId> = if let Some(minimum) = minimum_stable {
        capability
            .available
            .iter()
            .filter(|(version, stability)| {
                **stability == RoomVersionStability::Stable
                    && (cmp_room_versions(version, minimum) != Ordering::Less
                        || maximum_stable.is_some_and(|maximum| maximum == *version))
            })
            .map(|(version, _)| version.clone())
            .collect()
    } else {
        maximum_stable.into_iter().cloned().collect()
    };

    let mut unstable = Vec::new();
    if !current_is_stable {
        unstable.push(current.clone());
    }
    if *current != capability.default && !default_is_stable {
        unstable.push(capability.default.clone());
    }

    stable.sort_unstable_by(|a, b| cmp_room_versions(a, b));
    unstable.sort_unstable_by(|a, b| cmp_room_versions(a, b));

    let selected = stable
        .iter()
        .position(|version| *version == selected_version)
        .or_else(|| {
            unstable
                .iter()
                .position(|version| *version == selected_version)
                .map(|pos| stable.len() + pos)
        })
        .unwrap_or(0);

    FfiUpgradeInfo {
        current_version: current.to_string(),
        stable: stable.iter().map(ToString::to_string).collect(),
        unstable: unstable.iter().map(ToString::to_string).collect(),
        selected_index: u32::try_from(selected).unwrap_or(0),
        can_upgrade,
    }
}

/// Order room versions: whole-number versions numerically, before the
/// rest lexicographically — the application's digit-sequence-aware
/// comparison, simplified without changing the order of the versions
/// that exist.
fn cmp_room_versions(lhs: &ruma::RoomVersionId, rhs: &ruma::RoomVersionId) -> std::cmp::Ordering {
    match (lhs.as_str().parse::<u64>(), rhs.as_str().parse::<u64>()) {
        (Ok(lhs_number), Ok(rhs_number)) => lhs_number.cmp(&rhs_number),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        (Err(_), Err(_)) => lhs.as_str().cmp(rhs.as_str()),
    }
}

/// The join rule of a room, as the choice the details page offers.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum FfiJoinRuleValue {
    /// Anyone can join.
    Public,
    /// Only invited users can join.
    Invite,
    /// Users can knock to request an invite.
    Knock,
    /// Members of an allowed room can join.
    Restricted,
    /// Members of an allowed room can join, others can knock.
    KnockRestricted,
    /// A rule this page cannot edit.
    Unsupported,
}

/// A room's join rule with what the room's version supports.
#[derive(uniffi::Record)]
pub struct FfiJoinRuleInfo {
    /// The current rule.
    pub value: FfiJoinRuleValue,
    /// The rooms whose members a restricted rule allows.
    pub allow_room_ids: Vec<String>,
    /// Whether the room's version supports knocking.
    pub supports_knock: bool,
    /// Whether the room's version supports the restricted rule.
    pub supports_restricted: bool,
    /// Whether the room's version supports knock-restricted.
    pub supports_knock_restricted: bool,
    /// Whether we may change the rule.
    pub can_change: bool,
}

/// Who can read a room's history.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum FfiHistoryVisibility {
    /// Anyone, member or not.
    WorldReadable,
    /// Members, for everything sent since they could have known of the
    /// room.
    Shared,
    /// Members, since their invitation.
    Invited,
    /// Members, since they joined.
    Joined,
}

/// A room's history visibility, and whether we may change it.
#[derive(uniffi::Record)]
pub struct FfiHistoryVisibilityInfo {
    /// The current visibility.
    pub value: FfiHistoryVisibility,
    /// Whether we may change it.
    pub can_change: bool,
}

/// A room's addresses, as the details page shows them.
#[derive(uniffi::Record)]
pub struct FfiRoomAddresses {
    /// The canonical (main public) address.
    pub canonical: Option<String>,
    /// The other public addresses.
    pub alt: Vec<String>,
    /// The addresses registered on this homeserver.
    pub local: Vec<String>,
    /// Whether the room is published in the server's directory.
    pub published: bool,
    /// Whether we may change the public addresses.
    pub can_change: bool,
}

/// One edit to a room's addresses.
#[derive(uniffi::Enum)]
pub enum FfiAddressAction {
    /// Publish the room in (or withdraw it from) the directory.
    Publish {
        /// Whether the room should be listed.
        published: bool,
    },
    /// Register an address on this homeserver.
    RegisterLocal {
        /// The address.
        alias: String,
    },
    /// Unregister an address from this homeserver.
    UnregisterLocal {
        /// The address.
        alias: String,
    },
    /// Make an address the canonical one.
    SetCanonical {
        /// The address.
        alias: String,
    },
    /// Remove the canonical address.
    RemoveCanonical {
        /// The address.
        alias: String,
    },
    /// Add a public alternative address.
    AddAlt {
        /// The address.
        alias: String,
    },
    /// Remove a public alternative address.
    RemoveAlt {
        /// The address.
        alias: String,
    },
}

/// A room's server ACL.
#[derive(uniffi::Record)]
pub struct FfiServerAcl {
    /// The allowed server patterns.
    pub allow: Vec<String>,
    /// The denied server patterns.
    pub deny: Vec<String>,
    /// Whether servers named by IP literals are allowed.
    pub allow_ip_literals: bool,
    /// Whether we may change the ACL.
    pub can_change: bool,
}

/// A room's permission thresholds, flattened the way the application's
/// permissions page lays them out.
#[derive(uniffi::Record, Clone)]
pub struct FfiPowerLevelsMatrix {
    /// The default level of a member.
    pub users_default: i64,
    /// The level needed to send messages.
    pub events_default: i64,
    /// The default level needed to change state.
    pub state_default: i64,
    /// The level needed to invite.
    pub invite: i64,
    /// The level needed to kick.
    pub kick: i64,
    /// The level needed to ban.
    pub ban: i64,
    /// The level needed to remove others' messages.
    pub redact_others: i64,
    /// The level needed to remove one's own messages.
    pub redact_own: i64,
    /// The level needed to notify the whole room.
    pub notify_room: i64,
    /// The level needed to change the room name.
    pub name: i64,
    /// The level needed to change the topic.
    pub topic: i64,
    /// The level needed to change the avatar.
    pub avatar: i64,
    /// The level needed to change the addresses.
    pub aliases: i64,
    /// The level needed to change the history visibility.
    pub history_visibility: i64,
    /// The level needed to enable encryption.
    pub encryption: i64,
    /// The level needed to change these permissions.
    pub power_levels: i64,
    /// The level needed to change the server ACL.
    pub server_acl: i64,
    /// The level needed to upgrade the room.
    pub upgrade: i64,
    /// Whether we may change any of this.
    pub can_change: bool,
}

/// The room versions an upgrade could go to.
#[derive(uniffi::Record)]
pub struct FfiUpgradeInfo {
    /// The room's current version.
    pub current_version: String,
    /// The stable versions on offer, sorted.
    pub stable: Vec<String>,
    /// The unstable versions on offer, sorted.
    pub unstable: Vec<String>,
    /// The index of the suggested pick, stable and unstable
    /// concatenated.
    pub selected_index: u32,
    /// Whether we may upgrade this room at all.
    pub can_upgrade: bool,
}

/// The fixed Android redirect URI login flows come back on — the
/// application's own, from its `login/local_server.rs`.
const ANDROID_REDIRECT_URI: &str = "io.github.steeb-k.commune:/oauth2redirect";

/// The application's OAuth 2.0 client registration, exactly as its
/// `client_registration_data` builds it for Android.
fn oauth_client_registration_data() -> matrix_sdk::authentication::oauth::ClientRegistrationData {
    use matrix_sdk::authentication::oauth::registration::{
        ApplicationType, ClientMetadata, Localized, OAuthGrantType,
    };

    let redirect_uris = vec![url::Url::parse(ANDROID_REDIRECT_URI).expect("redirect URI is valid")];
    // matrix.org's authorization server requires the client URI to match
    // the redirect scheme read as reverse DNS.
    let client_uri = url::Url::parse("https://steeb-k.github.io/").expect("client URI is valid");

    let mut client_metadata = ClientMetadata::new(
        ApplicationType::Native,
        vec![OAuthGrantType::AuthorizationCode { redirect_uris }],
        Localized::new(client_uri, None),
    );
    client_metadata.client_name = Some(Localized::new("Commune".to_owned(), None));

    ruma::serde::Raw::new(&client_metadata)
        .expect("client metadata serializes")
        .into()
}

/// What a homeserver offers for logging in.
#[derive(uniffi::Record)]
pub struct FfiLoginMethods {
    /// The resolved homeserver URL after discovery.
    pub homeserver_url: String,
    /// Whether password login is offered.
    pub supports_password: bool,
    /// Whether Matrix SSO is offered.
    pub supports_sso: bool,
    /// Whether the homeserver speaks the OAuth 2.0 API (which then
    /// replaces the Matrix flows).
    pub supports_oauth: bool,
}

/// The server-side session of a password reset, carried between the
/// email ask and the new password.
#[derive(uniffi::Record, Clone)]
pub struct FfiResetHandle {
    /// The session ID the homeserver opened.
    pub sid: String,
    /// The client secret that pairs with it.
    pub client_secret: String,
}

/// Read one global account-data event as plain JSON, `None` when it is
/// absent or unreadable.
async fn read_account_data(
    client: &matrix_sdk::Client,
    event_type: &str,
) -> Option<serde_json::Value> {
    client
        .account()
        .account_data_raw(event_type.into())
        .await
        .ok()
        .flatten()
        .and_then(|raw| raw.deserialize_as::<serde_json::Value>().ok())
}

/// Read one MSC2545 image pack out of its JSON, keeping the images whose
/// usage allows the given one (an absent or empty usage allows
/// everything).
fn parse_sticker_pack(
    value: &serde_json::Value,
    fallback_name: &str,
    usage: &str,
) -> Option<FfiStickerPack> {
    let images = value.get("images")?.as_object()?;
    let name = value
        .pointer("/pack/display_name")
        .and_then(|name| name.as_str())
        .unwrap_or(fallback_name)
        .to_owned();

    let stickers: Vec<FfiSticker> = images
        .iter()
        .filter_map(|(shortcode, image)| {
            let url = image.get("url")?.as_str()?.to_owned();
            if let Some(allowed) = image.get("usage").and_then(|allowed| allowed.as_array())
                && !allowed.is_empty()
                && !allowed.iter().any(|entry| entry.as_str() == Some(usage))
            {
                return None;
            }

            Some(FfiSticker {
                shortcode: shortcode.clone(),
                body: image
                    .get("body")
                    .and_then(|body| body.as_str())
                    .unwrap_or(shortcode)
                    .to_owned(),
                url,
                width: image
                    .pointer("/info/w")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|w| u32::try_from(w).ok()),
                height: image
                    .pointer("/info/h")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|h| u32::try_from(h).ok()),
                mime_type: image
                    .pointer("/info/mimetype")
                    .and_then(|mime| mime.as_str())
                    .map(ToOwned::to_owned),
                info_json: image.get("info").map(ToString::to_string),
            })
        })
        .collect();

    if stickers.is_empty() {
        return None;
    }
    Some(FfiStickerPack { name, stickers })
}

/// The account's profile, as far as the server tells it.
#[derive(uniffi::Record)]
pub struct FfiProfile {
    /// The display name, when one is set.
    pub display_name: Option<String>,
    /// The avatar, as an `mxc:` URI, when one is set.
    pub avatar_url: Option<String>,
}

/// One room of the public directory.
#[derive(uniffi::Record)]
pub struct FfiPublicRoom {
    /// The ID of the room.
    pub room_id: String,
    /// The public name of the room, when it has one.
    pub name: Option<String>,
    /// The topic of the room, when it has one.
    pub topic: Option<String>,
    /// The canonical alias of the room, when it has one.
    pub alias: Option<String>,
    /// How many members the room has.
    pub joined_members: u64,
    /// Whether this session is already in the room.
    pub is_joined: bool,
}

/// One page of the public directory.
#[derive(uniffi::Record)]
pub struct FfiPublicRoomPage {
    /// The rooms of this page.
    pub rooms: Vec<FfiPublicRoom>,
    /// The token to request the next page with, absent at the end.
    pub next_batch: Option<String>,
}

/// One page of the media history.
#[derive(uniffi::Record)]
pub struct FfiHistoryPage {
    /// The media events of this page, newest first.
    pub events: Vec<FfiHistoryEvent>,
    /// The token to request the next page with, absent at the start of
    /// the room.
    pub next_token: Option<String>,
}

/// A GIF the picker can present and send.
#[derive(uniffi::Record)]
pub struct FfiGif {
    /// The identifier of the GIF, stable across requests.
    pub id: i64,
    /// The identifier used to report the GIF as shared, valid only for the
    /// response it came in.
    pub slug: String,
    /// A description of the GIF, never empty.
    pub title: String,
    /// The variant to present in the picker: the smallest one, WebP over GIF.
    pub preview_url: String,
    /// The width of the preview, in pixels.
    pub preview_width: u32,
    /// The height of the preview, in pixels.
    pub preview_height: u32,
    /// The variant to send: the largest GIF within the send-size limit.
    pub send_url: String,
    /// The width of the sent variant, in pixels.
    pub send_width: u32,
    /// The height of the sent variant, in pixels.
    pub send_height: u32,
    /// The size of the sent variant, in bytes.
    pub send_size: u64,
}

/// One page of GIF search results.
#[derive(uniffi::Record)]
pub struct FfiGifPage {
    /// The GIFs of this page.
    pub gifs: Vec<FfiGif>,
    /// Whether another page can be requested.
    pub has_next: bool,
}

impl CoreApp {
    /// Run the given action with the matrix room and parsed user ID, off
    /// the runtime.
    async fn with_room_user<F, Fut>(
        &self,
        room_id: String,
        user_id: String,
        action: F,
    ) -> Result<(), CoreError>
    where
        F: FnOnce(matrix_sdk::room::Room, ruma::OwnedUserId) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), CoreError>> + Send,
    {
        let Some(session) = self.first_ready_session() else {
            return Err(CoreError::Failed {
                msg: "No session".to_owned(),
            });
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| CoreError::Failed {
                    msg: "Invalid room ID".to_owned(),
                })?;
                let user_id =
                    ruma::UserId::parse(user_id.trim()).map_err(|_| CoreError::Failed {
                        msg: "That is not a valid user ID".to_owned(),
                    })?;
                let room = session
                    .room_list()
                    .get(&room_id)
                    .ok_or_else(|| CoreError::Failed {
                        msg: "Unknown room".to_owned(),
                    })?;

                action(room.matrix_room().clone(), user_id).await
            })
            .await
            .expect("task was not aborted")
    }

    /// Run the given action with the room and parsed event ID, off the
    /// runtime.
    async fn with_room_event<F, Fut>(
        &self,
        room_id: String,
        event_id: String,
        action: F,
    ) -> Result<(), ()>
    where
        F: FnOnce(crate::session::Room, ruma::OwnedEventId) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), ()>> + Send,
    {
        let Some(session) = self.first_ready_session() else {
            return Err(());
        };

        RUNTIME
            .spawn(async move {
                let room_id = ruma::RoomId::parse(&room_id).map_err(|_| ())?;
                let event_id = ruma::EventId::parse(&event_id).map_err(|_| ())?;
                let room = session.room_list().get(&room_id).ok_or(())?;
                action(room, event_id).await
            })
            .await
            .expect("task was not aborted")
    }

    /// The first session that is ready, if any.
    fn first_ready_session(&self) -> Option<Session> {
        self.session_list
            .subscribe_entries()
            .0
            .iter()
            .find_map(|entry| entry.session().cloned())
    }
}

impl Drop for CoreApp {
    fn drop(&mut self) {
        if let Ok(Some(handle)) = self.listener_handle.get_mut().map(Option::take) {
            handle.abort();
        }
    }
}

/// Wait until the session list has a ready session, and return it.
/// The verification flows in progress and their listener.
impl CoreApp {
    /// The login client the discovery step built, for the flow's next
    /// step.
    fn pending_login_client(&self) -> Result<matrix_sdk::Client, CoreError> {
        self.pending_login
            .lock()
            .expect("mutex is not poisoned")
            .clone()
            .ok_or_else(|| CoreError::Failed {
                msg: "No login in progress".to_owned(),
            })
    }
}

#[derive(Default)]
struct VerificationFlows {
    listener: Mutex<Option<Arc<dyn VerificationListener>>>,
    flows: Mutex<std::collections::HashMap<String, VerificationFlow>>,
    handlers_installed: std::sync::atomic::AtomicBool,
}

enum VerificationFlow {
    Request(matrix_sdk::encryption::verification::VerificationRequest),
    Sas(matrix_sdk::encryption::verification::SasVerification),
}

impl VerificationFlows {
    fn has(&self, flow_id: &str) -> bool {
        self.flows
            .lock()
            .expect("mutex is not poisoned")
            .contains_key(flow_id)
    }

    fn insert_request(
        &self,
        flow_id: String,
        request: matrix_sdk::encryption::verification::VerificationRequest,
    ) {
        self.flows
            .lock()
            .expect("mutex is not poisoned")
            .insert(flow_id, VerificationFlow::Request(request));
    }

    fn insert_sas(
        &self,
        flow_id: String,
        sas: matrix_sdk::encryption::verification::SasVerification,
    ) {
        self.flows
            .lock()
            .expect("mutex is not poisoned")
            .insert(flow_id, VerificationFlow::Sas(sas));
    }

    fn emit(&self, f: impl FnOnce(&Arc<dyn VerificationListener>)) {
        if let Some(listener) = self
            .listener
            .lock()
            .expect("mutex is not poisoned")
            .as_ref()
        {
            f(listener);
        }
    }

    /// Accept the flow: a request is accepted and its SAS awaited, a bare
    /// SAS is accepted directly. Either way the SAS is then followed to
    /// its end.
    async fn accept(self: &Arc<Self>, flow_id: &str) {
        use futures_util::StreamExt;
        use matrix_sdk::encryption::verification::{Verification, VerificationRequestState};

        tracing::info!("Accepting verification flow {flow_id}");

        let flow = {
            let flows = self.flows.lock().expect("mutex is not poisoned");
            match flows.get(flow_id) {
                Some(VerificationFlow::Request(request)) => Some(request.clone()),
                _ => None,
            }
        };

        if let Some(request) = flow {
            if let Err(accept_error) = request.accept().await {
                tracing::error!("Could not accept verification request: {accept_error}");
                return;
            }

            // Wait for the flow to transition into a SAS verification,
            // starting one ourselves once both sides are ready — someone
            // has to go first.
            let mut changes = request.changes();
            while let Some(state) = changes.next().await {
                match state {
                    VerificationRequestState::Ready { .. } => {
                        if let Err(start_error) = request.start_sas().await {
                            tracing::error!("Could not start SAS: {start_error}");
                        }
                    }
                    VerificationRequestState::Transitioned {
                        verification: Verification::SasV1(sas),
                    } => {
                        self.insert_sas(flow_id.to_owned(), sas.clone());
                        if let Err(sas_error) = sas.accept().await {
                            tracing::error!("Could not accept SAS: {sas_error}");
                            return;
                        }
                        self.clone().follow_sas(flow_id.to_owned(), sas);
                        return;
                    }
                    VerificationRequestState::Cancelled(info) => {
                        let flow_id = flow_id.to_owned();
                        self.emit(move |listener| {
                            listener.on_cancelled(flow_id, info.reason().to_owned());
                        });
                        return;
                    }
                    _ => {}
                }
            }
            return;
        }

        let sas = {
            let flows = self.flows.lock().expect("mutex is not poisoned");
            match flows.get(flow_id) {
                Some(VerificationFlow::Sas(sas)) => Some(sas.clone()),
                _ => None,
            }
        };
        if let Some(sas) = sas {
            if let Err(sas_error) = sas.accept().await {
                tracing::error!("Could not accept SAS: {sas_error}");
                return;
            }
            tracing::info!("SAS accepted for flow {flow_id}");
            self.clone().follow_sas(flow_id.to_owned(), sas);
        } else {
            tracing::warn!("No flow found to accept for {flow_id}");
        }
    }

    /// Follow a SAS to its end, reporting the emojis and the outcome.
    fn follow_sas(
        self: Arc<Self>,
        flow_id: String,
        sas: matrix_sdk::encryption::verification::SasVerification,
    ) {
        use futures_util::StreamExt;
        use matrix_sdk::encryption::verification::SasState;

        RUNTIME.spawn(async move {
            let mut changes = sas.changes();
            while let Some(state) = changes.next().await {
                tracing::info!("SAS state for {flow_id}: {state:?}");
                match state {
                    SasState::KeysExchanged {
                        emojis: Some(emojis),
                        ..
                    } => {
                        let ffi: Vec<FfiSasEmoji> = emojis
                            .emojis
                            .iter()
                            .map(|emoji| FfiSasEmoji {
                                symbol: emoji.symbol.to_owned(),
                                description: emoji.description.to_owned(),
                            })
                            .collect();
                        let flow_id = flow_id.clone();
                        self.emit(move |listener| listener.on_emojis(flow_id, ffi));
                    }
                    SasState::Done { .. } => {
                        let flow_id = flow_id.clone();
                        self.emit(move |listener| listener.on_done(flow_id));
                        break;
                    }
                    SasState::Cancelled(info) => {
                        let flow_id = flow_id.clone();
                        let reason = info.reason().to_owned();
                        self.emit(move |listener| listener.on_cancelled(flow_id, reason));
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    async fn confirm(&self, flow_id: &str) {
        let sas = {
            let flows = self.flows.lock().expect("mutex is not poisoned");
            match flows.get(flow_id) {
                Some(VerificationFlow::Sas(sas)) => Some(sas.clone()),
                _ => None,
            }
        };
        if let Some(sas) = sas
            && let Err(confirm_error) = sas.confirm().await
        {
            tracing::error!("Could not confirm verification: {confirm_error}");
        }
    }

    async fn cancel(&self, flow_id: &str) {
        let flow = self
            .flows
            .lock()
            .expect("mutex is not poisoned")
            .remove(flow_id);
        match flow {
            Some(VerificationFlow::Request(request)) => {
                let _ = request.cancel().await;
            }
            Some(VerificationFlow::Sas(sas)) => {
                let _ = sas.cancel().await;
            }
            None => {}
        }
    }
}

async fn wait_for_ready_session(list: &SessionList) -> Session {
    let (entries, mut stream) = list.subscribe_entries();

    if let Some(session) = entries.iter().find_map(|entry| entry.session().cloned()) {
        return session;
    }

    loop {
        let _ = stream.next().await;

        if let Some(session) = list
            .subscribe_entries()
            .0
            .iter()
            .find_map(|entry| entry.session().cloned())
        {
            return session;
        }
    }
}

/// Push the session's room list to the listener, now and on every change.
///
/// Snapshots rather than diffs, debounced: correctness first, the diff
/// bridge comes with the facade's real API pass.
async fn push_room_updates(session: &Session, listener: Arc<dyn RoomListListener>) {
    let room_list = session.room_list().clone();
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<()>();

    // Any room-list diff is a change, and every room added is watched for
    // the changes the sidebar shows.
    {
        let (rooms, mut stream) = room_list.subscribe_entries();
        for room in &rooms {
            watch_room(room, &notify_tx);
        }

        let notify_tx = notify_tx.clone();
        RUNTIME.spawn(async move {
            while let Some(diff) = stream.next().await {
                for room in diff_added_rooms(&diff) {
                    watch_room(&room, &notify_tx);
                }

                if notify_tx.send(()).is_err() {
                    break;
                }
            }
        });
    }

    loop {
        let snapshot: Vec<FfiRoom> = room_list.snapshot().iter().map(FfiRoom::from).collect();
        listener.on_update(snapshot);

        if notify_rx.recv().await.is_none() {
            break;
        }

        // Debounce: collect the burst before snapshotting again.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        while notify_rx.try_recv().is_ok() {}
    }
}

/// The rooms the given diff adds to the list.
fn diff_added_rooms(diff: &eyeball_im::VectorDiff<Room>) -> Vec<Room> {
    use eyeball_im::VectorDiff;

    match diff {
        VectorDiff::Append { values } | VectorDiff::Reset { values } => {
            values.iter().cloned().collect()
        }
        VectorDiff::PushBack { value }
        | VectorDiff::PushFront { value }
        | VectorDiff::Insert { value, .. }
        | VectorDiff::Set { value, .. } => vec![value.clone()],
        _ => Vec::new(),
    }
}

/// Nudge the notify channel whenever something the sidebar shows changes on
/// the given room.
fn watch_room(room: &Room, notify_tx: &mpsc::UnboundedSender<()>) {
    macro_rules! forward {
        ($subscriber:expr) => {{
            let notify_tx = notify_tx.clone();
            let mut subscriber = $subscriber;
            RUNTIME.spawn(async move {
                while subscriber.next().await.is_some() {
                    if notify_tx.send(()).is_err() {
                        break;
                    }
                }
            });
        }};
    }

    forward!(room.subscribe_display_name());
    forward!(room.subscribe_category());
    forward!(room.subscribe_highlight());
    forward!(room.subscribe_notification_count());
    forward!(room.subscribe_latest_activity());
    forward!(room.subscribe_is_read());
    forward!(room.subscribe_avatar_url());
}

/// Convert an SDK timeline item for the FFI.
/// What the given state-event content change is, for the timeline's
/// sentences.
fn ffi_state_change(
    change: &matrix_sdk_ui::timeline::AnyOtherStateEventContentChange,
) -> FfiStateChange {
    use matrix_sdk_ui::timeline::AnyOtherStateEventContentChange as Change;
    use ruma::events::StateEventContentChange;

    match change {
        Change::RoomName(state) => FfiStateChange::Name {
            name: match state {
                StateEventContentChange::Original { content, .. } => {
                    Some(content.name.clone()).filter(|name| !name.is_empty())
                }
                StateEventContentChange::Redacted(_) => None,
            },
        },
        Change::RoomTopic(state) => FfiStateChange::Topic {
            topic: match state {
                StateEventContentChange::Original { content, .. } => {
                    Some(content.topic.clone()).filter(|topic| !topic.is_empty())
                }
                StateEventContentChange::Redacted(_) => None,
            },
        },
        Change::RoomAvatar(_) => FfiStateChange::Avatar,
        Change::RoomCreate(_) => FfiStateChange::Create,
        Change::RoomEncryption(_) => FfiStateChange::Encryption,
        Change::RoomJoinRules(_) => FfiStateChange::JoinRules,
        Change::RoomHistoryVisibility(_) => FfiStateChange::HistoryVisibility,
        Change::RoomCanonicalAlias(_) => FfiStateChange::CanonicalAlias,
        Change::RoomPinnedEvents(_) => FfiStateChange::PinnedEvents,
        _ => FfiStateChange::Other,
    }
}

/// How many replies sit in the thread rooted at the given content.
fn ffi_thread_replies(content: &matrix_sdk_ui::timeline::TimelineItemContent) -> u64 {
    use matrix_sdk_ui::timeline::TimelineItemContent;

    match content {
        TimelineItemContent::MsgLike(msg_like) => msg_like
            .thread_summary
            .as_ref()
            .map_or(0, |summary| u64::from(summary.num_replies)),
        _ => 0,
    }
}

/// The reactions on the given content, aggregated per key.
fn ffi_reactions(
    content: &matrix_sdk_ui::timeline::TimelineItemContent,
    own_user_id: Option<&ruma::UserId>,
) -> Vec<FfiReaction> {
    use matrix_sdk_ui::timeline::TimelineItemContent;

    let TimelineItemContent::MsgLike(msg_like) = content else {
        return Vec::new();
    };

    msg_like
        .reactions
        .iter()
        .map(|(key, senders)| FfiReaction {
            key: key.clone(),
            count: senders.len() as u64,
            is_own: own_user_id.is_some_and(|own| senders.contains_key(own)),
        })
        .collect()
}

/// The reply context of the given content, if it is a reply.
fn ffi_in_reply_to(content: &matrix_sdk_ui::timeline::TimelineItemContent) -> Option<FfiInReplyTo> {
    use matrix_sdk_ui::timeline::{MsgLikeKind, TimelineDetails, TimelineItemContent};

    let TimelineItemContent::MsgLike(msg_like) = content else {
        return None;
    };

    msg_like.in_reply_to.as_ref().map(|details| {
        let embedded = match &details.event {
            TimelineDetails::Ready(embedded) => Some(embedded),
            _ => None,
        };
        FfiInReplyTo {
            event_id: details.event_id.to_string(),
            sender: embedded.map(|event| event.sender.to_string()),
            body: embedded.and_then(|event| match &event.content {
                TimelineItemContent::MsgLike(reply_like) => match &reply_like.kind {
                    MsgLikeKind::Message(message) => Some(message.msgtype().body().to_owned()),
                    _ => None,
                },
                _ => None,
            }),
        }
    })
}

/// Map the SDK's send state to the FFI's.
fn ffi_send_state(
    send_state: Option<&matrix_sdk_ui::timeline::EventSendState>,
) -> Option<FfiSendState> {
    use matrix_sdk_ui::timeline::EventSendState;

    Some(match send_state? {
        EventSendState::NotSentYet { .. } => FfiSendState::Sending,
        EventSendState::SendingFailed { is_recoverable, .. } => {
            if *is_recoverable {
                FfiSendState::RecoverableError
            } else {
                FfiSendState::PermanentError
            }
        }
        EventSendState::Sent { .. } => FfiSendState::Sent,
    })
}

/// Build the FFI view of a message's kind and body.
fn ffi_message_kind(message: &matrix_sdk_ui::timeline::Message) -> (FfiEventKind, String) {
    use ruma::events::room::message::MessageType;

    let msgtype = message.msgtype();
    let blurhash = match msgtype {
        MessageType::Image(image) => image.info.as_deref().and_then(|info| info.blurhash.clone()),
        MessageType::Video(video) => video.info.as_deref().and_then(|info| info.blurhash.clone()),
        _ => None,
    };
    let kind = match msgtype {
        MessageType::Text(_)
        | MessageType::Notice(_)
        | MessageType::Emote(_)
        | MessageType::ServerNotice(_) => FfiEventKind::Text,
        MessageType::Location(location) => FfiEventKind::Location {
            geo_uri: location.geo_uri.clone(),
        },
        MessageType::Image(_) => FfiEventKind::Media {
            kind: FfiMediaKind::Image,
            blurhash,
        },
        MessageType::Video(_) => FfiEventKind::Media {
            kind: FfiMediaKind::Video,
            blurhash,
        },
        MessageType::Audio(_) => FfiEventKind::Media {
            kind: FfiMediaKind::Audio,
            blurhash: None,
        },
        _ => FfiEventKind::Media {
            kind: FfiMediaKind::File,
            blurhash: None,
        },
    };

    (kind, msgtype.body().to_owned())
}

fn ffi_timeline_item(
    item: &matrix_sdk_ui::timeline::TimelineItem,
    own_user_id: Option<&ruma::UserId>,
) -> FfiTimelineItem {
    use matrix_sdk_ui::timeline::{
        MsgLikeKind, TimelineDetails, TimelineItemContent, TimelineItemKind, VirtualTimelineItem,
    };

    match item.kind() {
        TimelineItemKind::Event(event) => {
            let sender_display_name = match event.sender_profile() {
                TimelineDetails::Ready(profile) => profile.display_name.clone(),
                _ => None,
            };

            let (kind, body) = match event.content() {
                TimelineItemContent::MsgLike(msg_like) => match &msg_like.kind {
                    MsgLikeKind::Message(message) => ffi_message_kind(message),
                    MsgLikeKind::Sticker(sticker) => {
                        let content = sticker.content();
                        (
                            FfiEventKind::Media {
                                kind: FfiMediaKind::Image,
                                blurhash: content.info.blurhash.clone(),
                            },
                            content.body.clone(),
                        )
                    }
                    MsgLikeKind::Redacted => (FfiEventKind::Redacted, String::new()),
                    MsgLikeKind::UnableToDecrypt(_) => {
                        (FfiEventKind::UnableToDecrypt, String::new())
                    }
                    _ => (FfiEventKind::Unsupported, String::new()),
                },
                TimelineItemContent::MembershipChange(membership) => (
                    FfiEventKind::Membership {
                        user: membership.user_id().to_string(),
                        change: membership.change().into(),
                    },
                    String::new(),
                ),
                TimelineItemContent::ProfileChange(profile) => (
                    FfiEventKind::ProfileChange {
                        user: profile.user_id().to_string(),
                    },
                    String::new(),
                ),
                TimelineItemContent::OtherState(state) => (
                    FfiEventKind::OtherState {
                        change: ffi_state_change(state.content()),
                    },
                    String::new(),
                ),
                _ => (FfiEventKind::Unsupported, String::new()),
            };

            let thread_replies = ffi_thread_replies(event.content());

            let reactions = ffi_reactions(event.content(), own_user_id);

            let is_edited = {
                use crate::matrix::ext_traits::TimelineItemContentExt;
                event.content().is_edited()
            };

            let in_reply_to = ffi_in_reply_to(event.content());

            FfiTimelineItem::Event {
                unique_id: item.unique_id().0.clone(),
                event_id: event.event_id().map(ToString::to_string),
                thread_replies,
                reactions,
                in_reply_to,
                is_edited,
                receipts: event
                    .read_receipts()
                    .keys()
                    .map(ToString::to_string)
                    .collect(),
                sender: event.sender().to_string(),
                sender_display_name,
                timestamp: event.timestamp().get().into(),
                is_own: event.is_own(),
                kind,
                body,
                send_state: ffi_send_state(event.send_state()),
            }
        }
        TimelineItemKind::Virtual(virtual_item) => match virtual_item {
            VirtualTimelineItem::DateDivider(timestamp) => FfiTimelineItem::DateDivider {
                timestamp: timestamp.get().into(),
            },
            VirtualTimelineItem::ReadMarker => FfiTimelineItem::ReadMarker,
            VirtualTimelineItem::TimelineStart => FfiTimelineItem::TimelineStart,
        },
    }
}

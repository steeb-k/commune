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
    /// The KLIPY API key for the GIF search, from the embedder's build
    /// configuration. `None` or empty makes the feature inert, which is what
    /// a build without a key of its own gets — see
    /// [`crate::klipy::is_available()`]. It is a credential and is never
    /// stored in this repository.
    pub klipy_api_key: Option<String>,
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
        app_name: APP_NAME.to_owned(),
        // Shown in other clients' session lists when the account audits
        // what pushes to it; this embedder is the Android one.
        device_display_name: Some(format!("{APP_NAME} on Android")),
        oauth_client: config::OAuthClientConfig {
            client_uri: url::Url::parse(ANDROID_OAUTH_CLIENT_URI).expect("client URI is valid"),
            redirect_uris: vec![android_redirect_uri()],
        },
        profile: ffi_config.profile,
        data_dir: ffi_config.data_dir.into(),
        cache_dir: ffi_config.cache_dir.into(),
        settings_store: None,
        // The Kotlin application has no translations yet, so it takes the
        // core's English. When it grows them, this is where its own string
        // resource arrives.
        credential_label: None,
        klipy_api_key: ffi_config.klipy_api_key,
        // The core's English, until the Kotlin application has
        // translations; the room keeps whichever name it was created with.
        packs_room_name: None,
        packs_room_topic: None,
    });
}

/// Whether the GIF search is available in this build.
///
/// The desktop application hides the sticker picker's GIF tab entirely when
/// no API key was configured. The Kotlin picker should do the same rather
/// than presenting a search that can only fail.
#[uniffi::export]
#[must_use]
pub fn gif_search_available() -> bool {
    crate::klipy::is_available()
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

/// The core's error enums render themselves for the FFI.
///
/// This is where the English lives for the Kotlin side, and the reason a
/// core module returns a value rather than a sentence: the GTK application
/// implements `UserFacingError` over the same enum with `gettext`, and the
/// two renderings never have to agree.
impl From<crate::session::IgnoredUsersError> for CoreError {
    fn from(error: crate::session::IgnoredUsersError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::DeviceError> for CoreError {
    fn from(error: crate::session::DeviceError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::PushError> for CoreError {
    fn from(error: crate::session::PushError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::AccountError> for CoreError {
    fn from(error: crate::session::AccountError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::SearchError> for CoreError {
    fn from(error: crate::session::SearchError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::MediaHistoryError> for CoreError {
    fn from(error: crate::session::MediaHistoryError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::JoinError> for CoreError {
    fn from(error: crate::session::JoinError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::DirectChatError> for CoreError {
    fn from(error: crate::session::DirectChatError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::RemoteRoomError> for CoreError {
    fn from(error: crate::session::RemoteRoomError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::SpaceChildrenError> for CoreError {
    fn from(error: crate::session::SpaceChildrenError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::DirectoryError> for CoreError {
    fn from(error: crate::session::DirectoryError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::CreateRoomError> for CoreError {
    fn from(error: crate::session::CreateRoomError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::login::LoginError> for CoreError {
    fn from(error: crate::login::LoginError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::login::RegisterError> for CoreError {
    fn from(error: crate::login::RegisterError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::login::ResetPasswordError> for CoreError {
    fn from(error: crate::login::ResetPasswordError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::ImagePacksError> for CoreError {
    fn from(error: crate::session::ImagePacksError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::VerificationError> for CoreError {
    fn from(error: crate::session::VerificationError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::BootstrapError> for CoreError {
    fn from(error: crate::session::BootstrapError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::RecoveryError> for CoreError {
    fn from(error: crate::session::RecoveryError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::RoomKeysError> for CoreError {
    fn from(error: crate::session::RoomKeysError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::TimelineError> for CoreError {
    fn from(error: crate::session::TimelineError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::RoomDetailsError> for CoreError {
    fn from(error: crate::session::RoomDetailsError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::RoomSettingsError> for CoreError {
    fn from(error: crate::session::RoomSettingsError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::AliasError> for CoreError {
    fn from(error: crate::session::AliasError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::ServerAclError> for CoreError {
    fn from(error: crate::session::ServerAclError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::PermissionsError> for CoreError {
    fn from(error: crate::session::PermissionsError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::LogoutError> for CoreError {
    fn from(error: crate::session::LogoutError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::CallError> for CoreError {
    fn from(error: crate::session::CallError) -> Self {
        Self::Failed {
            msg: crate::UserFacingError::to_user_facing(&error),
        }
    }
}

impl From<crate::session::RecoveryState> for FfiRecoveryState {
    fn from(state: crate::session::RecoveryState) -> Self {
        use crate::session::RecoveryState;

        match state {
            RecoveryState::Unknown => Self::Unknown,
            RecoveryState::Enabled => Self::Enabled,
            RecoveryState::Disabled => Self::Disabled,
            RecoveryState::Incomplete => Self::Incomplete,
        }
    }
}

impl From<crate::session::CryptoIdentityState> for FfiCryptoIdentityState {
    fn from(state: crate::session::CryptoIdentityState) -> Self {
        use crate::session::CryptoIdentityState;

        match state {
            CryptoIdentityState::Unknown => Self::Unknown,
            CryptoIdentityState::Missing => Self::Missing,
            CryptoIdentityState::LastManStanding => Self::LastManStanding,
            CryptoIdentityState::OtherSessions => Self::OtherSessions,
        }
    }
}

impl From<crate::session::SessionVerificationState> for FfiVerificationState {
    fn from(state: crate::session::SessionVerificationState) -> Self {
        use crate::session::SessionVerificationState;

        match state {
            SessionVerificationState::Unknown => Self::Unknown,
            SessionVerificationState::Verified => Self::Verified,
            SessionVerificationState::Unverified => Self::Unverified,
        }
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
    /// A call, as the timeline remembers it afterwards.
    Call {
        /// Whether the call carried video.
        has_video: bool,
        /// What became of it, when this session saw the answer.
        outcome: Option<FfiCallOutcome>,
    },
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
    verification: Arc<VerificationBridge>,
    /// The task feeding the member-list listener.
    member_list_listener_handle: Mutex<Option<tokio::task::AbortHandle>>,
    /// The login the discovery step started, kept for the flow's next
    /// step (password, SSO, OAuth, registration, password reset).
    pending_login: Mutex<Option<crate::login::LoginFlow>>,
    /// The calls in flight and the listener following them.
    calls: Arc<CallBridge>,
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
            verification: Arc::new(VerificationBridge::default()),
            member_list_listener_handle: Mutex::new(None),
            pending_login: Mutex::new(None),
            calls: Arc::new(CallBridge::default()),
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
        // The application logs in with the client its discovery built; the
        // Kotlin flow discovers before it shows the password page, so the
        // pending login is that client. Without one — a caller that skipped
        // discovery — the homeserver is discovered here first.
        let flow = match self.pending_login_flow() {
            Ok(flow) => flow,
            Err(_) => crate::login::LoginFlow::discover(&homeserver, true)
                .await
                .map_err(CoreError::from)?,
        };

        flow.login_with_password(&username, &password)
            .await
            .map_err(CoreError::from)?;

        self.adopt_login(flow).await
    }

    /// What the given homeserver offers for logging in, per the
    /// application's discovery: the OAuth 2.0 API when its metadata
    /// resolves, the Matrix native flows otherwise. The client built
    /// here is kept for the flow's next step.
    pub async fn discover_login(&self, homeserver: String) -> Result<FfiLoginMethods, CoreError> {
        use crate::login::LoginApi;

        // Always with autodiscovery: the application's advanced dialog can
        // turn it off for a typed URL, and the Kotlin flow has no such
        // dialog.
        let flow = crate::login::LoginFlow::discover(&homeserver, true)
            .await
            .map_err(CoreError::from)?;

        let (supports_password, supports_sso, supports_oauth) = match flow.api() {
            LoginApi::OAuth => (false, false, true),
            LoginApi::Matrix {
                supports_password,
                supports_sso,
            } => (supports_password, supports_sso, false),
        };
        let methods = FfiLoginMethods {
            homeserver_url: flow.homeserver().to_string(),
            supports_password,
            supports_sso,
            supports_oauth,
        };

        *self.pending_login.lock().expect("mutex is not poisoned") = Some(flow);
        Ok(methods)
    }

    /// The Matrix SSO URL to open in the browser; the redirect carries
    /// the login token back on the application's fixed Android scheme.
    pub async fn sso_login_url(&self) -> Result<String, CoreError> {
        let flow = self.pending_login_flow()?;

        flow.sso_url(&android_redirect_uri())
            .await
            .map(|url| url.to_string())
            .map_err(CoreError::from)
    }

    /// Finish a Matrix SSO login with the token the redirect carried.
    pub async fn finish_sso_login(&self, login_token: String) -> Result<(), CoreError> {
        let flow = self.pending_login_flow()?;

        flow.finish_sso_login(crate::login::SsoCallback::Token(login_token))
            .await
            .map_err(CoreError::from)?;

        self.adopt_login(flow).await
    }

    /// The OAuth 2.0 authorization URL to open in the browser, with the
    /// application's exact client registration.
    pub async fn oauth_login_url(&self) -> Result<String, CoreError> {
        let flow = self.pending_login_flow()?;

        // Never to create an account: the Kotlin flow creates one through
        // the Matrix native register endpoint only.
        flow.oauth_authorization(android_redirect_uri(), false)
            .await
            .map(|data| data.url.to_string())
            .map_err(CoreError::from)
    }

    /// Finish an OAuth 2.0 login with the query string the redirect
    /// carried.
    pub async fn finish_oauth_login(&self, redirect_query: String) -> Result<(), CoreError> {
        use matrix_sdk::utils::UrlOrQuery;

        let flow = self.pending_login_flow()?;

        flow.finish_oauth_login(UrlOrQuery::Query(redirect_query))
            .await
            .map_err(CoreError::from)?;

        self.adopt_login(flow).await
    }

    /// Create an account on the discovered homeserver, walking the
    /// stages a headless client can answer (dummy, and terms — creating
    /// the account is accepting them). Anything more wants a browser.
    pub async fn register_user(&self, username: String, password: String) -> Result<(), CoreError> {
        use crate::login::{AuthStage, RegisterError};

        let flow = self.pending_login_flow()?;

        // The application's `AuthDialog` loop, without the dialog: the
        // stages it would have shown a page for are the ones this cannot
        // answer.
        let mut auth = None;
        for _attempt in 0..4 {
            match flow.register(&username, &password, auth.take()).await {
                Ok(()) => return self.adopt_login(flow).await,
                Err(RegisterError::Uiaa(uiaa_info)) => {
                    let stage = AuthStage::next(&uiaa_info, AuthStage::WITHOUT_INPUT);
                    auth = stage
                        .and_then(|stage| stage.auth_data_without_input())
                        .ok_or_else(|| CoreError::Failed {
                            msg: "This homeserver asks for steps this app cannot answer yet"
                                .to_owned(),
                        })
                        .map(Some)?;
                }
                Err(register_error) => return Err(register_error.into()),
            }
        }

        Err(CoreError::Failed {
            msg: "Could not create account".to_owned(),
        })
    }

    /// Ask the homeserver to email a password-reset link.
    pub async fn request_password_reset(&self, email: String) -> Result<FfiResetHandle, CoreError> {
        let flow = self.pending_login_flow()?;

        // Always a first send: the handle carries no address or attempt
        // count, so a resend from this side is a new session rather than
        // the application's bumped attempt on the same one.
        let session = flow
            .request_password_reset_email(&email, None)
            .await
            .map_err(CoreError::from)?;

        Ok(FfiResetHandle {
            sid: session.sid.to_string(),
            client_secret: session.client_secret.to_string(),
        })
    }

    /// Set the new password once the emailed link was opened, signing
    /// every other session out, as the application does.
    pub async fn reset_password(
        &self,
        new_password: String,
        handle: FfiResetHandle,
    ) -> Result<(), CoreError> {
        let flow = self.pending_login_flow()?;

        let sid = ruma::SessionId::parse(handle.sid).map_err(|_| CoreError::Failed {
            msg: "Invalid reset session".to_owned(),
        })?;
        let client_secret =
            ruma::ClientSecret::parse(handle.client_secret).map_err(|_| CoreError::Failed {
                msg: "Invalid reset secret".to_owned(),
            })?;
        // The address and the attempt count only matter for a resend,
        // which the handle cannot ask for.
        let session = crate::login::EmailSession {
            address: String::new(),
            sid,
            client_secret,
            send_attempt: 1,
        };

        flow.reset_password(&session, &new_password)
            .await
            .map_err(CoreError::from)
    }

    /// Follow 1:1 calls with the given listener: the core speaks
    /// `m.call.*` and hands the embedder the session descriptions and
    /// candidates that WebRTC needs, as the parity plan's split says.
    ///
    /// Replaces any previous listener; registering starts watching.
    pub fn set_call_listener(&self, listener: Arc<dyn CallListener>) {
        let bridge = self.calls.clone();
        *bridge.listener.lock().expect("mutex is not poisoned") = Some(listener);

        // The core's `Calls` installs the handlers when the session is
        // prepared and keeps the one active call; this follows that call
        // and turns what the core hears into the listener's five calls.
        let list = self.session_list.clone();
        let followers = bridge.clone();
        let handle = RUNTIME
            .spawn(async move {
                let session = wait_for_ready_session(&list).await;
                let calls = session.calls().clone();
                let mut active = calls.subscribe_active_call();
                let mut followed: Option<String> = None;

                loop {
                    if let Some(call) = calls.active_call() {
                        let call_id = call.call_id().to_string();
                        if followed.as_ref() != Some(&call_id) {
                            followed = Some(call_id);
                            followers.clone().follow(call);
                        }
                    }

                    if active.next().await.is_none() {
                        break;
                    }
                }
            })
            .abort_handle();

        if let Some(previous) = bridge
            .watch_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// Place a call: send `m.call.invite` with the offer the embedder's
    /// WebRTC produced, and return the call ID everything else uses.
    pub async fn place_call(
        &self,
        room_id: String,
        invitee: String,
        sdp: String,
    ) -> Result<String, CoreError> {
        let session = self.session()?;
        let room = self.room(&room_id)?;

        // The invitee is the room's one other member, as the application
        // addresses it; the one handed over is only checked against it.
        // The member list may have to load, which spawns onto the runtime.
        let call = RUNTIME
            .spawn(async move { session.calls().place(&room, sdp).await })
            .await
            .expect("task was not aborted")
            .map_err(CoreError::from)?;

        if let Some(remote) = call.remote_user_id()
            && remote.as_str() != invitee.trim()
        {
            tracing::warn!("The call was placed to {remote}, not to the {invitee} asked for");
        }

        Ok(call.call_id().to_string())
    }

    /// Answer a call with the embedder's answer description.
    #[allow(
        clippy::unused_async,
        reason = "the signature is the bindings'; the core answers without waiting"
    )]
    pub async fn answer_call(&self, call_id: String, sdp: String) -> Result<(), CoreError> {
        let session = self.session()?;

        session
            .calls()
            .accept(&call_id, sdp)
            .map_err(CoreError::from)
    }

    /// Send gathered ICE candidates. Both `sdp_mid` and the media-line
    /// index ride along: the specification asks for one, and clients in
    /// the wild want the other.
    #[allow(
        clippy::unused_async,
        reason = "the signature is the bindings'; the core answers without waiting"
    )]
    pub async fn send_call_candidates(
        &self,
        call_id: String,
        candidates: Vec<FfiIceCandidate>,
        end_of_candidates: bool,
    ) -> Result<(), CoreError> {
        use ruma::{UInt, events::call::candidates::Candidate};

        let call = self.call(&call_id)?;

        // Queued rather than sent: the application batches its candidates
        // after the invite or the answer, and the end of gathering sends
        // whatever is left with the empty candidate that says so.
        let list: Vec<Candidate> = candidates
            .into_iter()
            .map(|candidate| {
                let mut queued = Candidate::new(candidate.candidate);
                queued.sdp_mid = candidate.sdp_mid;
                queued.sdp_m_line_index = Some(UInt::from(candidate.sdp_m_line_index));
                queued
            })
            .collect();
        call.add_local_candidates(list);

        if end_of_candidates {
            call.local_gathering_done();
        }

        Ok(())
    }

    /// Offer or accept a new session description mid-call — what the
    /// application sends when the camera comes on partway through.
    #[allow(
        clippy::unused_async,
        reason = "the signature is the bindings'; the core answers without waiting"
    )]
    pub async fn send_call_negotiate(
        &self,
        call_id: String,
        sdp: String,
        session_type: String,
    ) -> Result<(), CoreError> {
        let call = self.call(&call_id)?;

        call.send_negotiate(sdp, session_type == "answer");

        Ok(())
    }

    /// Tell the other end what this end has muted, as the application's
    /// `send_stream_metadata` does.
    #[allow(
        clippy::unused_async,
        reason = "the signature is the bindings'; the core answers without waiting"
    )]
    pub async fn send_call_stream_metadata(
        &self,
        call_id: String,
        stream_id: String,
        audio_muted: bool,
        video_muted: bool,
    ) -> Result<(), CoreError> {
        let call = self.call(&call_id)?;

        // The stream is the one our own description named; the embedder's
        // name for it is the fallback when the description named none.
        call.set_local_stream_id_if_missing(stream_id);
        call.set_muted(audio_muted, video_muted);

        Ok(())
    }

    /// Hang up a call that was placed or answered.
    #[allow(
        clippy::unused_async,
        reason = "the signature is the bindings'; the core answers without waiting"
    )]
    pub async fn hangup_call(&self, call_id: String) -> Result<(), CoreError> {
        let call = self.call(&call_id)?;

        call.hangup();

        Ok(())
    }

    /// Decline an incoming call.
    #[allow(
        clippy::unused_async,
        reason = "the signature is the bindings'; the core answers without waiting"
    )]
    pub async fn reject_call(&self, call_id: String) -> Result<(), CoreError> {
        let call = self.call(&call_id)?;

        // Only a ringing call is declined; the application's `reject` does
        // nothing for any other state.
        call.reject();

        Ok(())
    }

    /// The ICE servers the homeserver hands out, with the credentials
    /// that go with them and how long they last.
    pub async fn turn_servers(&self) -> FfiTurnServers {
        let Ok(session) = self.session() else {
            return FfiTurnServers::default();
        };

        // Kept until they go stale, as the application keeps them; the
        // URIs come sorted so that a relay over UDP is the one a call
        // gets. No TURN is not no call: a local network often carries one
        // on host candidates alone.
        RUNTIME
            .spawn(async move { session.calls().turn_credentials_raw().await })
            .await
            .expect("task was not aborted")
            .map_or_else(FfiTurnServers::default, |raw| FfiTurnServers {
                uris: raw.uris,
                username: raw.username,
                password: raw.password,
                ttl_seconds: raw.ttl.as_secs(),
            })
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
                let outcomes = ffi_call_outcomes(&session);
                listener.on_update(
                    items
                        .iter()
                        .map(|item| ffi_timeline_item(item, Some(&own_user_id), &outcomes))
                        .collect(),
                );

                let mut items = items;
                while let Some(diffs) = stream.next().await {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                    let outcomes = ffi_call_outcomes(&session);
                    listener.on_update(
                        items
                            .iter()
                            .map(|item| ffi_timeline_item(item, Some(&own_user_id), &outcomes))
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

                let outcomes = ffi_call_outcomes(&session);
                listener.on_update(
                    items
                        .iter()
                        .map(|item| ffi_timeline_item(item, Some(&own_user_id), &outcomes))
                        .collect(),
                );

                let mut items = items;
                while let Some(diffs) = stream.next().await {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                    let outcomes = ffi_call_outcomes(&session);
                    listener.on_update(
                        items
                            .iter()
                            .map(|item| ffi_timeline_item(item, Some(&own_user_id), &outcomes))
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
        // The doc comment above is part of the generated bindings and their
        // checksum, so it is kept verbatim; what is actually handled is
        // every `MediaMessage` variant — images, videos, audio, files and
        // stickers.
        let session = self.first_ready_session()?;
        let room = self.room(&room_id).ok()?;

        // The timeline may still have to be built, which spawns onto the
        // runtime and so has to run on it.
        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .media_message(&unique_id)
                    .await?
                    .into_file(&session.client())
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
        use ruma::api::client::receipt::create_receipt::v3::ReceiptType;

        let Ok(room) = self.room(&room_id) else {
            return;
        };

        // The read receipt through the displayed timeline and the
        // fully-read marker through the room, as the room history sends
        // them when the newest message is looked at; the timeline this
        // side displays is the live one. The timeline may still have to
        // be built, which spawns onto the runtime and so has to run on it.
        RUNTIME
            .spawn(async move {
                room.send_receipt(ReceiptType::Read, crate::session::ReceiptPosition::End)
                    .await;
                room.send_receipt(ReceiptType::FullyRead, crate::session::ReceiptPosition::End)
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
                let outcomes = ffi_call_outcomes(&session);
                listener.on_update(
                    items
                        .iter()
                        .map(|item| ffi_timeline_item(item, Some(&own_user_id), &outcomes))
                        .collect(),
                );

                let mut items = items;
                while let Some(diffs) = stream.next().await {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                    let outcomes = ffi_call_outcomes(&session);
                    listener.on_update(
                        items
                            .iter()
                            .map(|item| ffi_timeline_item(item, Some(&own_user_id), &outcomes))
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
        let room = self.room(&room_id)?;
        let thread_root = parse_event_id(&root_event_id)?;

        // Through the composer, as the thread's own composer is in the
        // application, minus the mention and emoticon completion the
        // Kotlin thread view does not have. An empty message is not sent.
        let Some(content) = crate::session::compose_message(
            &composer_chunks(&body, &[], &[], false),
            MARKDOWN_ENABLED,
        ) else {
            return Ok(());
        };

        RUNTIME
            .spawn(async move {
                room.thread_timeline(thread_root)
                    .send_message(content)
                    .await
                    .map_err(|error| timeline_failure(error, "Could not send the message"))
            })
            .await
            .expect("task was not aborted")
    }

    /// Paginate the given room's timeline backwards.
    pub async fn paginate_backwards(&self, room_id: String) {
        let Ok(room) = self.room(&room_id) else {
            return;
        };

        // One batch, as one request for older history is; the timeline
        // refuses when it is loading already, not ready yet, or at the
        // start of the room's history.
        RUNTIME
            .spawn(async move {
                room.live_timeline().paginate_backwards().await;
            })
            .await
            .expect("task was not aborted");
    }

    /// A snapshot of the given room's members, loading the list on first
    /// use — the composer's mention completion reads this.
    pub async fn room_members(&self, room_id: String) -> Vec<FfiMember> {
        let Ok(room) = self.room(&room_id) else {
            return Vec::new();
        };

        // The first call starts the load; the list says when it is done.
        let member_list = room.member_list();
        member_list.loaded().await;

        member_list.snapshot().iter().map(FfiMember::from).collect()
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
        let room = self.room(&room_id)?;

        // The application's completion offers `@room` only where our own
        // member may notify the room and the room is not a direct chat.
        let permissions = room.permissions();
        permissions.ensure_loaded().await;
        let allow_at_room = !room.is_direct() && permissions.state().can_notify_room;

        // The composer's chunks — text, mentions, emoticons — become the
        // application's exact content in the core; finding the chunks in
        // the plain text is this side's shortcut. An empty message is not
        // sent, as the composer does not send one.
        let Some(content) = crate::session::compose_message(
            &composer_chunks(&body, &mentions, &emoticons, allow_at_room),
            MARKDOWN_ENABLED,
        ) else {
            return Ok(());
        };

        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .send_message(content)
                    .await
                    .map_err(|error| timeline_failure(error, "Could not send the message"))
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
        let room = self.room(&room_id)?;
        let event_id = parse_event_id(&event_id)?;

        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .toggle_reaction(event_id, &key)
                    .await
                    .map_err(|error| timeline_failure(error, "Could not toggle the reaction"))
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
        let room = self.room(&room_id)?;
        let in_reply_to = parse_event_id(&in_reply_to)?;

        // Through the composer, as the toolbar's reply is: the body is
        // Markdown, whatever the doc comment above — part of the bindings'
        // checksum — still calls it.
        let Some(content) = crate::session::compose_message(
            &composer_chunks(&body, &[], &[], false),
            MARKDOWN_ENABLED,
        ) else {
            return Ok(());
        };

        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .send_reply(content, in_reply_to)
                    .await
                    .map_err(|error| timeline_failure(error, "Could not send the reply"))
            })
            .await
            .expect("task was not aborted")
    }

    /// Replace the given event's content with the given plain text.
    pub async fn edit_message(
        &self,
        room_id: String,
        event_id: String,
        new_body: String,
    ) -> Result<(), CoreError> {
        let room = self.room(&room_id)?;
        let event_id = parse_event_id(&event_id)?;

        // Through the composer, as the toolbar's edit is; see `send_reply`
        // about the doc comment.
        let Some(content) = crate::session::compose_message(
            &composer_chunks(&new_body, &[], &[], false),
            MARKDOWN_ENABLED,
        ) else {
            return Ok(());
        };

        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .edit(event_id, content)
                    .await
                    .map_err(|error| timeline_failure(error, "Could not edit the message"))
            })
            .await
            .expect("task was not aborted")
    }

    /// Redact the given event in the given room.
    pub async fn redact_event(&self, room_id: String, event_id: String) -> Result<(), CoreError> {
        let room = self.room(&room_id)?;
        let event_id = parse_event_id(&event_id)?;

        // Through the room, as the application's remove action is: the
        // event does not have to be among the timeline's loaded items.
        room.redact(&[event_id], None)
            .await
            .map_err(|_failed| CoreError::Failed {
                msg: "Could not remove the message".to_owned(),
            })
    }

    /// Move the given room to the given category: accepting an invite is a
    /// move to Normal, declining it (or leaving) a move to Left.
    pub async fn change_room_category(
        &self,
        room_id: String,
        category: FfiTargetRoomCategory,
    ) -> Result<(), CoreError> {
        let room = self.room(&room_id)?;

        room.change_category(category.into())
            .await
            .map_err(|change_error| CoreError::Failed {
                msg: format!("Could not move the room: {change_error}"),
            })
    }

    /// Open a direct chat with the given user: the existing one when
    /// there is one, a newly created encrypted DM otherwise. Returns the
    /// room ID.
    pub async fn create_direct_chat(&self, user_id: String) -> Result<String, CoreError> {
        let session = self.session()?;
        let user_id = parse_user_id(&user_id)?;

        // The wait for the new room to reach the list is a timer-less
        // `get_wait`, but the room list's stream is driven by the sync
        // task, so this runs on the runtime with it.
        RUNTIME
            .spawn(async move {
                session
                    .room_list()
                    .get_or_create_direct_chat(&user_id)
                    .await
                    .map(|room| room.room_id().to_string())
                    .map_err(CoreError::from)
            })
            .await
            .expect("task was not aborted")
    }

    /// Join the room with the given ID or alias. Returns the room ID.
    pub async fn join_room(&self, room_id_or_alias: String) -> Result<String, CoreError> {
        let session = self.session()?;

        // A matrix.to link or a matrix: URI is taken too, with its `via`
        // servers, as the application's join dialog takes them.
        let uri =
            crate::matrix::MatrixRoomIdUri::parse(room_id_or_alias.trim()).ok_or_else(|| {
                CoreError::Failed {
                    msg: "Not a room ID or alias".to_owned(),
                }
            })?;

        // The application previews the room first and then knocks or
        // joins by what the preview said the join rule is; without a
        // preview to show, this does the same two steps back to back.
        let remote_room = session.remote_room(uri).await.map_err(CoreError::from)?;

        session
            .room_list()
            .knock_or_join(&remote_room)
            .await
            .map(|room_id| room_id.to_string())
            .map_err(CoreError::from)
    }

    /// Follow device verifications with the given listener, accepting
    /// the flows the listener's side approves.
    ///
    /// Replaces any previous listener; registering starts watching for
    /// incoming requests.
    pub fn set_verification_listener(&self, listener: Arc<dyn VerificationListener>) {
        let bridge = self.verification.clone();

        *bridge.listener.lock().expect("mutex is not poisoned") = Some(listener);

        // As with calls: the list belongs to one session, and an account
        // switch hands us a different one. Following once per ready session
        // and no more is what the bound id records.
        let active_id = self
            .first_ready_session()
            .map(|session| session.session_id().to_owned());
        if active_id.is_some() {
            let mut bound = bridge.bound_session.lock().expect("mutex is not poisoned");
            if *bound == active_id {
                return;
            }
            bound.clone_from(&active_id);
        }

        let list = self.session_list.clone();
        let watcher = bridge.clone();
        let handle = RUNTIME
            .spawn(async move {
                use ruma::events::key::verification::VerificationMethod;

                let session = wait_for_ready_session(&list).await;
                let verifications = session.verification_list().clone();

                // This side can scan a QR code and compare emojis, but has
                // nowhere to show a QR code of its own.
                verifications.set_supported_methods(vec![
                    VerificationMethod::SasV1,
                    VerificationMethod::QrCodeScanV1,
                    VerificationMethod::ReciprocateV1,
                ]);

                // Follow every verification the list holds, now and as they
                // arrive; the core's list is what the application's
                // notifications and views bind to.
                let mut changed = verifications.subscribe_changed();
                loop {
                    for verification in verifications.snapshot() {
                        let flow_id = verification.flow_id().to_owned();
                        let is_new = watcher
                            .followed
                            .lock()
                            .expect("mutex is not poisoned")
                            .insert(flow_id);
                        if is_new {
                            watcher.clone().follow(verification);
                        }
                    }
                    if changed.next().await.is_none() {
                        break;
                    }
                }
            })
            .abort_handle();

        if let Some(previous) = bridge
            .watch_handle
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// Search the given room's messages on the server — the application's
    /// search criteria: message bodies, most recent first. Encrypted
    /// rooms cannot be searched by the server.
    pub async fn search_room(
        &self,
        room_id: String,
        search_term: String,
    ) -> Result<Vec<FfiSearchResult>, CoreError> {
        // The doc comment above is part of the generated bindings and their
        // checksum, so it is kept verbatim. It is now incomplete rather than
        // wrong: the server still cannot search an encrypted room, and
        // `RoomSearch` searches the local index for one instead, as the
        // application does.
        //
        // One page only. The search object pages, but nothing on this side
        // of the FFI asks for a second page, so the page is a wider one than
        // the application's — thirty results rather than twenty — and that
        // is the only respect in which this differs from the application's
        // search.
        const SINGLE_PAGE_SIZE: usize = 30;

        let room = self.room(&room_id)?;
        let search = crate::session::RoomSearch::with_page_size(&room, SINGLE_PAGE_SIZE);
        search.set_search_term(&search_term);

        let results = search.load_more().await.map_err(CoreError::from)?;

        Ok(results.iter().map(FfiSearchResult::from).collect())
    }

    /// Set the given room's name and topic.
    pub async fn set_room_details(
        &self,
        room_id: String,
        name: String,
        topic: String,
    ) -> Result<(), CoreError> {
        let room = self.room(&room_id)?;

        // Whitespace is trimmed and an emptied field removes the value, as
        // the details page does; only what changed is sent, as it sends
        // only what was edited.
        let name = Some(name.trim())
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned);
        if name != room.name() {
            room.set_name(name.as_deref())
                .await
                .map_err(CoreError::from)?;
        }

        let topic = Some(topic.trim())
            .filter(|topic| !topic.is_empty())
            .map(ToOwned::to_owned);
        if topic != room.topic() {
            room.set_topic(topic.as_deref())
                .await
                .map_err(CoreError::from)?;
        }

        Ok(())
    }

    /// The rooms inside the given space, from the server's hierarchy.
    pub async fn space_children(&self, space_id: String) -> Result<Vec<FfiSpaceChild>, CoreError> {
        let session = self.session()?;
        let space_id = parse_room_id(&space_id)?;

        let hierarchy = crate::session::SpaceChildren::load(&session, space_id)
            .await
            .map_err(CoreError::from)?;

        // The application presents the hierarchy as a tree that opens a
        // subspace on demand; the FFI's list is flat, so the tree is walked
        // depth-first here — every row the tree could show, in the order
        // it would show them, with the same guard against a space that
        // contains itself.
        let mut rows = Vec::new();
        let mut pending = hierarchy.children();
        pending.reverse();
        while let Some(child) = pending.pop() {
            if let Some(children) = child.children(&hierarchy) {
                pending.extend(children.into_iter().rev());
            }
            rows.push(child);
        }

        Ok(rows
            .iter()
            .map(|child| {
                let room = &child.room;
                FfiSpaceChild {
                    room_id: room.room_id.to_string(),
                    name: room.name.clone(),
                    topic: room.topic.clone(),
                    num_joined_members: u64::from(room.joined_members_count),
                    is_joined: room.local_room(session.room_list()).is_some(),
                    is_space: room.is_space,
                }
            })
            .collect())
    }

    /// Feed a scanned QR code into the verification with the given flow
    /// ID. The outcome arrives through the listener: done, or cancelled.
    pub async fn scan_qr(&self, flow_id: String, data: Vec<u8>) -> Result<(), CoreError> {
        use matrix_sdk::encryption::verification::QrVerificationData;

        let verification = self.verification(&flow_id)?;

        let data = QrVerificationData::from_bytes(&data).map_err(|_| CoreError::Failed {
            msg: "Not a verification QR code".to_owned(),
        })?;

        verification
            .qr_code_scanned(data)
            .await
            .map_err(CoreError::from)
    }

    /// Ask the account's verified sessions to verify this one. The flow
    /// then arrives through the listener like an incoming one: emojis,
    /// then done.
    pub async fn request_verification(&self) -> Result<String, CoreError> {
        let session = self.session()?;

        // The flow then arrives through the listener like any other: the
        // list's watcher picks a new verification up as it is added.
        session
            .verification_list()
            .create(None)
            .await
            .map(|verification| verification.flow_id().to_owned())
            .map_err(CoreError::from)
    }

    /// Ask another user to verify: the request goes into the direct
    /// chat as a message, and the flow then runs like any other SAS.
    pub async fn request_user_verification(&self, user_id: String) -> Result<String, CoreError> {
        let session = self.session()?;
        let user_id = parse_user_id(&user_id)?;

        session
            .verification_list()
            .create(Some(&user_id))
            .await
            .map(|verification| verification.flow_id().to_owned())
            .map_err(CoreError::from)
    }

    /// Accept the verification with the given flow ID; the emojis arrive
    /// through the listener when both sides are ready.
    pub async fn accept_verification(&self, flow_id: String) {
        // The listener's follower starts SAS once the request is ready:
        // this side has no page to choose a method on.
        if let Ok(verification) = self.verification(&flow_id)
            && let Err(accept_error) = verification.accept().await
        {
            tracing::error!("Could not accept verification {flow_id}: {accept_error}");
        }
    }

    /// Confirm that the emojis matched.
    pub async fn confirm_verification(&self, flow_id: String) {
        if let Ok(verification) = self.verification(&flow_id)
            && let Err(confirm_error) = verification.sas_match().await
        {
            tracing::error!("Could not confirm verification {flow_id}: {confirm_error}");
        }
    }

    /// Cancel the verification — the emojis did not match, or the user
    /// declined.
    pub async fn cancel_verification(&self, flow_id: String) {
        if let Ok(verification) = self.verification(&flow_id)
            && let Err(cancel_error) = verification.cancel().await
        {
            tracing::error!("Could not cancel verification {flow_id}: {cancel_error}");
        }
    }

    /// Where account recovery stands for the first ready session.
    pub async fn recovery_state(&self) -> FfiRecoveryState {
        let Ok(session) = self.session() else {
            return FfiRecoveryState::Unknown;
        };

        let security = session.security().clone();
        security.ensure_loaded().await;
        security.recovery_state().into()
    }

    /// Set up recovery, returning the recovery key to write down.
    pub async fn enable_recovery(&self) -> Result<String, CoreError> {
        let session = self.session()?;

        // Without a passphrase: the Kotlin flow shows the key and has no
        // field for one.
        session
            .security()
            .enable_recovery(None)
            .await
            .map_err(CoreError::from)
    }

    /// Where this session stands on encryption: whether the account has
    /// a crypto identity and other verified sessions, whether this
    /// session is verified, and whether recovery is set up. The
    /// application's `session/security.rs` computes the same three.
    pub async fn security_state(&self) -> FfiSecurityState {
        let Ok(session) = self.session() else {
            return FfiSecurityState {
                identity: FfiCryptoIdentityState::Unknown,
                verification: FfiVerificationState::Unknown,
                recovery: FfiRecoveryState::Unknown,
            };
        };

        let security = session.security().clone();
        security.ensure_loaded().await;

        FfiSecurityState {
            identity: security.crypto_identity_state().into(),
            verification: security.verification_state().into(),
            recovery: security.recovery_state().into(),
        }
    }

    /// Create the account's crypto identity — cross-signing — for an
    /// account that has none, answering the password stage the
    /// homeserver asks for.
    pub async fn bootstrap_cross_signing(&self, password: String) -> Result<(), CoreError> {
        use ruma::api::client::uiaa::AuthType;

        use crate::{login::AuthStage, session::BootstrapError};

        let session = self.session()?;
        let security = session.security().clone();

        // The application's `AuthDialog` loop, without the dialog: the
        // password is the one stage this side can answer.
        let Err(BootstrapError::Uiaa(uiaa_info)) = security.bootstrap_cross_signing(None).await
        else {
            return security
                .bootstrap_cross_signing(None)
                .await
                .map_err(CoreError::from);
        };

        let stage = AuthStage::next(&uiaa_info, &[AuthType::Password])
            .filter(|stage| stage.stage == AuthType::Password)
            .ok_or_else(|| CoreError::Failed {
                msg: "This homeserver asks for steps this app cannot answer yet".to_owned(),
            })?;
        let auth = stage.password_data(session.user_id(), &password);

        security
            .bootstrap_cross_signing(Some(auth))
            .await
            .map_err(CoreError::from)
    }

    /// Recover the account's secrets with the given recovery key.
    pub async fn recover(&self, recovery_key: String) -> Result<(), CoreError> {
        let session = self.session()?;

        // An incomplete recovery is not a failure: the secrets that came in
        // are kept, and the recovery state says what is still missing.
        session
            .security()
            .recover(recovery_key.trim())
            .await
            .map(|_outcome| ())
            .map_err(CoreError::from)
    }

    /// Send the file at the given path as an attachment to the given room.
    pub async fn send_attachment(
        &self,
        room_id: String,
        file_path: String,
        mime_type: String,
    ) -> Result<(), CoreError> {
        let room = self.room(&room_id)?;
        let mime = mime_type
            .parse::<mime::Mime>()
            .unwrap_or(mime::APPLICATION_OCTET_STREAM);

        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .send_attachment(file_path.into(), mime)
                    .await
                    .map_err(|error| timeline_failure(error, "Could not send the attachment"))
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
        let room = self.room(&room_id)?;

        let selection = crate::klipy::SelectedGif {
            url: gif.send_url,
            width: gif.send_width,
            height: gif.send_height,
            size: gif.send_size,
            slug: gif.slug,
            title: gif.title,
        };
        let is_encrypted = room.is_encrypted();

        // Through the timeline, as the application sends it. No upload-size
        // check first: the application bounds the download instead, and
        // leaves the homeserver to refuse what is too large.
        RUNTIME
            .spawn(async move { room.live_timeline().send_gif(selection, is_encrypted).await })
            .await
            .expect("task was not aborted")
            .map_err(|send_error| CoreError::Failed {
                msg: crate::UserFacingError::to_user_facing(&send_error),
            })
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
        let room = self.room(&room_id)?;

        let page = room
            .media_history_page(from.as_deref())
            .await
            .map_err(CoreError::from)?;

        Ok(FfiHistoryPage {
            events: page.events.iter().map(FfiHistoryEvent::from).collect(),
            next_token: page.end,
        })
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
            .spawn(async move { session.log_out().await.map_err(CoreError::from) })
            .await
            .expect("task was not aborted")?;

        self.session_list.remove(&session_id);
        // Whoever is next resolves as active from here.
        self.session_list.set_active(None);
        Ok(())
    }

    /// Every session on this device, ready or not, in the stored order.
    pub fn sessions(&self) -> Vec<FfiSessionInfo> {
        let active = self
            .first_ready_session()
            .map(|s| s.session_id().to_owned());
        self.session_list
            .subscribe_entries()
            .0
            .iter()
            .map(|entry| {
                let ready = entry.session().is_some();
                let (user_id, homeserver) = match entry.session() {
                    Some(session) => (
                        session.user_id().to_string(),
                        session.info().homeserver.to_string(),
                    ),
                    None => (String::new(), String::new()),
                };
                FfiSessionInfo {
                    session_id: entry.session_id().to_owned(),
                    user_id,
                    homeserver,
                    ready,
                    active: active.as_deref() == Some(entry.session_id()),
                }
            })
            .collect()
    }

    /// Make the given session the one everything resolves to. The
    /// embedder re-arms its listeners after this, the way it does after
    /// a login.
    pub fn set_active_session(&self, session_id: String) {
        self.session_list.set_active(Some(session_id));
    }

    /// The account's current profile.
    pub async fn account_profile(&self) -> Option<FfiProfile> {
        let session = self.first_ready_session()?;

        // The session keeps the profile as an observable, refreshed from
        // the homeserver and cached on disk. Fetching separately here left
        // two answers to one question inside the same core.
        let refresh = session.clone();
        RUNTIME
            .spawn(async move { refresh.refresh_profile().await })
            .await
            .expect("task was not aborted");

        let profile = session.profile();
        Some(FfiProfile {
            display_name: profile.display_name,
            avatar_url: profile.avatar_url.map(|url| url.to_string()),
        })
    }

    /// Change the account's display name.
    pub async fn set_display_name(&self, name: String) -> Result<(), CoreError> {
        let session = self.session()?;

        RUNTIME
            .spawn(async move { session.set_display_name(name.trim()).await })
            .await
            .expect("task was not aborted")
            .map_err(CoreError::from)
    }

    /// Upload the file at the given path as the account's avatar.
    pub async fn set_account_avatar(
        &self,
        file_path: String,
        mime_type: String,
    ) -> Result<(), CoreError> {
        let session = self.session()?;

        RUNTIME
            .spawn(async move {
                let data = std::fs::read(&file_path)
                    .map_err(|_| crate::session::AccountError::UnreadableImage)?;
                let mime = mime_type.parse::<mime::Mime>().unwrap_or(mime::IMAGE_JPEG);
                session.set_avatar(&mime, data).await
            })
            .await
            .expect("task was not aborted")
            .map_err(CoreError::from)
    }

    /// Invite the given user to the given room.
    pub async fn invite_user(&self, room_id: String, user_id: String) -> Result<(), CoreError> {
        let room = self.room(&room_id)?;
        let user_id = parse_user_id(&user_id)?;

        room.invite(&[user_id])
            .await
            .map_err(|_failed| CoreError::Failed {
                msg: "Could not invite the user".to_owned(),
            })
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
        use crate::session::{CreateRoomOptions, CreateRoomVisibility};

        let session = self.session()?;

        // The application's dialog refuses a public room without an
        // address; this side has no form to refuse in, so an absent one is
        // sent as empty and the homeserver answers. A leading `#` is
        // stripped because the Kotlin form has no suffix showing the
        // server name, so people type the whole alias.
        let visibility = if public {
            CreateRoomVisibility::Public {
                address: alias
                    .map(|a| a.trim().trim_start_matches('#').to_owned())
                    .unwrap_or_default(),
            }
        } else {
            CreateRoomVisibility::Private { encrypted }
        };

        session
            .create_room(CreateRoomOptions {
                name: Some(name),
                topic,
                is_space: false,
                visibility,
            })
            .await
            .map(|room_id| room_id.to_string())
            .map_err(CoreError::from)
    }

    /// The account's sessions, ours first.
    pub async fn list_devices(&self) -> Result<Vec<FfiDevice>, CoreError> {
        let session = self.session()?;

        let sessions = session.user_sessions().clone();
        RUNTIME
            .spawn(async move { sessions.ensure_loaded().await })
            .await
            .expect("task was not aborted");

        let sessions = session.user_sessions();
        // Only when neither source answered: the application shows what it
        // has when one of the two fails, and so does the core.
        if sessions.state() == crate::utils::LoadingState::Error {
            return Err(crate::session::DeviceError::NotLoaded.into());
        }

        Ok(sessions
            .snapshot()
            .into_iter()
            .map(FfiDevice::from)
            .collect())
    }

    /// Rename one of the account's sessions.
    pub async fn rename_device(&self, device_id: String, name: String) -> Result<(), CoreError> {
        let session = self.session()?;
        let device_id: ruma::OwnedDeviceId = device_id.into();

        RUNTIME
            .spawn(async move {
                session
                    .user_sessions()
                    .rename(&device_id, name.trim())
                    .await
            })
            .await
            .expect("task was not aborted")
            .map_err(CoreError::from)
    }

    /// Sign another of the account's sessions out. The server demands the
    /// password again for this.
    pub async fn sign_out_device(
        &self,
        device_id: String,
        password: String,
    ) -> Result<(), CoreError> {
        let session = self.session()?;
        let device_id: ruma::OwnedDeviceId = device_id.into();

        RUNTIME
            .spawn(async move {
                session
                    .user_sessions()
                    .sign_out(&device_id, Some(&password))
                    .await
            })
            .await
            .expect("task was not aborted")
            .map_err(CoreError::from)
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
        let Ok(session) = self.session() else {
            return Vec::new();
        };

        // Reading the cache alone would answer "nobody" for a session that
        // is active but has not finished preparing.
        let ensure = session.clone();
        RUNTIME
            .spawn(async move { ensure.ignored_users().ensure_loaded().await })
            .await
            .expect("task was not aborted");

        session
            .ignored_users()
            .snapshot()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Ignore the given user: their messages disappear everywhere.
    pub async fn ignore_user(&self, user_id: String) -> Result<(), CoreError> {
        let session = self.session()?;
        let user_id = parse_user_id(&user_id)?;

        RUNTIME
            .spawn(async move { session.ignored_users().add(&user_id).await })
            .await
            .expect("task was not aborted")
            .map_err(CoreError::from)
    }

    /// Stop ignoring the given user.
    pub async fn unignore_user(&self, user_id: String) -> Result<(), CoreError> {
        let session = self.session()?;
        let user_id = parse_user_id(&user_id)?;

        RUNTIME
            .spawn(async move { session.ignored_users().remove(&user_id).await })
            .await
            .expect("task was not aborted")
            .map_err(CoreError::from)
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
        let session = self.session()?;
        let room = self.room(&room_id)?;
        let mime = mime_type
            .parse::<mime::Mime>()
            .unwrap_or(mime::APPLICATION_OCTET_STREAM);

        let data = std::fs::read(&file_path).map_err(|read_error| CoreError::Failed {
            msg: format!("Could not read the file: {read_error}"),
        })?;

        // The application uploads without asking the limit first; this
        // side asks, as it does for every upload, so an oversized picture
        // is refused before it is sent.
        let size = u64::try_from(data.len()).ok();
        crate::session::check_upload_size(&session.client(), size).await?;

        room.set_avatar(&mime, data, width.map(Into::into), height.map(Into::into))
            .await
            .map_err(CoreError::from)
    }

    /// Remove the room's avatar.
    pub async fn remove_room_avatar(&self, room_id: String) -> Result<(), CoreError> {
        let room = self.room(&room_id)?;

        room.remove_avatar().await.map_err(CoreError::from)
    }

    /// The room's join rule, with what this room's version supports and
    /// whether we may change it.
    pub async fn room_join_rule(&self, room_id: String) -> Result<FfiJoinRuleInfo, CoreError> {
        use ruma::events::room::join_rules::JoinRule;

        use crate::session::JoinRuleValue;

        let room = self.room(&room_id)?;
        let state = room.join_rule().state();

        // The application's value and knock flag, as the FFI's one enum.
        let value = match (state.value, state.can_knock) {
            (JoinRuleValue::Public, _) => FfiJoinRuleValue::Public,
            (JoinRuleValue::Invite, false) => FfiJoinRuleValue::Invite,
            (JoinRuleValue::Invite, true) => FfiJoinRuleValue::Knock,
            (JoinRuleValue::RoomMembership, false) => FfiJoinRuleValue::Restricted,
            (JoinRuleValue::RoomMembership, true) => FfiJoinRuleValue::KnockRestricted,
            (JoinRuleValue::Unsupported, _) => FfiJoinRuleValue::Unsupported,
        };
        let allow = match &state.matrix_join_rule {
            Some(JoinRule::Restricted(restricted) | JoinRule::KnockRestricted(restricted)) => {
                allow_room_ids(restricted)
            }
            _ => Vec::new(),
        };

        let authorization = room.rules().authorization;

        // As the application's page decides it: a rule it cannot edit is
        // not changed whatever the power level.
        let can_change = state.value.can_be_edited()
            && can_send_state(&room, ruma::events::StateEventType::RoomJoinRules).await;

        Ok(FfiJoinRuleInfo {
            value,
            allow_room_ids: allow,
            supports_knock: authorization.knocking,
            supports_restricted: authorization.restricted_join_rule,
            supports_knock_restricted: authorization.knock_restricted_join_rule,
            can_change,
        })
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
        use ruma::events::room::join_rules::{AllowRule, JoinRule, Restricted};

        use crate::session::{JoinRuleValue, compute_join_rule};

        let room = self.room(&room_id)?;

        // The FFI's one enum is the application's value and knock switch.
        let (value, can_knock) = match value {
            FfiJoinRuleValue::Public => (JoinRuleValue::Public, false),
            FfiJoinRuleValue::Invite => (JoinRuleValue::Invite, false),
            FfiJoinRuleValue::Knock => (JoinRuleValue::Invite, true),
            FfiJoinRuleValue::Restricted => (JoinRuleValue::RoomMembership, false),
            FfiJoinRuleValue::KnockRestricted => (JoinRuleValue::RoomMembership, true),
            FfiJoinRuleValue::Unsupported => {
                return Err(CoreError::Failed {
                    msg: "Cannot set an unsupported rule".to_owned(),
                });
            }
        };

        // The page hides what the room's version cannot take; this side
        // refuses it, since there is no picker to hide it from.
        let authorization = room.rules().authorization;
        let version_supports = match (value, can_knock) {
            (JoinRuleValue::RoomMembership, true) => authorization.knock_restricted_join_rule,
            (JoinRuleValue::RoomMembership, false) => authorization.restricted_join_rule,
            (JoinRuleValue::Invite, true) => authorization.knocking,
            _ => true,
        };
        if !version_supports {
            return Err(CoreError::Failed {
                msg: "The version of this room does not support this rule".to_owned(),
            });
        }

        let current = room.join_rule().matrix_join_rule();

        // A space picked here replaces the whole allow list. A list left
        // alone is carried over verbatim, however many rooms are in it, so
        // that changing whether people may knock never changes who may join.
        let restricted = match allow_space_id {
            Some(space_id) => Some(Restricted::new(vec![AllowRule::room_membership(
                parse_room_id(&space_id)?,
            )])),
            None => match &current {
                Some(JoinRule::Restricted(restricted) | JoinRule::KnockRestricted(restricted)) => {
                    Some(restricted.clone())
                }
                _ => None,
            },
        };

        // The membership rule with nothing to allow is a half-finished
        // choice, not a rule: the page will not send it, and neither will
        // this.
        let Some(rule) = compute_join_rule(value, can_knock, restricted) else {
            return Err(CoreError::Failed {
                msg: "A membership rule needs a space".to_owned(),
            });
        };

        // The page saves only a change.
        if current.unwrap_or(JoinRule::Invite) == rule {
            return Ok(());
        }

        room.join_rule()
            .set_matrix_join_rule(rule)
            .await
            .map_err(CoreError::from)
    }

    /// The room's history visibility, and whether we may change it.
    pub async fn room_history_visibility(
        &self,
        room_id: String,
    ) -> Result<FfiHistoryVisibilityInfo, CoreError> {
        use crate::session::HistoryVisibilityValue;

        let room = self.room(&room_id)?;
        let visibility = room.history_visibility();

        // The FFI's enum has no unsupported variant, so an unsupported
        // value shows as the most restrictive one and cannot be changed —
        // the page shows the real value and refuses the change.
        let value = match visibility {
            HistoryVisibilityValue::WorldReadable => FfiHistoryVisibility::WorldReadable,
            HistoryVisibilityValue::Shared => FfiHistoryVisibility::Shared,
            HistoryVisibilityValue::Invited => FfiHistoryVisibility::Invited,
            HistoryVisibilityValue::Joined | HistoryVisibilityValue::Unsupported => {
                FfiHistoryVisibility::Joined
            }
        };
        let can_change = visibility != HistoryVisibilityValue::Unsupported
            && can_send_state(&room, ruma::events::StateEventType::RoomHistoryVisibility).await;

        Ok(FfiHistoryVisibilityInfo { value, can_change })
    }

    /// Change the room's history visibility.
    pub async fn set_room_history_visibility(
        &self,
        room_id: String,
        value: FfiHistoryVisibility,
    ) -> Result<(), CoreError> {
        use crate::session::HistoryVisibilityValue;

        let room = self.room(&room_id)?;

        let value = match value {
            FfiHistoryVisibility::WorldReadable => HistoryVisibilityValue::WorldReadable,
            FfiHistoryVisibility::Shared => HistoryVisibilityValue::Shared,
            FfiHistoryVisibility::Invited => HistoryVisibilityValue::Invited,
            FfiHistoryVisibility::Joined => HistoryVisibilityValue::Joined,
        };

        // The page saves only a change.
        if room.history_visibility() == value {
            return Ok(());
        }

        room.set_history_visibility(value)
            .await
            .map_err(CoreError::from)
    }

    /// The room's addresses: canonical and alternative public aliases,
    /// the aliases registered on this homeserver, and whether the room
    /// is published in the server's directory.
    pub async fn room_addresses(&self, room_id: String) -> Result<FfiRoomAddresses, CoreError> {
        let room = self.room(&room_id)?;
        let aliases = room.aliases();
        let state = aliases.state();

        // The application's page shows nothing for a list it could not
        // fetch, and an unpublished switch for a visibility it could not
        // read; this record has no room for "unknown".
        let local = aliases
            .local_aliases()
            .await
            .unwrap_or_default()
            .iter()
            .map(ToString::to_string)
            .collect();
        let published = room.is_published().await.unwrap_or(false);

        // There is no clear definition of who is allowed to publish a room
        // to the directory in the Matrix spec; the application assumes it
        // does not make sense unless the user can change the public
        // addresses.
        let can_change =
            can_send_state(&room, ruma::events::StateEventType::RoomCanonicalAlias).await;

        Ok(FfiRoomAddresses {
            canonical: state.canonical_alias.map(|alias| alias.to_string()),
            alt: state.alt_aliases.iter().map(ToString::to_string).collect(),
            local,
            published,
            can_change,
        })
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
        let room = self.room(&room_id)?;
        let aliases = room.aliases();

        let parse_alias = |alias: &str| {
            ruma::RoomAliasId::parse(alias).map_err(|_| CoreError::Failed {
                msg: "Invalid address".to_owned(),
            })
        };

        // Each edit is the application's, refusals included: it refuses to
        // send an event that changes nothing, and names the two reasons an
        // address cannot be added and the one it cannot be registered.
        match action {
            FfiAddressAction::Publish { published } => {
                room.set_published(published).await.map_err(CoreError::from)
            }
            FfiAddressAction::RegisterLocal { alias } => aliases
                .register_local_alias(parse_alias(&alias)?)
                .await
                .map_err(|error| alias_failure(error, "Could not register local address")),
            FfiAddressAction::UnregisterLocal { alias } => aliases
                .unregister_local_alias(parse_alias(&alias)?)
                .await
                .map_err(|error| alias_failure(error, "Could not unregister local address")),
            FfiAddressAction::SetCanonical { alias } => aliases
                .set_canonical_alias(parse_alias(&alias)?)
                .await
                .map_err(|error| alias_failure(error, "Could not set main public address")),
            FfiAddressAction::RemoveCanonical { alias } => aliases
                .remove_canonical_alias(&parse_alias(&alias)?)
                .await
                .map_err(|error| alias_failure(error, "Could not remove public address")),
            FfiAddressAction::AddAlt { alias } => aliases
                .add_alt_alias(parse_alias(&alias)?)
                .await
                .map_err(|error| alias_failure(error, "Could not add public address")),
            FfiAddressAction::RemoveAlt { alias } => aliases
                .remove_alt_alias(&parse_alias(&alias)?)
                .await
                .map_err(|error| alias_failure(error, "Could not remove public address")),
        }
    }

    /// The room's server ACL. An absent event reads as the open default:
    /// every server allowed, IP literals too.
    pub async fn room_server_acl(&self, room_id: String) -> Result<FfiServerAcl, CoreError> {
        let room = self.room(&room_id)?;

        // A room with no ACL at all starts from the unrestricted one, as
        // the page does, so blocking a single server is one action rather
        // than two.
        let content = room
            .server_acl()
            .await
            .map_err(CoreError::from)?
            .unwrap_or_else(crate::session::unrestricted_acl);

        Ok(FfiServerAcl {
            allow: content.allow,
            deny: content.deny,
            allow_ip_literals: content.allow_ip_literals,
            can_change: can_send_state(&room, ruma::events::StateEventType::RoomServerAcl).await,
        })
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

        use crate::session::AclProblem;

        let session = self.session()?;
        let room = self.room(&room_id)?;

        let acl = RoomServerAclEventContent::new(allow_ip_literals, allow, deny);

        // The page's two checks, from the worst down. An ACL that allows
        // no server is refused: it shuts every homeserver out of the room
        // and nothing can send the repair. One that shuts our own server
        // out is confirmed by the page; this side has no dialog to ask
        // with, so it is refused too, and says why.
        match crate::session::check_acl(&acl, session.user_id().server_name()) {
            Some(AclProblem::NoServerAllowed) => {
                return Err(CoreError::Failed {
                    msg: "At least one allowed server is required. An empty list shuts every \
                          homeserver out of the room, which cannot be undone from here."
                        .to_owned(),
                });
            }
            Some(AclProblem::OwnServerExcluded) => {
                return Err(CoreError::Failed {
                    msg: "This list would shut your own homeserver out of the room".to_owned(),
                });
            }
            None => {}
        }

        // The page saves only a change.
        let remote = room
            .server_acl()
            .await
            .map_err(CoreError::from)?
            .unwrap_or_else(crate::session::unrestricted_acl);
        if crate::session::acls_are_equal(&acl, &remote) {
            return Ok(());
        }

        room.set_server_acl(acl).await.map_err(CoreError::from)
    }

    /// The room's permission thresholds — the application's permissions
    /// page, flattened: role defaults, action levels, and the per-event
    /// overrides it exposes.
    pub async fn room_permissions_matrix(
        &self,
        room_id: String,
    ) -> Result<FfiPowerLevelsMatrix, CoreError> {
        use ruma::events::{StateEventType, room::power_levels::PowerLevelAction};

        let room = self.room(&room_id)?;
        let permissions = room.permissions();
        permissions.ensure_loaded().await;

        let matrix =
            crate::session::PowerLevelsMatrix::from_power_levels(&permissions.power_levels());

        Ok(FfiPowerLevelsMatrix {
            users_default: matrix.users_default,
            events_default: matrix.events_default,
            state_default: matrix.state_default,
            invite: matrix.invite,
            kick: matrix.kick,
            ban: matrix.ban,
            redact_others: matrix.redact_others,
            redact_own: matrix.redact_own,
            notify_room: matrix.notify_room,
            name: matrix.name,
            topic: matrix.topic,
            avatar: matrix.avatar,
            aliases: matrix.aliases,
            history_visibility: matrix.history_visibility,
            encryption: matrix.encryption,
            power_levels: matrix.power_levels,
            server_acl: matrix.server_acl,
            upgrade: matrix.upgrade,
            can_change: permissions
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomPowerLevels)),
        })
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
        let room = self.room(&room_id)?;
        let permissions = room.permissions();
        permissions.ensure_loaded().await;

        let rows = crate::session::PowerLevelsMatrix {
            users_default: matrix.users_default,
            events_default: matrix.events_default,
            state_default: matrix.state_default,
            invite: matrix.invite,
            kick: matrix.kick,
            ban: matrix.ban,
            redact_others: matrix.redact_others,
            redact_own: matrix.redact_own,
            notify_room: matrix.notify_room,
            name: matrix.name,
            topic: matrix.topic,
            avatar: matrix.avatar,
            aliases: matrix.aliases,
            history_visibility: matrix.history_visibility,
            encryption: matrix.encryption,
            power_levels: matrix.power_levels,
            server_acl: matrix.server_acl,
            upgrade: matrix.upgrade,
        };

        // The page saves only a change; the rows are compared as the page
        // reads them, after the same clamping.
        let mut power_levels = permissions.power_levels();
        if crate::session::PowerLevelsMatrix::from_power_levels(&power_levels) == rows {
            return Ok(());
        }
        rows.apply_to(&mut power_levels);

        permissions
            .set_power_levels(power_levels)
            .await
            .map_err(CoreError::from)
    }

    /// The room versions an upgrade could go to, per the application's
    /// rules: stable versions at or above both the current and the
    /// server's default, the current and default versions listed as
    /// unstable when they are, sorted, with the suggested pick marked.
    pub async fn room_upgrade_info(&self, room_id: String) -> Result<FfiUpgradeInfo, CoreError> {
        let session = self.session()?;
        let room = self.room(&room_id)?;

        // The general page fetches the server's room versions once, when it
        // opens; the answer is the SDK's to cache.
        let capability = session
            .client()
            .homeserver_capabilities()
            .room_versions()
            .await
            .map_err(|capability_error| CoreError::Failed {
                msg: format!("Could not ask the server: {capability_error}"),
            })?;

        let info = room
            .upgrade_info(&capability)
            .ok_or_else(|| CoreError::Failed {
                msg: "The room has no create event".to_owned(),
            })?;

        room.permissions().ensure_loaded().await;
        let can_upgrade = room.can_upgrade();

        Ok(FfiUpgradeInfo {
            current_version: info.current_room_version.to_string(),
            stable: info
                .stable_room_versions
                .iter()
                .map(ToString::to_string)
                .collect(),
            unstable: info
                .unstable_room_versions
                .iter()
                .map(ToString::to_string)
                .collect(),
            selected_index: u32::try_from(info.selected).unwrap_or(0),
            can_upgrade,
        })
    }

    /// Upgrade the room to the given version.
    pub async fn upgrade_room(
        &self,
        room_id: String,
        new_version: String,
    ) -> Result<(), CoreError> {
        use ruma::api::client::room::upgrade_room;

        let session = self.session()?;
        let room_id = parse_room_id(&room_id)?;
        let new_version =
            ruma::RoomVersionId::try_from(new_version.as_str()).map_err(|_| CoreError::Failed {
                msg: "Invalid room version".to_owned(),
            })?;

        // The one request the general page makes once the dialog answers;
        // a passthrough on both sides, by the standing ruling.
        session
            .client()
            .send(upgrade_room::v3::Request::new(room_id, new_version))
            .await
            .map(|_response| ())
            .map_err(|upgrade_error| CoreError::Failed {
                msg: format!("Could not upgrade the room: {upgrade_error}"),
            })
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
        let session = self.session()?;

        session
            .security()
            .export_room_keys(path.into(), &passphrase)
            .await
            .map_err(CoreError::from)
    }

    /// Import room keys from an encrypted export at the given path.
    ///
    /// Returns how many keys came in.
    pub async fn import_room_keys(
        &self,
        path: String,
        passphrase: String,
    ) -> Result<u64, CoreError> {
        let session = self.session()?;

        session
            .security()
            .import_room_keys(path.into(), &passphrase)
            .await
            .map(|count| count as u64)
            .map_err(CoreError::from)
    }

    /// Send a recorded voice message.
    pub async fn send_voice_message(
        &self,
        room_id: String,
        file_path: String,
        mime_type: String,
        duration_ms: u64,
    ) -> Result<(), CoreError> {
        let room = self.room(&room_id)?;
        let mime = mime_type
            .parse::<mime::Mime>()
            .unwrap_or(mime::APPLICATION_OCTET_STREAM);

        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .send_voice(
                        file_path.into(),
                        mime,
                        duration_ms,
                        VOICE_MESSAGE_FILENAME.to_owned(),
                    )
                    .await
                    .map_err(|error| timeline_failure(error, "Could not send the voice message"))
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
        // The doc comment above is part of the generated bindings and their
        // checksum, so it is kept verbatim. What is listed is what the
        // application lists — the packs enabled everywhere, then a room's
        // own — with the packs room standing in for the open room, which
        // this side of the FFI has no way to name. Personal `im.ponies`
        // packs are not read: the specification dropped them, and neither
        // does the application.
        self.image_packs_for_usage(crate::events::image_packs::PackUsage::Sticker)
            .await
    }

    /// The emoticon images of the same packs, for the composer's
    /// `:shortcode:` completion.
    pub async fn emoticon_packs(&self) -> Vec<FfiStickerPack> {
        self.image_packs_for_usage(crate::events::image_packs::PackUsage::Emoticon)
            .await
    }

    /// Send the user's location to the room, as the application's
    /// message toolbar does.
    pub async fn send_location(&self, room_id: String, geo_uri: String) -> Result<(), CoreError> {
        let room = self.room(&room_id)?;

        // The body is the embedder's sentence: the application's is
        // translated and stamps local time. UTC keeps this side off the
        // platform's timezone database.
        let timestamp = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Iso8601::DEFAULT)
            .unwrap_or_default();
        let body = format!("User Location {geo_uri} at {timestamp}");

        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .send_location(geo_uri, body)
                    .await
                    .map_err(|error| timeline_failure(error, "Could not send the location"))
            })
            .await
            .expect("task was not aborted")
    }

    /// The image packs this account owns — the ones in Commune's own
    /// packs room, which are the ones it may edit.
    pub async fn my_image_packs(&self) -> Vec<FfiOwnedPack> {
        let Ok(image_packs) = self.image_packs() else {
            return Vec::new();
        };

        RUNTIME
            .spawn(async move {
                let Some(room) = image_packs.stored_packs_room().await else {
                    return Vec::new();
                };

                // Including the empty ones: the Kotlin flow creates a pack
                // and adds its images afterwards, so a named pack with no
                // images yet is one being made, not one that was deleted —
                // which is what a pack with neither name nor images is.
                crate::session::room_state_packs_including_empty(&room)
                    .await
                    .into_iter()
                    .filter(|(_, (_, content))| {
                        !content.images.is_empty() || content.pack.display_name.is_some()
                    })
                    .map(|(state_key, (_, content))| FfiOwnedPack {
                        name: content
                            .pack
                            .display_name
                            .clone()
                            .unwrap_or_else(|| state_key.clone()),
                        images: content
                            .images
                            .iter()
                            .map(|(shortcode, image)| FfiSticker::from_pack_image(shortcode, image))
                            .collect(),
                        state_key,
                    })
                    .collect()
            })
            .await
            .expect("task was not aborted")
    }

    /// Create an image pack, making Commune's packs room first when
    /// there is none — as the application's `packs_room` does, down to
    /// the room's name, topic, privacy and low-priority tag.
    pub async fn create_image_pack(&self, name: String) -> Result<String, CoreError> {
        use crate::{
            events::image_packs::{PackContent, PackMeta},
            session::{ImagePackSource, ImagePacks, RoomPackKind},
        };

        let image_packs = self.image_packs()?;

        // `packs_room` waits for the created room to reach the list, which
        // the sync task drives, so this runs on the runtime.
        RUNTIME
            .spawn(async move {
                let room = image_packs.packs_room().await.map_err(CoreError::from)?;
                let state_key = ImagePacks::unused_state_key(&room).await;
                let source = ImagePackSource {
                    room: room.clone(),
                    state_key: state_key.clone(),
                    // A pack that we create uses the event type that we send.
                    kind: RoomPackKind::Stable,
                };

                // Without an image yet: the application's editor refuses to
                // save a pack without one, but the Kotlin flow names the pack
                // first and adds images to it, and `my_image_packs` keeps a
                // named empty pack visible for that.
                let mut pack = PackMeta::default();
                pack.display_name = Some(name);
                let content = PackContent {
                    images: std::collections::BTreeMap::new(),
                    pack,
                };
                image_packs
                    .save_pack(&source, content)
                    .await
                    .map_err(CoreError::from)?;

                // A pack is only usable in the room it lives in, and a pack
                // that was just created lives in a room that exists for that,
                // so it would be usable nowhere the user meant — the
                // application enables a new pack everywhere on its first save.
                let _ = image_packs
                    .set_pack_enabled(room.room_id(), &state_key, true)
                    .await;

                Ok(state_key)
            })
            .await
            .expect("task was not aborted")
    }

    /// Add an image to one of this account's packs: upload the file,
    /// then write it into the pack's state event.
    pub async fn add_pack_image(
        &self,
        state_key: String,
        shortcode: String,
        body: String,
        file_path: String,
        mime_type: String,
    ) -> Result<(), CoreError> {
        use ruma::{assign, events::room::ImageInfo};

        use crate::events::image_packs::PackImage;

        let session = self.session()?;
        let image_packs = self.image_packs()?;
        let (source, mut content) = self.owned_pack(&state_key).await?;

        let mime = mime_type
            .parse::<mime::Mime>()
            .unwrap_or(mime::APPLICATION_OCTET_STREAM);
        let data = std::fs::read(&file_path).map_err(|read_error| CoreError::Failed {
            msg: format!("Could not read the file: {read_error}"),
        })?;
        let size = u64::try_from(data.len()).ok();
        let client = session.client();
        crate::session::check_upload_size(&client, size).await?;

        let upload_mime = mime.clone();
        let response = RUNTIME
            .spawn(async move { client.media().upload(&upload_mime, data, None).await })
            .await
            .expect("task was not aborted")
            .map_err(|upload_error| CoreError::Failed {
                msg: format!("Could not upload the image: {upload_error}"),
            })?;

        // The application's editor also records the width and height it
        // decoded; this side has no decoder, so the info carries what is
        // known without one.
        let mut image = PackImage::new(response.content_uri);
        image.body = Some(body);
        image.info = Some(Box::new(assign!(ImageInfo::new(), {
            size: size.and_then(|size| size.try_into().ok()),
            mimetype: Some(mime.to_string()),
        })));
        content.images.insert(shortcode, image);

        image_packs
            .save_pack(&source, content)
            .await
            .map_err(CoreError::from)
    }

    /// Remove one image from a pack.
    pub async fn remove_pack_image(
        &self,
        state_key: String,
        shortcode: String,
    ) -> Result<(), CoreError> {
        let image_packs = self.image_packs()?;
        let (source, mut content) = self.owned_pack(&state_key).await?;

        content.images.remove(&shortcode);

        image_packs
            .save_pack(&source, content)
            .await
            .map_err(CoreError::from)
    }

    /// Rename a pack.
    pub async fn rename_image_pack(
        &self,
        state_key: String,
        name: String,
    ) -> Result<(), CoreError> {
        let image_packs = self.image_packs()?;
        let (source, mut content) = self.owned_pack(&state_key).await?;

        content.pack.display_name = Some(name);

        image_packs
            .save_pack(&source, content)
            .await
            .map_err(CoreError::from)
    }

    /// Delete a pack. A state event cannot be removed, so a deleted
    /// pack is one with no images — what a redacted pack looks like
    /// too — and it stops being enabled everywhere.
    pub async fn delete_image_pack(&self, state_key: String) -> Result<(), CoreError> {
        let image_packs = self.image_packs()?;
        let (source, _) = self.owned_pack(&state_key).await?;

        image_packs
            .delete_pack(&source)
            .await
            .map_err(CoreError::from)
    }

    /// Enable or disable a room's pack everywhere, through the stable
    /// `m.image_pack.rooms` account data.
    pub async fn set_pack_enabled(
        &self,
        room_id: String,
        state_key: String,
        enabled: bool,
    ) -> Result<(), CoreError> {
        let image_packs = self.image_packs()?;
        let room_id = parse_room_id(&room_id)?;

        image_packs.ensure_loaded().await;
        image_packs
            .set_pack_enabled(&room_id, &state_key, enabled)
            .await
            .map_err(CoreError::from)
    }

    /// Send a sticker from a pack.
    pub async fn send_sticker(
        &self,
        room_id: String,
        sticker: FfiSticker,
    ) -> Result<(), CoreError> {
        use ruma::events::{room::ImageInfo, sticker::StickerEventContent};

        let room = self.room(&room_id)?;

        let url = ruma::OwnedMxcUri::from(sticker.url);
        // The event carries the pack image's declared `info`
        // whole, as the application's `sticker_content` does.
        let info = sticker
            .info_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<ImageInfo>(json).ok())
            .unwrap_or_default();
        let content = StickerEventContent::new(sticker.body, info, url);

        // Through the timeline, as the application sends it: a local echo
        // and the send queue, rather than a bare request.
        RUNTIME
            .spawn(async move { room.live_timeline().send_sticker(content).await })
            .await
            .expect("task was not aborted")
            .map_err(|error| timeline_failure(error, "Could not send the sticker"))
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
        let room = self.room(&room_id)?;
        let event_id = parse_event_id(&event_id)?;

        // Never fails past this point: without routing, the link is the
        // room ID's own, as the application falls back to.
        Ok(room.matrix_to_event_uri(event_id).await.to_string())
    }

    /// The raw JSON of the given event, pretty-printed — the properties
    /// dialog's source view.
    pub async fn event_source(
        &self,
        room_id: String,
        event_id: String,
    ) -> Result<String, CoreError> {
        let room = self.room(&room_id)?;
        let event_id = parse_event_id(&event_id)?;

        // Read from the loaded timeline item, as the properties dialog
        // reads it; the application offers the view only for an event it
        // has the source of.
        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .event_source(&event_id)
                    .await
                    .ok_or_else(|| CoreError::Failed {
                        msg: "The source of the event is not available".to_owned(),
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
        let room = self.room(&room_id)?;
        let event_id = parse_event_id(&event_id)?;

        room.report_events(&[(event_id, reason)])
            .await
            .map_err(|_failed| CoreError::Failed {
                msg: "Could not report the event".to_owned(),
            })
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
        let room = self.room(&room_id)?;
        let target = self.room(&target_room_id)?;
        let event_id = parse_event_id(&event_id)?;

        // With no precedent to mirror there is no core method either: this
        // stays the FFI's own design, and stays here, until the
        // application registers its Forward action.
        RUNTIME
            .spawn(async move {
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
        let room = self.room(&room_id)?;

        RUNTIME
            .spawn(async move {
                room.live_timeline()
                    .discard_local_echo(&unique_id)
                    .await
                    .map_err(|error| timeline_failure(error, "Could not discard the message"))
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
        let session = self.session()?;

        session
            .set_push_gateway(&gateway_url, &pushkey)
            .await
            .map_err(CoreError::from)
    }

    /// Remove the pusher with the given pushkey, so the homeserver stops
    /// pushing to it.
    pub async fn remove_push_gateway(&self, pushkey: String) -> Result<(), CoreError> {
        let session = self.session()?;

        session
            .remove_push_gateway(&pushkey)
            .await
            .map_err(CoreError::from)
    }

    /// One page of the public room directory, optionally filtered by a
    /// search term, continuing from `since` when given.
    pub async fn explore_rooms(
        &self,
        search: Option<String>,
        since: Option<String>,
    ) -> Result<FfiPublicRoomPage, CoreError> {
        let session = self.session()?;

        // Our own homeserver on the Matrix network: the application's
        // server chooser and third-party networks are not on this side of
        // the FFI yet.
        let query = crate::session::PublicRoomsQuery {
            search_term: search.filter(|term| !term.is_empty()),
            server: None,
            third_party_network: None,
        };

        let page = session
            .public_rooms(&query, since)
            .await
            .map_err(CoreError::from)?;

        let rooms = page
            .rooms
            .iter()
            .map(|room| FfiPublicRoom {
                room_id: room.room_id.to_string(),
                name: room.name.clone(),
                topic: room.topic.clone(),
                alias: room.canonical_alias.as_ref().map(ToString::to_string),
                joined_members: u64::from(room.joined_members_count),
                is_joined: room.local_room(session.room_list()).is_some(),
            })
            .collect();

        Ok(FfiPublicRoomPage {
            rooms,
            next_batch: page.next_batch,
        })
    }

    /// Fetch the media of a history event into a file, returning its path.
    pub async fn get_history_media(&self, room_id: String, event_id: String) -> Option<String> {
        let session = self.first_ready_session()?;
        let room = self.room(&room_id).ok()?;
        let event_id = ruma::EventId::parse(&event_id).ok()?;

        // The application's history viewer keeps the event it listed; the
        // FFI hands an ID across, so the event is fetched again — from the
        // store when it is there.
        let matrix_room = room.matrix_room().clone();
        let event = RUNTIME
            .spawn(async move { matrix_room.event(&event_id, None).await })
            .await
            .expect("task was not aborted")
            .ok()?;
        let message = crate::matrix::original_message_event_from_raw(event.raw())?;

        crate::matrix::media::MediaMessage::from_message(&message.content.msgtype)?
            .into_file(&session.client())
            .await
            .map(|path| path.to_string_lossy().into_owned())
    }
}

impl From<crate::session::MediaHistoryKind> for FfiHistoryKind {
    fn from(kind: crate::session::MediaHistoryKind) -> Self {
        use crate::session::MediaHistoryKind;

        match kind {
            MediaHistoryKind::Media => Self::Media,
            MediaHistoryKind::File => Self::File,
            MediaHistoryKind::Audio => Self::Audio,
        }
    }
}

/// The FFI view of one media-history event.
impl From<&crate::session::MediaHistoryEvent> for FfiHistoryEvent {
    fn from(event: &crate::session::MediaHistoryEvent) -> Self {
        use ruma::events::room::message::MessageType;

        let message = event.event();

        let (body, mime_type, size) = match &message.content.msgtype {
            MessageType::Image(image) => (
                image.filename().to_owned(),
                image.info.as_ref().and_then(|info| info.mimetype.clone()),
                image.info.as_ref().and_then(|info| info.size),
            ),
            MessageType::Video(video) => (
                video.filename().to_owned(),
                video.info.as_ref().and_then(|info| info.mimetype.clone()),
                video.info.as_ref().and_then(|info| info.size),
            ),
            MessageType::Audio(audio) => (
                audio.filename().to_owned(),
                audio.info.as_ref().and_then(|info| info.mimetype.clone()),
                audio.info.as_ref().and_then(|info| info.size),
            ),
            MessageType::File(file) => (
                file.filename().to_owned(),
                file.info.as_ref().and_then(|info| info.mimetype.clone()),
                file.info.as_ref().and_then(|info| info.size),
            ),
            // The core only builds a history event for the four kinds above.
            _ => (message.content.body().to_owned(), None, None),
        };

        Self {
            event_id: message.event_id.to_string(),
            sender: message.sender.to_string(),
            timestamp: message.origin_server_ts.0.into(),
            kind: event.kind().into(),
            body,
            mime_type,
            size: size.map(u64::from),
            is_video: matches!(&message.content.msgtype, MessageType::Video(_)),
        }
    }
}

impl From<&crate::session::SearchResult> for FfiSearchResult {
    fn from(result: &crate::session::SearchResult) -> Self {
        Self {
            event_id: result.event_id().to_string(),
            sender: result.sender_id().to_string(),
            body: result.body(),
            timestamp: result.timestamp().get().into(),
        }
    }
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

impl From<crate::session::Device> for FfiDevice {
    fn from(device: crate::session::Device) -> Self {
        Self {
            device_id: device.device_id.to_string(),
            display_name: device.display_name,
            is_current: device.is_current,
            is_verified: device.is_verified,
            last_seen_ts: device.last_seen_ts,
            last_seen_ip: device.last_seen_ip,
        }
    }
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

/// One of this account's own image packs, with the state key that
/// identifies it for editing.
#[derive(uniffi::Record)]
pub struct FfiOwnedPack {
    /// The state key the pack lives at in the packs room.
    pub state_key: String,
    /// The pack's display name.
    pub name: String,
    /// The images in the pack.
    pub images: Vec<FfiSticker>,
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

impl FfiSticker {
    /// The FFI view of one image of a pack.
    fn from_pack_image(shortcode: &str, image: &crate::events::image_packs::PackImage) -> Self {
        let info = image.info.as_deref();

        Self {
            shortcode: shortcode.to_owned(),
            body: image.body.clone().unwrap_or_else(|| shortcode.to_owned()),
            url: image.url.to_string(),
            width: info
                .and_then(|info| info.width)
                .and_then(|width| u32::try_from(width).ok()),
            height: info
                .and_then(|info| info.height)
                .and_then(|height| u32::try_from(height).ok()),
            mime_type: info.and_then(|info| info.mimetype.clone()),
            // The pack image's raw `info`, sent whole with the sticker.
            info_json: info.and_then(|info| serde_json::to_string(info).ok()),
        }
    }
}

impl From<&crate::session::ImagePack> for FfiStickerPack {
    fn from(pack: &crate::session::ImagePack) -> Self {
        Self {
            // The application falls back to the name of the room; when that
            // is one of the interface's own sentences, the state key names
            // the pack, as it did before this group moved.
            name: pack
                .display_name()
                .unwrap_or_else(|| pack.source.state_key.clone()),
            stickers: pack
                .content
                .images
                .iter()
                .map(|(shortcode, image)| FfiSticker::from_pack_image(shortcode, image))
                .collect(),
        }
    }
}

/// The image packs, and the pack this account owns.
impl CoreApp {
    /// The image packs of the active session.
    fn image_packs(&self) -> Result<crate::session::ImagePacks, CoreError> {
        Ok(self.session()?.image_packs().clone())
    }

    /// The packs whose images can be used for the given usage: the ones
    /// enabled everywhere, then the packs room's own.
    ///
    /// The application lists the packs enabled everywhere and then the
    /// open room's own. This side of the FFI has no room to name, so the
    /// packs room — the one room the Kotlin application writes packs into,
    /// and where a pack it created before this group enabled new packs
    /// lives — stands in for it.
    async fn image_packs_for_usage(
        &self,
        usage: crate::events::image_packs::PackUsage,
    ) -> Vec<FfiStickerPack> {
        use crate::session::ImagePacks;

        let Ok(image_packs) = self.image_packs() else {
            return Vec::new();
        };

        RUNTIME
            .spawn(async move {
                image_packs.ensure_loaded().await;

                let mut packs = image_packs.enabled_everywhere(Some(&usage)).await;

                if let Some(room) = image_packs.stored_packs_room().await {
                    let seen = packs
                        .iter()
                        .map(|pack| {
                            (
                                pack.source.room.room_id().to_owned(),
                                pack.source.state_key.clone(),
                            )
                        })
                        .collect::<Vec<_>>();

                    for pack in ImagePacks::room_packs(&room).await {
                        let key = (
                            pack.source.room.room_id().to_owned(),
                            pack.source.state_key.clone(),
                        );
                        if !seen.contains(&key) && !pack.is_empty() && pack.has_usage(&usage) {
                            packs.push(pack);
                        }
                    }
                }

                packs.iter().map(FfiStickerPack::from).collect()
            })
            .await
            .expect("task was not aborted")
    }

    /// One of this account's own packs, with the source to write it back
    /// under, ready to edit.
    async fn owned_pack(
        &self,
        state_key: &str,
    ) -> Result<
        (
            crate::session::ImagePackSource,
            crate::events::image_packs::PackContent,
        ),
        CoreError,
    > {
        use crate::session::{ImagePackSource, ImagePacksError};

        let image_packs = self.image_packs()?;
        let room = image_packs
            .stored_packs_room()
            .await
            .ok_or(ImagePacksError::NoPacksRoom)?;

        // Including the empty ones: a pack that was just created has no
        // images yet, and this is how it gets its first.
        let (kind, content) = crate::session::room_state_packs_including_empty(&room)
            .await
            .shift_remove(state_key)
            .ok_or(ImagePacksError::UnknownPack)?;

        Ok((
            ImagePackSource {
                room,
                state_key: state_key.to_owned(),
                kind,
            },
            content,
        ))
    }
}

/// Parse a room ID handed over the FFI.
fn parse_room_id(room_id: &str) -> Result<ruma::OwnedRoomId, CoreError> {
    ruma::RoomId::parse(room_id).map_err(|_| CoreError::Failed {
        msg: "Invalid room ID".to_owned(),
    })
}

/// Parse a user ID handed over the FFI, trimmed as the application trims
/// what somebody typed.
fn parse_user_id(user_id: &str) -> Result<ruma::OwnedUserId, CoreError> {
    ruma::UserId::parse(user_id.trim()).map_err(|_| CoreError::Failed {
        msg: "That is not a valid user ID".to_owned(),
    })
}

/// Parse an event ID handed over the FFI.
fn parse_event_id(event_id: &str) -> Result<ruma::OwnedEventId, CoreError> {
    ruma::EventId::parse(event_id).map_err(|_| CoreError::Failed {
        msg: "Invalid event ID".to_owned(),
    })
}

/// Whether messages are sent as Markdown.
///
/// The application's setting, on by default; the Kotlin application has
/// no such setting.
const MARKDOWN_ENABLED: bool = true;

/// The file name of a voice message, which is what other clients show as
/// its body.
///
/// The application's is translated; this is the Kotlin side's English.
const VOICE_MESSAGE_FILENAME: &str = "Voice message.ogg";

/// The FFI's sentence for a failed timeline action.
///
/// The application's toasts name the action, so the sentence is the
/// caller's. The upload limit is the one refusal the core has a value
/// for, and that one is rendered with it.
fn timeline_failure(error: crate::session::TimelineError, sentence: &str) -> CoreError {
    use crate::session::TimelineError;

    match error {
        TimelineError::UploadTooLarge { .. } => CoreError::from(error),
        TimelineError::NoTimeline
        | TimelineError::UnknownEvent
        | TimelineError::Read(_)
        | TimelineError::Send(_) => CoreError::Failed {
            msg: sentence.to_owned(),
        },
    }
}

/// The FFI's sentence for a failed address edit.
///
/// The application's toasts name the action, so the sentence is the
/// caller's; the three refusals the application names — not registered,
/// another room's, already registered — are rendered from the value.
fn alias_failure(error: crate::session::AliasError, sentence: &str) -> CoreError {
    use crate::session::AliasError;

    match error {
        AliasError::NotRegistered | AliasError::OtherRoom | AliasError::AlreadyInUse => {
            CoreError::from(error)
        }
        AliasError::NothingToDo | AliasError::Read(_) | AliasError::Server(_) => {
            CoreError::Failed {
                msg: sentence.to_owned(),
            }
        }
    }
}

/// The chunks of a message typed in the Kotlin composer.
///
/// The application's composer holds mentions and emoticons as pills, so
/// its parser knows one when it meets one. The Kotlin composer is plain
/// text: this is where `@Name`, `:shortcode:` and `@room` are found in
/// it, the FFI's shortcut rather than the core's behaviour. `@room` is
/// found as the push rules find it, and only where the application's
/// completion would offer the pill: `allow_at_room` is that gate.
fn composer_chunks(
    body: &str,
    mentions: &[FfiMention],
    emoticons: &[FfiSticker],
    allow_at_room: bool,
) -> Vec<crate::session::ComposerChunk> {
    use crate::{
        matrix::{AT_ROOM, find_at_room},
        session::ComposerChunk,
    };

    // Every place a pill would stand: where it starts, where it ends, and
    // what it is.
    let mut pills = Vec::new();

    for mention in mentions {
        let Ok(user_id) = ruma::UserId::parse(&mention.user_id) else {
            continue;
        };
        let at_name = format!("@{}", mention.display_name);
        for (start, _) in body.match_indices(&at_name) {
            pills.push((
                start,
                start + at_name.len(),
                ComposerChunk::user_mention(&user_id, Some(&mention.display_name)),
            ));
        }
    }

    for emoticon in emoticons {
        let plain = crate::session::emoticon_plain(&emoticon.shortcode);
        for (start, _) in body.match_indices(&plain) {
            pills.push((
                start,
                start + plain.len(),
                ComposerChunk::Emoticon {
                    shortcode: emoticon.shortcode.clone(),
                    uri: emoticon.url.clone(),
                    body: emoticon.body.clone(),
                },
            ));
        }
    }

    let mut from = 0;
    while allow_at_room && let Some(offset) = find_at_room(&body[from..]) {
        let start = from + offset;
        let end = start + AT_ROOM.len();
        pills.push((start, end, ComposerChunk::AtRoom));
        from = end;
    }

    // Earliest first; where two overlap, the longer wins.
    pills.sort_by(|(a_start, a_end, _), (b_start, b_end, _)| {
        a_start.cmp(b_start).then(b_end.cmp(a_end))
    });

    let mut chunks = Vec::new();
    let mut pos = 0;
    for (start, end, chunk) in pills {
        if start < pos {
            continue;
        }
        if start > pos {
            chunks.push(ComposerChunk::Text(body[pos..start].to_owned()));
        }
        chunks.push(chunk);
        pos = end;
    }
    if pos < body.len() {
        chunks.push(ComposerChunk::Text(body[pos..].to_owned()));
    }

    chunks
}

/// Whether our own user may send the given state event in the room.
///
/// The application's `Permissions::is_allowed_to`, after the permissions
/// are loaded — the pages ask it on every change.
async fn can_send_state(
    room: &crate::session::Room,
    event_type: ruma::events::StateEventType,
) -> bool {
    use ruma::events::room::power_levels::PowerLevelAction;

    let permissions = room.permissions();
    permissions.ensure_loaded().await;
    permissions.is_allowed_to(PowerLevelAction::SendState(event_type))
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "the page's three version flags and one permission, as the bindings carry them"
)]
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

/// The name this embedder presents itself to homeservers under.
const APP_NAME: &str = "Commune";

/// The fixed Android redirect URI login flows come back on — the
/// application's own, from its `login/local_server.rs`: a custom scheme
/// rather than the loopback address other platforms use, because no
/// browser on Android will follow a redirect to another application's
/// loopback listener. `MainActivity` receives it as an `Intent`.
const ANDROID_REDIRECT_URI: &str = "io.github.steeb-k.commune:/oauth2redirect";

/// The client URI the Android registration is made under.
///
/// matrix.org's authorization server requires the client URI of a native
/// client whose redirect scheme is not `https` to match that scheme read as
/// reverse DNS: `steeb-k.github.io` reversed is `io.github.steeb-k`, which
/// is the domain the scheme above is derived from.
const ANDROID_OAUTH_CLIENT_URI: &str = "https://steeb-k.github.io/";

/// The Android redirect URI, parsed.
fn android_redirect_uri() -> url::Url {
    url::Url::parse(ANDROID_REDIRECT_URI).expect("redirect URI is valid")
}

/// Whether the account has a crypto identity, and whether this session
/// can verify against another of its own.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum FfiCryptoIdentityState {
    /// Not known yet.
    Unknown,
    /// Cross-signing was never set up for this account.
    Missing,
    /// There are no other verified sessions to verify against.
    LastManStanding,
    /// There are other verified sessions.
    OtherSessions,
}

/// Whether this session itself is verified.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum FfiVerificationState {
    /// Not known yet.
    Unknown,
    /// This session is verified.
    Verified,
    /// This session is not verified.
    Unverified,
}

/// Where a session stands on encryption, as the setup view reads it.
#[derive(uniffi::Record)]
pub struct FfiSecurityState {
    /// The account's crypto identity.
    pub identity: FfiCryptoIdentityState,
    /// Whether this session is verified.
    pub verification: FfiVerificationState,
    /// Whether account recovery is set up.
    pub recovery: FfiRecoveryState,
}

/// One session on this device.
#[derive(uniffi::Record)]
pub struct FfiSessionInfo {
    /// The local identifier of the session.
    pub session_id: String,
    /// The user the session belongs to (empty until it is ready).
    pub user_id: String,
    /// The homeserver the session lives on (empty until it is ready).
    pub homeserver: String,
    /// Whether the session is restored and running.
    pub ready: bool,
    /// Whether this is the session everything resolves to right now.
    pub active: bool,
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

    /// The active session — or, when none was chosen or the chosen one
    /// is gone, the first session that is ready.
    fn first_ready_session(&self) -> Option<Session> {
        self.session_list.active_session()
    }

    /// The call with the given ID, if it is the one that is happening.
    fn call(&self, call_id: &str) -> Result<crate::session::Call, CoreError> {
        let session = self.session()?;

        session
            .calls()
            .call_with_id(call_id)
            .ok_or_else(|| CoreError::from(crate::session::CallError::NoSuchCall))
    }

    /// The active session, or the error the FFI reports when there is
    /// none.
    ///
    /// The same four lines were written out 66 times before Phase 3; every
    /// method that resolves one is expected to come through here.
    fn session(&self) -> Result<Session, CoreError> {
        self.first_ready_session().ok_or_else(|| CoreError::Failed {
            msg: "No session".to_owned(),
        })
    }

    /// The room with the given ID in the active session, or the error the
    /// FFI reports when the ID does not parse or the room is not known.
    ///
    /// The other half of the preamble Phase 3 collapses: "Invalid room ID"
    /// was written out 39 times and "Unknown room" 34.
    fn room(&self, room_id: &str) -> Result<crate::session::Room, CoreError> {
        let session = self.session()?;
        let room_id = parse_room_id(room_id)?;

        session
            .room_list()
            .get(&room_id)
            .ok_or_else(|| CoreError::Failed {
                msg: "Unknown room".to_owned(),
            })
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
    /// The login the discovery step started, for the flow's next step.
    fn pending_login_flow(&self) -> Result<crate::login::LoginFlow, CoreError> {
        self.pending_login
            .lock()
            .expect("mutex is not poisoned")
            .clone()
            .ok_or_else(|| CoreError::Failed {
                msg: "No login in progress".to_owned(),
            })
    }

    /// The ongoing verification with the given flow ID.
    fn verification(
        &self,
        flow_id: &str,
    ) -> Result<crate::session::IdentityVerification, CoreError> {
        self.session()?
            .verification_list()
            .snapshot()
            .into_iter()
            .find(|verification| verification.flow_id() == flow_id)
            .ok_or_else(|| CoreError::Failed {
                msg: "No verification in progress".to_owned(),
            })
    }

    /// Make the logged-in client of the given flow a stored session and
    /// the active one — the tail of every login.
    async fn adopt_login(&self, flow: crate::login::LoginFlow) -> Result<(), CoreError> {
        let list = self.session_list.clone();

        // `Session::create` opens the sqlite store and `prepare` starts the
        // sync, so this runs on the runtime.
        let session_id = RUNTIME
            .spawn(async move {
                list.adopt_logged_in_client(flow.into_client())
                    .await
                    .map(|session| session.session_id().to_owned())
            })
            .await
            .expect("task was not aborted")
            .map_err(|adopt_error| CoreError::Failed {
                msg: crate::UserFacingError::to_user_facing(&adopt_error),
            })?;

        self.set_active_session(session_id);
        Ok(())
    }
}

/// What became of a call, as far as the room can tell.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum FfiCallOutcome {
    /// An invite was seen and nothing has happened to it yet.
    Ringing,
    /// Somebody answered it.
    Answered,
    /// Somebody said no to it.
    Declined,
    /// It stopped ringing without being answered.
    Missed,
}

/// One ICE candidate, in both spellings the wild asks for.
#[derive(uniffi::Record, Clone)]
pub struct FfiIceCandidate {
    /// The candidate line.
    pub candidate: String,
    /// The media stream it belongs to.
    pub sdp_mid: Option<String>,
    /// The index of the media line it belongs to.
    pub sdp_m_line_index: u32,
}

/// Why a call ended, as far as the other end said.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum FfiCallEnd {
    /// Somebody hung up.
    HungUp,
    /// The other party declined.
    Declined,
    /// Another of our own sessions took it.
    AnsweredElsewhere,
}

/// The ICE servers a homeserver hands out.
#[derive(uniffi::Record, Default)]
pub struct FfiTurnServers {
    /// The server URIs, `stun:` and `turn:` as the server gave them.
    pub uris: Vec<String>,
    /// The username for the TURN servers.
    pub username: String,
    /// The password for the TURN servers.
    pub password: String,
    /// How long the credentials last.
    pub ttl_seconds: u64,
}

/// What the embedder is told about calls. The core speaks `m.call.*`;
/// the media itself is the embedder's business.
#[uniffi::export(with_foreign)]
pub trait CallListener: Send + Sync {
    /// Somebody is calling, with the description they offered.
    fn on_incoming(&self, call_id: String, room_id: String, caller: String, sdp: String);
    /// A call we placed was answered, with the description to apply.
    fn on_answer(&self, call_id: String, sdp: String);
    /// The other end gathered candidates.
    fn on_candidates(&self, call_id: String, candidates: Vec<FfiIceCandidate>);
    /// The other end wants to renegotiate: apply the description, and
    /// when it is an offer answer it with `send_call_negotiate`.
    fn on_negotiate(&self, call_id: String, sdp: String, session_type: String);
    /// The call is over.
    fn on_ended(&self, call_id: String, reason: FfiCallEnd);
}

/// What feeds the foreign call listener from the core's calls.
///
/// The core's `Calls` is the state; this follows the one active call and
/// turns the application's events and states into the listener's five
/// calls.
#[derive(Default)]
struct CallBridge {
    listener: Mutex<Option<Arc<dyn CallListener>>>,
    /// The task following the active call.
    watch_handle: Mutex<Option<tokio::task::AbortHandle>>,
}

impl CallBridge {
    fn emit(&self, f: impl FnOnce(&Arc<dyn CallListener>)) {
        if let Some(listener) = self
            .listener
            .lock()
            .expect("mutex is not poisoned")
            .as_ref()
        {
            f(listener);
        }
    }

    /// Follow one call to its end.
    ///
    /// An incoming call is reported with its offer; from then on what the
    /// other end sends is handed over, and the end is reported with the
    /// closest of the listener's three reasons.
    fn follow(self: Arc<Self>, call: crate::session::Call) {
        use crate::session::{CallEvent, CallState};

        RUNTIME.spawn(async move {
            let call_id = call.call_id().to_string();
            let mut events = call.subscribe_events();
            let mut states = call.subscribe_state();

            if !call.is_outgoing()
                && let Some(offer) = call.pending_offer()
            {
                // A call that took over from ours in a glare is one the
                // application answers on the spot; this side has no way
                // to say so, and it rings.
                let room_id = call.room().room_id().to_string();
                let caller = call
                    .remote_user_id()
                    .map(|user_id| user_id.to_string())
                    .unwrap_or_default();
                let call_id = call_id.clone();
                self.emit(move |listener| {
                    listener.on_incoming(call_id, room_id, caller, offer.sdp);
                });
            }

            loop {
                if call.state().is_ended() {
                    let reason = ffi_call_end(call.end_reason());
                    let call_id = call_id.clone();
                    self.emit(move |listener| listener.on_ended(call_id, reason));
                    break;
                }

                tokio::select! {
                    event = events.recv() => match event {
                        Ok(CallEvent::Answer { sdp }) => {
                            let call_id = call_id.clone();
                            self.emit(move |listener| listener.on_answer(call_id, sdp));
                        }
                        Ok(CallEvent::Candidates(candidates)) => {
                            let candidates: Vec<FfiIceCandidate> = candidates
                                .iter()
                                // The empty candidate means "that is all of
                                // them"; WebRTC has nothing to do with it.
                                .filter(|candidate| !candidate.candidate.is_empty())
                                .map(|candidate| FfiIceCandidate {
                                    candidate: candidate.candidate.clone(),
                                    sdp_mid: candidate.sdp_mid.clone(),
                                    sdp_m_line_index: candidate
                                        .sdp_m_line_index
                                        .and_then(|index| u32::try_from(u64::from(index)).ok())
                                        .unwrap_or(0),
                                })
                                .collect();
                            if candidates.is_empty() {
                                continue;
                            }
                            let call_id = call_id.clone();
                            self.emit(move |listener| listener.on_candidates(call_id, candidates));
                        }
                        Ok(CallEvent::Negotiate { sdp, is_answer }) => {
                            let session_type = if is_answer { "answer" } else { "offer" };
                            let call_id = call_id.clone();
                            self.emit(move |listener| {
                                listener.on_negotiate(call_id, sdp, session_type.to_owned());
                            });
                        }
                        // The callee's crossed offer gives way; the
                        // listener has no call for a rollback, and the
                        // embedder's WebRTC settles it on its own.
                        Ok(CallEvent::RollbackLocalDescription)
                        | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    state = states.next() => {
                        if state.is_none() || state == Some(CallState::Ended) {
                            let reason = ffi_call_end(call.end_reason());
                            let call_id = call_id.clone();
                            self.emit(move |listener| listener.on_ended(call_id, reason));
                            break;
                        }
                    }
                }
            }
        });
    }
}

/// The listener's reason for a call ending, the closest of its three.
fn ffi_call_end(reason: crate::session::CallEndReason) -> FfiCallEnd {
    use crate::session::CallEndReason;

    match reason {
        CallEndReason::Declined => FfiCallEnd::Declined,
        CallEndReason::AnsweredElsewhere => FfiCallEnd::AnsweredElsewhere,
        CallEndReason::HungUp
        | CallEndReason::NotAnswered
        | CallEndReason::NoConnection
        | CallEndReason::MediaFailed
        | CallEndReason::Failed => FfiCallEnd::HungUp,
    }
}

impl From<crate::session::CallOutcome> for FfiCallOutcome {
    fn from(outcome: crate::session::CallOutcome) -> Self {
        use crate::session::CallOutcome;

        match outcome {
            CallOutcome::Ringing => Self::Ringing,
            CallOutcome::Answered => Self::Answered,
            CallOutcome::Declined => Self::Declined,
            CallOutcome::Missed => Self::Missed,
        }
    }
}

/// What became of the calls the session saw, keyed as the timeline items
/// carry the call ID.
fn ffi_call_outcomes(session: &Session) -> std::collections::HashMap<String, FfiCallOutcome> {
    session
        .calls()
        .outcomes_snapshot()
        .into_iter()
        .map(|(call_id, outcome)| (call_id.to_string(), outcome.into()))
        .collect()
}

/// What feeds the foreign verification listener from the core's list.
///
/// The core's `VerificationList` is the state; this is the task that
/// follows each verification it holds and turns the application's states
/// into the four calls the listener has.
#[derive(Default)]
struct VerificationBridge {
    listener: Mutex<Option<Arc<dyn VerificationListener>>>,
    /// The flows already being followed.
    followed: Mutex<std::collections::HashSet<String>>,
    /// The session whose list is being followed, if any.
    bound_session: Mutex<Option<String>>,
    /// The task following the list.
    watch_handle: Mutex<Option<tokio::task::AbortHandle>>,
}

impl VerificationBridge {
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

    /// Follow one verification to its end, reporting what the listener can
    /// take: the request, the emojis, done, or cancelled.
    ///
    /// Where the application asks the user to choose a method, this side
    /// has no page for it and starts SAS — the one method it can drive to
    /// completion — as the application itself does when SAS is the only
    /// method both sides support.
    fn follow(self: Arc<Self>, verification: crate::session::IdentityVerification) {
        use crate::session::VerificationState;

        RUNTIME.spawn(async move {
            let flow_id = verification.flow_id().to_owned();
            let mut states = verification.subscribe_state();
            let mut reported_request = false;
            let mut reported_emojis = false;

            loop {
                let state = states.get();
                match state {
                    VerificationState::Requested if !reported_request => {
                        reported_request = true;
                        let flow_id = flow_id.clone();
                        let user_id = verification.other_user_id().to_string();
                        self.emit(move |listener| listener.on_request(flow_id, user_id));
                    }
                    VerificationState::Ready if !verification.started_by_us() => {
                        if let Err(start_error) = verification.start_sas().await {
                            tracing::error!("Could not start SAS for {flow_id}: {start_error}");
                        }
                    }
                    VerificationState::SasConfirm if !reported_emojis => {
                        reported_emojis = true;
                        let emojis: Vec<FfiSasEmoji> = verification
                            .sas_emoji()
                            .map(|emojis| {
                                emojis
                                    .iter()
                                    .map(|emoji| FfiSasEmoji {
                                        symbol: emoji.symbol.to_owned(),
                                        description: emoji.description.to_owned(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let flow_id = flow_id.clone();
                        self.emit(move |listener| listener.on_emojis(flow_id, emojis));
                    }
                    VerificationState::Done => {
                        let flow_id = flow_id.clone();
                        self.emit(move |listener| listener.on_done(flow_id));
                        break;
                    }
                    VerificationState::Cancelled
                    | VerificationState::Dismissed
                    | VerificationState::RoomLeft
                    | VerificationState::Error
                    | VerificationState::NoSupportedMethods => {
                        let reason = verification
                            .cancel_info()
                            .map_or_else(|| format!("{state:?}"), |info| info.reason().to_owned());
                        let flow_id = flow_id.clone();
                        self.emit(move |listener| listener.on_cancelled(flow_id, reason));
                        break;
                    }
                    _ => {}
                }

                if states.next().await.is_none() {
                    break;
                }
            }

            self.followed
                .lock()
                .expect("mutex is not poisoned")
                .remove(&flow_id);
        });
    }
}

async fn wait_for_ready_session(list: &SessionList) -> Session {
    let (_, mut stream) = list.subscribe_entries();

    if let Some(session) = list.active_session() {
        return session;
    }

    loop {
        let _ = stream.next().await;

        if let Some(session) = list.active_session() {
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

#[allow(
    clippy::too_many_lines,
    reason = "one match arm per kind of timeline item, and the record built from it"
)]
fn ffi_timeline_item(
    item: &matrix_sdk_ui::timeline::TimelineItem,
    own_user_id: Option<&ruma::UserId>,
    outcomes: &std::collections::HashMap<String, FfiCallOutcome>,
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
                // The invite is the event that says a call happened;
                // the rest of the module is signalling and would be a
                // dozen rows for one call. What became of it is what
                // this session watched happen.
                TimelineItemContent::CallInvite => {
                    let raw = event
                        .original_json()
                        .and_then(|raw| raw.deserialize_as::<serde_json::Value>().ok());
                    let content = raw.as_ref().and_then(|value| value.get("content"));
                    let call_id = content
                        .and_then(|content| content.get("call_id"))
                        .and_then(|id| id.as_str());
                    let sdp = content
                        .and_then(|content| content.pointer("/offer/sdp"))
                        .and_then(|sdp| sdp.as_str())
                        .unwrap_or_default();

                    (
                        FfiEventKind::Call {
                            has_video: crate::session::sdp_has_video(sdp),
                            outcome: call_id.and_then(|id| outcomes.get(id).copied()),
                        },
                        String::new(),
                    )
                }
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

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
        }
    }
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
    },
    /// A sticker.
    Sticker,
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
                let MsgLikeKind::Message(message) = &msg_like.kind else {
                    return None;
                };
                let source = match message.msgtype() {
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

    /// Mark the given room as read, sending a read receipt at the end of
    /// its timeline.
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
                    .send_text(body)
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

    /// Send a plain-text message to the given room.
    pub async fn send_message(&self, room_id: String, body: String) -> Result<(), CoreError> {
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
                    .send_text(body)
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
                client
                    .join_room_by_id_or_alias(&id_or_alias, &[])
                    .await
                    .map(|room| room.room_id().to_string())
                    .map_err(|join_error| CoreError::Failed {
                        msg: format!("Could not join the room: {join_error}"),
                    })
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
            let session = wait_for_ready_session(&list).await;
            let client = session.client();
            use ruma::events::key::verification::{
                request::ToDeviceKeyVerificationRequestEvent,
                start::ToDeviceKeyVerificationStartEvent,
            };

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
}

impl CoreApp {
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
                    MsgLikeKind::Message(message) => {
                        use ruma::events::room::message::MessageType;

                        let msgtype = message.msgtype();
                        let kind = match msgtype {
                            MessageType::Text(_)
                            | MessageType::Notice(_)
                            | MessageType::Emote(_)
                            | MessageType::ServerNotice(_) => FfiEventKind::Text,
                            MessageType::Image(_) => FfiEventKind::Media {
                                kind: FfiMediaKind::Image,
                            },
                            MessageType::Video(_) => FfiEventKind::Media {
                                kind: FfiMediaKind::Video,
                            },
                            MessageType::Audio(_) => FfiEventKind::Media {
                                kind: FfiMediaKind::Audio,
                            },
                            _ => FfiEventKind::Media {
                                kind: FfiMediaKind::File,
                            },
                        };
                        (kind, msgtype.body().to_owned())
                    }
                    MsgLikeKind::Sticker(_) => (FfiEventKind::Sticker, String::new()),
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

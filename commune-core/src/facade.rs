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

/// A timeline item, as the message list needs it.
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
        is_image: bool,
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
    OtherState,
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

                listener.on_update(items.iter().map(|item| ffi_timeline_item(item)).collect());

                let mut items = items;
                while let Some(diffs) = stream.next().await {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                    listener.on_update(items.iter().map(|item| ffi_timeline_item(item)).collect());
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
                let MessageType::Image(image) = message.msgtype() else {
                    return None;
                };

                let request = matrix_sdk::media::MediaRequestParameters {
                    source: image.source.clone(),
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

                listener.on_update(items.iter().map(|item| ffi_timeline_item(item)).collect());

                let mut items = items;
                while let Some(diffs) = stream.next().await {
                    for diff in diffs {
                        diff.apply(&mut items);
                    }
                    listener.on_update(items.iter().map(|item| ffi_timeline_item(item)).collect());
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
fn ffi_timeline_item(item: &matrix_sdk_ui::timeline::TimelineItem) -> FfiTimelineItem {
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
                            MessageType::Image(_) => FfiEventKind::Media { is_image: true },
                            _ => FfiEventKind::Media { is_image: false },
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
                TimelineItemContent::OtherState(_) => (FfiEventKind::OtherState, String::new()),
                _ => (FfiEventKind::Unsupported, String::new()),
            };

            let thread_replies = match event.content() {
                TimelineItemContent::MsgLike(msg_like) => msg_like
                    .thread_summary
                    .as_ref()
                    .map_or(0, |summary| u64::from(summary.num_replies)),
                _ => 0,
            };

            FfiTimelineItem::Event {
                unique_id: item.unique_id().0.clone(),
                event_id: event.event_id().map(ToString::to_string),
                thread_replies,
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

//! Collection of methods related to the Matrix specification.

use std::{borrow::Cow, str::FromStr};

use gettextrs::gettext;
use gtk::{glib, prelude::*};
use matrix_sdk::{
    authentication::matrix::MatrixSession,
    config::RequestConfig,
    deserialized_responses::RawAnySyncOrStrippedTimelineEvent,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    Client, ClientBuildError, SessionMeta,
};
use ruma::{
    events::{
        room::{member::MembershipState, message::MessageType},
        AnyMessageLikeEventContent, AnyStrippedStateEvent, AnySyncMessageLikeEvent,
        AnySyncTimelineEvent,
    },
    html::{
        matrix::{AnchorUri, MatrixElement},
        Children, Html, HtmlSanitizerMode, NodeRef, RemoveReplyFallback, StrTendril,
    },
    matrix_uri::MatrixId,
    serde::Raw,
    EventId, IdParseError, MatrixToUri, MatrixUri, MatrixUriError, MilliSecondsSinceUnixEpoch,
    OwnedEventId, OwnedRoomAliasId, OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName, OwnedUserId,
    RoomId, RoomOrAliasId, UserId,
};
use thiserror::Error;
use tracing::error;

pub mod ext_traits;
mod media_message;

pub use self::media_message::{MediaMessage, VisualMediaMessage};
use crate::{
    components::Pill,
    gettext_f,
    prelude::*,
    secret::{SessionTokens, StoredSession},
    session::model::{RemoteRoom, Room},
};

/// The result of a password validation.
#[derive(Debug, Default, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
pub struct PasswordValidity {
    /// Whether the password includes at least one lowercase letter.
    pub has_lowercase: bool,
    /// Whether the password includes at least one uppercase letter.
    pub has_uppercase: bool,
    /// Whether the password includes at least one number.
    pub has_number: bool,
    /// Whether the password includes at least one symbol.
    pub has_symbol: bool,
    /// Whether the password is at least 8 characters long.
    pub has_length: bool,
    /// The percentage of checks passed for the password, between 0 and 100.
    ///
    /// If progress is 100, the password is valid.
    pub progress: u32,
}

impl PasswordValidity {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Validate a password according to the Matrix specification.
///
/// A password should include a lower-case letter, an upper-case letter, a
/// number and a symbol and be at a minimum 8 characters in length.
///
/// See: <https://spec.matrix.org/v1.1/client-server-api/#notes-on-password-management>
pub fn validate_password(password: &str) -> PasswordValidity {
    let mut validity = PasswordValidity::new();

    for char in password.chars() {
        if char.is_numeric() {
            validity.has_number = true;
        } else if char.is_lowercase() {
            validity.has_lowercase = true;
        } else if char.is_uppercase() {
            validity.has_uppercase = true;
        } else {
            validity.has_symbol = true;
        }
    }

    validity.has_length = password.len() >= 8;

    let mut passed = 0;
    if validity.has_number {
        passed += 1;
    }
    if validity.has_lowercase {
        passed += 1;
    }
    if validity.has_uppercase {
        passed += 1;
    }
    if validity.has_symbol {
        passed += 1;
    }
    if validity.has_length {
        passed += 1;
    }
    validity.progress = passed * 100 / 5;

    validity
}

/// An deserialized event received in a sync response.
#[derive(Debug, Clone)]
pub enum AnySyncOrStrippedTimelineEvent {
    /// An event from a joined or left room.
    Sync(AnySyncTimelineEvent),
    /// An event from an invited room.
    Stripped(AnyStrippedStateEvent),
}

impl AnySyncOrStrippedTimelineEvent {
    /// Deserialize the given raw event.
    pub fn from_raw(raw: &RawAnySyncOrStrippedTimelineEvent) -> Result<Self, serde_json::Error> {
        let ev = match raw {
            RawAnySyncOrStrippedTimelineEvent::Sync(ev) => Self::Sync(ev.deserialize()?),
            RawAnySyncOrStrippedTimelineEvent::Stripped(ev) => Self::Stripped(ev.deserialize()?),
        };

        Ok(ev)
    }

    /// The sender of the event.
    pub fn sender(&self) -> &UserId {
        match self {
            AnySyncOrStrippedTimelineEvent::Sync(ev) => ev.sender(),
            AnySyncOrStrippedTimelineEvent::Stripped(ev) => ev.sender(),
        }
    }

    /// The ID of the event, if it's not a stripped state event.
    pub fn event_id(&self) -> Option<&EventId> {
        match self {
            AnySyncOrStrippedTimelineEvent::Sync(ev) => Some(ev.event_id()),
            AnySyncOrStrippedTimelineEvent::Stripped(_) => None,
        }
    }
}

/// Extract the body from the given event.
///
/// If the event does not have a body but is supported, this will return a
/// localized string.
///
/// Returns `None` if the event type is not supported.
pub fn get_event_body(
    event: &AnySyncOrStrippedTimelineEvent,
    sender_name: &str,
    own_user: &UserId,
    show_sender: bool,
) -> Option<String> {
    match event {
        AnySyncOrStrippedTimelineEvent::Sync(AnySyncTimelineEvent::MessageLike(message)) => {
            get_message_event_body(message, sender_name, show_sender)
        }
        AnySyncOrStrippedTimelineEvent::Sync(_) => None,
        AnySyncOrStrippedTimelineEvent::Stripped(state) => {
            get_stripped_state_event_body(state, sender_name, own_user)
        }
    }
}

/// Extract the body from the given message event.
///
/// If it's a media message, this will return a localized body.
///
/// Returns `None` if the message type is not supported.
pub fn get_message_event_body(
    event: &AnySyncMessageLikeEvent,
    sender_name: &str,
    show_sender: bool,
) -> Option<String> {
    match event.original_content()? {
        AnyMessageLikeEventContent::RoomMessage(mut message) => {
            message.sanitize(HtmlSanitizerMode::Compat, RemoveReplyFallback::Yes);

            let body = match message.msgtype {
                MessageType::Audio(_) => {
                    gettext_f("{user} sent an audio file.", &[("user", sender_name)])
                }
                MessageType::Emote(content) => format!("{sender_name} {}", content.body),
                MessageType::File(_) => gettext_f("{user} sent a file.", &[("user", sender_name)]),
                MessageType::Image(_) => {
                    gettext_f("{user} sent an image.", &[("user", sender_name)])
                }
                MessageType::Location(_) => {
                    gettext_f("{user} sent their location.", &[("user", sender_name)])
                }
                MessageType::Notice(content) => {
                    text_event_body(content.body, sender_name, show_sender)
                }
                MessageType::ServerNotice(content) => {
                    text_event_body(content.body, sender_name, show_sender)
                }
                MessageType::Text(content) => {
                    text_event_body(content.body, sender_name, show_sender)
                }
                MessageType::Video(_) => {
                    gettext_f("{user} sent a video.", &[("user", sender_name)])
                }
                _ => return None,
            };
            Some(body)
        }
        AnyMessageLikeEventContent::Sticker(_) => Some(gettext_f(
            "{user} sent a sticker.",
            &[("user", sender_name)],
        )),
        _ => None,
    }
}

fn text_event_body(message: String, sender_name: &str, show_sender: bool) -> String {
    if show_sender {
        gettext_f(
            "{user}: {message}",
            &[("user", sender_name), ("message", &message)],
        )
    } else {
        message
    }
}

/// Extract the body from the given state event.
///
/// This will return a localized body.
///
/// Returns `None` if the state event type is not supported.
pub fn get_stripped_state_event_body(
    event: &AnyStrippedStateEvent,
    sender_name: &str,
    own_user: &UserId,
) -> Option<String> {
    if let AnyStrippedStateEvent::RoomMember(member_event) = event {
        if member_event.content.membership == MembershipState::Invite
            && member_event.state_key == own_user
        {
            // Translators: Do NOT translate the content between '{' and '}', this is a
            // variable name.
            return Some(gettext_f("{user} invited you", &[("user", sender_name)]));
        }
    }

    None
}

/// All errors that can occur when setting up the Matrix client.
#[derive(Error, Debug)]
pub enum ClientSetupError {
    /// An error when building the client.
    #[error(transparent)]
    Client(#[from] ClientBuildError),
    /// An error when using the client.
    #[error(transparent)]
    Sdk(#[from] matrix_sdk::Error),
    /// An error creating the unique local ID of the session.
    #[error("Could not generate unique session ID")]
    NoSessionId,
    /// An error accessing the session tokens.
    #[error("Could not access session tokens")]
    NoSessionTokens,
}

impl UserFacingError for ClientSetupError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::Client(err) => err.to_user_facing(),
            Self::Sdk(err) => err.to_user_facing(),
            Self::NoSessionId => gettext("Could not generate unique session ID"),
            Self::NoSessionTokens => gettext("Could not access the session tokens"),
        }
    }
}

/// Create a [`Client`] with the given stored session.
pub async fn client_with_stored_session(
    session: StoredSession,
    tokens: SessionTokens,
) -> Result<Client, ClientSetupError> {
    let has_refresh_token = tokens.refresh_token.is_some();
    let data_path = session.data_path();
    let cache_path = session.cache_path();

    let StoredSession {
        homeserver,
        user_id,
        device_id,
        passphrase,
        ..
    } = session;

    let session_data = MatrixSession {
        meta: SessionMeta { user_id, device_id },
        tokens: tokens.into(),
    };

    let encryption_settings = EncryptionSettings {
        auto_enable_cross_signing: true,
        backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
        auto_enable_backups: true,
    };

    let mut client_builder = Client::builder()
        .homeserver_url(homeserver)
        .sqlite_store_with_cache_path(data_path, cache_path, Some(&passphrase))
        // force_auth option to solve an issue with some servers configuration to require
        // auth for profiles:
        // https://gitlab.gnome.org/World/fractal/-/issues/934
        .request_config(RequestConfig::new().retry_limit(2).force_auth())
        .with_encryption_settings(encryption_settings);

    if has_refresh_token {
        client_builder = client_builder.handle_refresh_tokens();
    }

    let client = client_builder.build().await?;

    client.restore_session(session_data).await?;

    if let Err(error) = client.event_cache().enable_storage() {
        error!("Failed to enable event cache storage: {error}");
    }

    Ok(client)
}

/// Find mentions in the given HTML string.
///
/// Returns a list of `(pill, mention_content)` tuples.
pub fn find_html_mentions(html: &str, room: &Room) -> Vec<(Pill, StrTendril)> {
    let mut mentions = Vec::new();
    let html = Html::parse(html);

    append_children_mentions(&mut mentions, html.children(), room);

    mentions
}

/// Find mentions in the given child nodes and append them to the given list.
fn append_children_mentions(
    mentions: &mut Vec<(Pill, StrTendril)>,
    children: Children,
    room: &Room,
) {
    for node in children {
        if let Some(mention) = node_as_mention(&node, room) {
            mentions.push(mention);
            continue;
        }

        append_children_mentions(mentions, node.children(), room);
    }
}

/// Try to convert the given node to a mention.
///
/// This does not recurse into children.
fn node_as_mention(node: &NodeRef, room: &Room) -> Option<(Pill, StrTendril)> {
    // Mentions are links.
    let MatrixElement::A(anchor) = node.as_element()?.to_matrix().element else {
        return None;
    };

    // Mentions contain Matrix URIs.
    let id = MatrixIdUri::try_from(anchor.href?).ok()?;

    // Mentions contain one text child node.
    let child = node.children().next()?;

    if child.next_sibling().is_some() {
        return None;
    }

    let content = child.as_text()?.borrow().clone();
    let pill = id.into_pill(room)?;

    Some((pill, content))
}

/// The textual representation of a room mention.
pub const AT_ROOM: &str = "@room";

/// Find `@room` in the given string.
///
/// This uses the same algorithm as the pushrules from the Matrix spec to detect
/// it in the `body`.
///
/// Returns the position of the first match.
pub fn find_at_room(s: &str) -> Option<usize> {
    for (pos, _) in s.match_indices(AT_ROOM) {
        let is_at_word_start = pos == 0 || s[..pos].ends_with(char_is_ascii_word_boundary);
        if !is_at_word_start {
            continue;
        }

        let pos_after_match = pos + 5;
        let is_at_word_end = pos_after_match == s.len()
            || s[pos_after_match..].starts_with(char_is_ascii_word_boundary);
        if is_at_word_end {
            return Some(pos);
        }
    }

    None
}

/// Whether the given `char` is a word boundary, according to the Matrix spec.
///
/// A word boundary is any character not in the sets `[A-Z]`, `[a-z]`, `[0-9]`
/// or `_`.
fn char_is_ascii_word_boundary(c: char) -> bool {
    !c.is_ascii_alphanumeric() && c != '_'
}

/// Compare two raw JSON sources.
pub fn raw_eq<T, U>(lhs: Option<&Raw<T>>, rhs: Option<&Raw<U>>) -> bool {
    let Some(lhs) = lhs else {
        // They are equal only if both are `None`.
        return rhs.is_none();
    };
    let Some(rhs) = rhs else {
        // They cannot be equal.
        return false;
    };

    lhs.json().get() == rhs.json().get()
}

/// A URI for a Matrix ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatrixIdUri {
    /// A room.
    Room(MatrixRoomIdUri),
    /// A user.
    User(OwnedUserId),
    /// An event.
    Event(MatrixEventIdUri),
}

impl MatrixIdUri {
    /// Constructs a `MatrixIdUri` from the given ID and servers list.
    fn try_from_parts(id: MatrixId, via: &[OwnedServerName]) -> Result<Self, ()> {
        let uri = match id {
            MatrixId::Room(room_id) => Self::Room(MatrixRoomIdUri {
                id: room_id.into(),
                via: via.to_owned(),
            }),
            MatrixId::RoomAlias(room_alias) => Self::Room(MatrixRoomIdUri {
                id: room_alias.into(),
                via: via.to_owned(),
            }),
            MatrixId::User(user_id) => Self::User(user_id),
            MatrixId::Event(room_id, event_id) => Self::Event(MatrixEventIdUri {
                event_id,
                room_uri: MatrixRoomIdUri {
                    id: room_id,
                    via: via.to_owned(),
                },
            }),
            _ => return Err(()),
        };

        Ok(uri)
    }

    /// Try parsing a `&str` into a `MatrixIdUri`.
    pub fn parse(s: &str) -> Result<Self, MatrixIdUriParseError> {
        if let Ok(uri) = MatrixToUri::parse(s) {
            return uri.try_into();
        }

        MatrixUri::parse(s)?.try_into()
    }

    /// Try to construct a [`Pill`] from this ID in the given room.
    pub fn into_pill(self, room: &Room) -> Option<Pill> {
        match self {
            Self::Room(room_uri) => {
                let session = room.session()?;
                session
                    .room_list()
                    .get_by_identifier(&room_uri.id)
                    .as_ref()
                    .map(Pill::new)
                    .or_else(|| Some(Pill::new(&RemoteRoom::new(&session, room_uri))))
            }
            Self::User(user_id) => {
                // We should have a strong reference to the list wherever we show a user pill,
                // so we can use `get_or_create_members()`.
                let user = room.get_or_create_members().get_or_create(user_id);
                Some(Pill::new(&user))
            }
            Self::Event(_) => None,
        }
    }
}

impl TryFrom<&MatrixUri> for MatrixIdUri {
    type Error = MatrixIdUriParseError;

    fn try_from(uri: &MatrixUri) -> Result<Self, Self::Error> {
        // We ignore the action, because we always offer to join a room or DM a user.
        Self::try_from_parts(uri.id().clone(), uri.via())
            .map_err(|()| MatrixIdUriParseError::UnsupportedId(uri.id().clone()))
    }
}

impl TryFrom<MatrixUri> for MatrixIdUri {
    type Error = MatrixIdUriParseError;

    fn try_from(uri: MatrixUri) -> Result<Self, Self::Error> {
        Self::try_from(&uri)
    }
}

impl TryFrom<&MatrixToUri> for MatrixIdUri {
    type Error = MatrixIdUriParseError;

    fn try_from(uri: &MatrixToUri) -> Result<Self, Self::Error> {
        Self::try_from_parts(uri.id().clone(), uri.via())
            .map_err(|()| MatrixIdUriParseError::UnsupportedId(uri.id().clone()))
    }
}

impl TryFrom<MatrixToUri> for MatrixIdUri {
    type Error = MatrixIdUriParseError;

    fn try_from(uri: MatrixToUri) -> Result<Self, Self::Error> {
        Self::try_from(&uri)
    }
}

impl FromStr for MatrixIdUri {
    type Err = MatrixIdUriParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<&str> for MatrixIdUri {
    type Error = MatrixIdUriParseError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

impl TryFrom<&AnchorUri> for MatrixIdUri {
    type Error = MatrixIdUriParseError;

    fn try_from(value: &AnchorUri) -> Result<Self, Self::Error> {
        match value {
            AnchorUri::Matrix(uri) => MatrixIdUri::try_from(uri),
            AnchorUri::MatrixTo(uri) => MatrixIdUri::try_from(uri),
            // The same error that should be returned by `parse()` when parsing a non-Matrix URI.
            _ => Err(IdParseError::InvalidMatrixUri(MatrixUriError::WrongScheme).into()),
        }
    }
}

impl TryFrom<AnchorUri> for MatrixIdUri {
    type Error = MatrixIdUriParseError;

    fn try_from(value: AnchorUri) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}

impl StaticVariantType for MatrixIdUri {
    fn static_variant_type() -> Cow<'static, glib::VariantTy> {
        String::static_variant_type()
    }
}

impl FromVariant for MatrixIdUri {
    fn from_variant(variant: &glib::Variant) -> Option<Self> {
        Self::parse(&variant.get::<String>()?).ok()
    }
}

/// A URI for a Matrix room ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRoomIdUri {
    /// The room ID.
    pub id: OwnedRoomOrAliasId,
    /// Matrix servers usable to route a `RoomId`.
    pub via: Vec<OwnedServerName>,
}

impl MatrixRoomIdUri {
    /// Try parsing a `&str` into a `MatrixRoomIdUri`.
    pub fn parse(s: &str) -> Option<MatrixRoomIdUri> {
        MatrixIdUri::parse(s)
            .ok()
            .and_then(|uri| match uri {
                MatrixIdUri::Room(room_uri) => Some(room_uri),
                _ => None,
            })
            .or_else(|| RoomOrAliasId::parse(s).ok().map(Into::into))
    }
}

impl From<OwnedRoomOrAliasId> for MatrixRoomIdUri {
    fn from(id: OwnedRoomOrAliasId) -> Self {
        Self {
            id,
            via: Vec::new(),
        }
    }
}

impl From<OwnedRoomId> for MatrixRoomIdUri {
    fn from(value: OwnedRoomId) -> Self {
        OwnedRoomOrAliasId::from(value).into()
    }
}

impl From<OwnedRoomAliasId> for MatrixRoomIdUri {
    fn from(value: OwnedRoomAliasId) -> Self {
        OwnedRoomOrAliasId::from(value).into()
    }
}

impl From<&MatrixRoomIdUri> for MatrixUri {
    fn from(value: &MatrixRoomIdUri) -> Self {
        match <&RoomId>::try_from(&*value.id) {
            Ok(room_id) => room_id.matrix_uri_via(value.via.clone(), false),
            Err(alias) => alias.matrix_uri(false),
        }
    }
}

/// A URI for a Matrix event ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixEventIdUri {
    /// The event ID.
    pub event_id: OwnedEventId,
    /// The event's room ID URI.
    pub room_uri: MatrixRoomIdUri,
}

/// Errors encountered when parsing a Matrix ID URI.
#[derive(Debug, Clone, Error)]
pub enum MatrixIdUriParseError {
    /// Not a valid Matrix URI.
    #[error(transparent)]
    InvalidUri(#[from] IdParseError),
    /// Unsupported Matrix ID.
    #[error("unsupported Matrix ID: {0:?}")]
    UnsupportedId(MatrixId),
}

/// Convert the given timestamp to a `GDateTime`.
pub(crate) fn timestamp_to_date(ts: MilliSecondsSinceUnixEpoch) -> glib::DateTime {
    seconds_since_unix_epoch_to_date(ts.as_secs().into())
}

/// Convert the given number of seconds since Unix EPOCH to a `GDateTime`.
pub(crate) fn seconds_since_unix_epoch_to_date(secs: i64) -> glib::DateTime {
    glib::DateTime::from_unix_utc(secs)
        .and_then(|date| date.to_local())
        .expect("constructing GDateTime from timestamp should work")
}

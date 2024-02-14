//! Collection of methods related to the Matrix specification.

use std::{
    fmt::{self, Write},
    str::FromStr,
};

use html2pango::html_escape;
use html5gum::{HtmlString, Token, Tokenizer};
use matrix_sdk::{
    config::RequestConfig, deserialized_responses::RawAnySyncOrStrippedTimelineEvent, Client,
    ClientBuildError,
};
use ruma::{
    events::{
        room::{member::MembershipState, message::MessageType},
        AnyMessageLikeEventContent, AnyStrippedStateEvent, AnySyncMessageLikeEvent,
        AnySyncTimelineEvent,
    },
    html::{HtmlSanitizerMode, RemoveReplyFallback},
    matrix_uri::MatrixId,
    serde::Raw,
    EventId, IdParseError, MatrixToUri, MatrixUri, OwnedEventId, OwnedRoomAliasId, OwnedRoomId,
    OwnedRoomOrAliasId, OwnedServerName, OwnedUserId, RoomOrAliasId, UserId,
};
use thiserror::Error;

use super::media::filename_for_mime;
use crate::{
    components::{Pill, DEFAULT_PLACEHOLDER},
    gettext_f,
    prelude::*,
    secret::StoredSession,
    session::model::{RemoteRoom, Room, Session},
    spawn_tokio,
};

/// The result of a password validation.
#[derive(Debug, Default, Clone, Copy)]
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
        AnySyncOrStrippedTimelineEvent::Stripped(state) => {
            get_stripped_state_event_body(state, sender_name, own_user)
        }
        _ => None,
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
                MessageType::VerificationRequest(_) => gettext_f(
                    "{user} sent a verification request.",
                    &[("user", sender_name)],
                ),
                _ => unimplemented!(),
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
    #[error(transparent)]
    Client(#[from] ClientBuildError),
    #[error(transparent)]
    Sdk(#[from] matrix_sdk::Error),
}

impl UserFacingError for ClientSetupError {
    fn to_user_facing(&self) -> String {
        match self {
            ClientSetupError::Client(err) => err.to_user_facing(),
            ClientSetupError::Sdk(err) => err.to_user_facing(),
        }
    }
}

/// Create a [`Client`] with the given stored session.
pub async fn client_with_stored_session(
    session: StoredSession,
) -> Result<Client, ClientSetupError> {
    let (homeserver, path, passphrase, data) = session.into_parts();

    let client = Client::builder()
        .homeserver_url(homeserver)
        .sqlite_store(path, Some(&passphrase))
        // force_auth option to solve an issue with some servers configuration to require
        // auth for profiles:
        // https://gitlab.gnome.org/World/fractal/-/issues/934
        .request_config(RequestConfig::new().retry_limit(2).force_auth())
        .build()
        .await?;

    client.restore_session(data).await?;

    Ok(client)
}

/// Fetch the content of the media message in the given message.
///
/// Compatible messages:
///
/// - File.
/// - Image.
/// - Video.
/// - Audio.
///
/// Returns `Ok((filename, binary_content))` on success.
///
/// Returns `Err` if an error occurred while fetching the content. Panics on
/// an incompatible event.
pub async fn get_media_content(
    client: Client,
    message: MessageType,
) -> Result<(String, Vec<u8>), matrix_sdk::Error> {
    let media = client.media();

    match message {
        MessageType::File(content) => {
            let filename = content
                .filename
                .as_ref()
                .filter(|name| !name.is_empty())
                .or(Some(&content.body))
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| {
                    filename_for_mime(
                        content
                            .info
                            .as_ref()
                            .and_then(|info| info.mimetype.as_deref()),
                        None,
                    )
                });
            let handle = spawn_tokio!(async move { media.get_file(&content, true).await });
            let data = handle.await.unwrap()?.unwrap();
            Ok((filename, data))
        }
        MessageType::Image(content) => {
            let filename = if content.body.is_empty() {
                filename_for_mime(
                    content
                        .info
                        .as_ref()
                        .and_then(|info| info.mimetype.as_deref()),
                    Some(mime::IMAGE),
                )
            } else {
                content.body.clone()
            };
            let handle = spawn_tokio!(async move { media.get_file(&content, true).await });
            let data = handle.await.unwrap()?.unwrap();
            Ok((filename, data))
        }
        MessageType::Video(content) => {
            let filename = if content.body.is_empty() {
                filename_for_mime(
                    content
                        .info
                        .as_ref()
                        .and_then(|info| info.mimetype.as_deref()),
                    Some(mime::VIDEO),
                )
            } else {
                content.body.clone()
            };
            let handle = spawn_tokio!(async move { media.get_file(&content, true).await });
            let data = handle.await.unwrap()?.unwrap();
            Ok((filename, data))
        }
        MessageType::Audio(content) => {
            let filename = if content.body.is_empty() {
                filename_for_mime(
                    content
                        .info
                        .as_ref()
                        .and_then(|info| info.mimetype.as_deref()),
                    Some(mime::AUDIO),
                )
            } else {
                content.body.clone()
            };
            let handle = spawn_tokio!(async move { media.get_file(&content, true).await });
            let data = handle.await.unwrap()?.unwrap();
            Ok((filename, data))
        }
        _ => {
            panic!("Trying to get the media content of a message of incompatible type");
        }
    }
}

/// Extract mentions from the given string.
///
/// Returns a new string with placeholders and the corresponding widgets and the
/// string they are replacing.
pub fn extract_mentions(s: &str, room: &Room) -> (String, Vec<(Pill, String)>) {
    let session = room.session().unwrap();
    let mut mentions = Vec::new();
    let mut mention = None;
    let mut new_string = String::new();

    for token in Tokenizer::new(s).infallible() {
        match token {
            Token::StartTag(tag) => {
                if tag.name == HtmlString(b"a".to_vec()) && !tag.self_closing {
                    if let Some(pill) = tag
                        .attributes
                        .get(&HtmlString(b"href".to_vec()))
                        .map(|href| String::from_utf8_lossy(href))
                        .and_then(|s| parse_pill(&s, room, &session))
                    {
                        mention = Some((pill, String::new()));
                        new_string.push_str(DEFAULT_PLACEHOLDER);
                        continue;
                    }
                }

                mention = None;

                // Restore HTML.
                write!(new_string, "<{}", String::from_utf8_lossy(&tag.name)).unwrap();
                for (attr_name, attr_value) in &tag.attributes {
                    write!(
                        new_string,
                        r#" {}="{}""#,
                        String::from_utf8_lossy(attr_name),
                        html_escape(&String::from_utf8_lossy(attr_value)),
                    )
                    .unwrap();
                }
                if tag.self_closing {
                    write!(new_string, " /").unwrap();
                }
                write!(new_string, ">").unwrap();
            }
            Token::String(s) => {
                if let Some((_, string)) = &mut mention {
                    write!(string, "{}", String::from_utf8_lossy(&s)).unwrap();
                    continue;
                }

                write!(new_string, "{}", html_escape(&String::from_utf8_lossy(&s))).unwrap();
            }
            Token::EndTag(tag) => {
                if let Some(mention) = mention.take() {
                    mentions.push(mention);
                    continue;
                }

                write!(new_string, "</{}>", String::from_utf8_lossy(&tag.name)).unwrap();
            }
            _ => {}
        }
    }

    (new_string, mentions)
}

/// Try to parse the given string to a Matrix URI and generate a pill for it.
fn parse_pill(s: &str, room: &Room, session: &Session) -> Option<Pill> {
    let uri = html_escape::decode_html_entities(s);

    let Ok(id) = MatrixIdUri::parse(&uri) else {
        return None;
    };

    match id {
        MatrixIdUri::Room(room_uri) => session
            .room_list()
            .get_by_identifier(&room_uri.id)
            .as_ref()
            .map(Pill::new)
            .or_else(|| Some(Pill::new(&RemoteRoom::new(session, room_uri)))),
        MatrixIdUri::User(user_id) => {
            // We should have a strong reference to the list wherever we show a user pill,
            // so we can use `get_or_create_members()`.
            let user = room.get_or_create_members().get_or_create(user_id);
            Some(Pill::new(&user))
        }
        _ => None,
    }
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

/// A unique identifier for a room.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MatrixRoomId {
    /// A room ID.
    Id(OwnedRoomId),
    /// A room alias.
    Alias(OwnedRoomAliasId),
}

impl MatrixRoomId {
    /// The room ID, if this is an ID.
    pub fn as_id(&self) -> Option<&OwnedRoomId> {
        match self {
            Self::Id(room_id) => Some(room_id),
            Self::Alias(_) => None,
        }
    }

    /// The room alias, if this is an alias.
    pub fn as_alias(&self) -> Option<&OwnedRoomAliasId> {
        match self {
            Self::Id(_) => None,
            Self::Alias(alias) => Some(alias),
        }
    }
}

impl From<OwnedRoomId> for MatrixRoomId {
    fn from(value: OwnedRoomId) -> Self {
        Self::Id(value)
    }
}

impl From<OwnedRoomAliasId> for MatrixRoomId {
    fn from(value: OwnedRoomAliasId) -> Self {
        Self::Alias(value)
    }
}

impl From<OwnedRoomOrAliasId> for MatrixRoomId {
    fn from(value: OwnedRoomOrAliasId) -> Self {
        if value.is_room_id() {
            Self::Id(
                value
                    .try_into()
                    .expect("Conversion into known variant should not fail"),
            )
        } else {
            Self::Alias(
                value
                    .try_into()
                    .expect("Conversion into known variant should not fail"),
            )
        }
    }
}

impl From<MatrixRoomId> for OwnedRoomOrAliasId {
    fn from(value: MatrixRoomId) -> Self {
        match value {
            MatrixRoomId::Id(id) => id.into(),
            MatrixRoomId::Alias(alias) => alias.into(),
        }
    }
}

impl fmt::Display for MatrixRoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => id.fmt(f),
            Self::Alias(alias) => alias.fmt(f),
        }
    }
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
                    id: room_id.into(),
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
}

impl TryFrom<&MatrixUri> for MatrixIdUri {
    type Error = MatrixIdUriParseError;

    fn try_from(uri: &MatrixUri) -> Result<Self, Self::Error> {
        // We ignore the action, because we always offer to join a room or DM a user.
        Self::try_from_parts(uri.id().clone(), uri.via())
            .map_err(|_| MatrixIdUriParseError::UnsupportedId(uri.id().clone()))
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
            .map_err(|_| MatrixIdUriParseError::UnsupportedId(uri.id().clone()))
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

/// A URI for a Matrix room ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRoomIdUri {
    /// The room ID.
    pub id: MatrixRoomId,
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
            .or_else(|| {
                RoomOrAliasId::parse(s)
                    .ok()
                    .map(MatrixRoomId::from)
                    .map(Into::into)
            })
    }
}

impl From<MatrixRoomId> for MatrixRoomIdUri {
    fn from(id: MatrixRoomId) -> Self {
        Self {
            id,
            via: Vec::new(),
        }
    }
}

impl From<&MatrixRoomIdUri> for MatrixUri {
    fn from(value: &MatrixRoomIdUri) -> Self {
        match &value.id {
            MatrixRoomId::Id(room_id) => room_id.matrix_uri_via(value.via.clone(), false),
            MatrixRoomId::Alias(alias) => alias.matrix_uri(false),
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

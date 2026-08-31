//! Collection of methods related to the Matrix specification.

use std::{borrow::Cow, fmt, str::FromStr};

use gettextrs::gettext;
use gtk::{glib, prelude::*};
use ruma::{
    IdParseError, MatrixToUri, MatrixUri, MatrixUriError, MilliSecondsSinceUnixEpoch, OwnedEventId,
    OwnedRoomAliasId, OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName, OwnedUserId, RoomId,
    RoomOrAliasId,
    html::{
        Children, Html, NodeRef, StrTendril,
        matrix::{AnchorUri, MatrixElement},
    },
    matrix_uri::MatrixId,
};
use thiserror::Error;

pub(crate) mod ext_traits;
mod media_message;
mod mutual_rooms;
mod url_preview;

/// What is left of this module, and why.
///
/// Everything defined below is here because it cannot be anywhere else: it
/// returns a `Pill` widget, it produces a `glib::DateTime`, or it is a
/// `gettext` call. The Matrix-specification half — the password rules,
/// `@room` detection, raw event comparison, the client builder — is
/// `commune_core::matrix` now, re-exported here under the paths it already
/// had so that no consumer changed. See `doc/track3-convergence.md`.
///
/// `ClientSetupError` is one of them and is the one that keeps an
/// implementation on this side: its variants were transcribed from here
/// unchanged, so all that had to stay behind is the sentences, which
/// `gettext` reaches and the core cannot.
pub(crate) use commune_core::matrix::{
    AT_ROOM, AnySyncOrStrippedTimelineEvent, ClientSetupError, MessageCacheKey,
    client_with_stored_session, find_at_room, original_message_event_from_raw, raw_eq,
    validate_password,
};

pub(crate) use self::{media_message::*, mutual_rooms::fetch_mutual_rooms, url_preview::*};
use crate::{
    components::{AvatarImageSafetySetting, Pill},
    prelude::*,
    session::Room,
};

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

/// Find mentions in the given HTML string.
///
/// Returns a list of `(pill, mention_content)` tuples.
pub(crate) fn find_html_mentions(html: &str, room: &Room) -> Vec<(Pill, StrTendril)> {
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

/// A URI for a Matrix ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MatrixIdUri {
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
    pub(crate) fn parse(s: &str) -> Result<Self, MatrixIdUriParseError> {
        if let Ok(uri) = MatrixToUri::parse(s) {
            return uri.try_into();
        }

        MatrixUri::parse(s)?.try_into()
    }

    /// Try to construct a [`Pill`] from this ID in the given room.
    pub(crate) fn into_pill(self, room: &Room) -> Option<Pill> {
        match self {
            Self::Room(room_uri) => {
                let session = room.session()?;

                let pill =
                    if let Some(uri_room) = session.room_list().get_by_identifier(&room_uri.id) {
                        // We do not need to watch safety settings for local rooms, they will be
                        // watched automatically.
                        Pill::new(&uri_room, AvatarImageSafetySetting::None, None)
                    } else {
                        Pill::new(
                            &session.remote_cache().room(room_uri),
                            AvatarImageSafetySetting::MediaPreviews,
                            Some(room.clone()),
                        )
                    };

                Some(pill)
            }
            Self::User(user_id) => {
                // We should have a strong reference to the list wherever we show a user pill,
                // so we can use `get_or_create_members()`.
                let user = room.get_or_create_members().get_or_create(user_id);

                // We do not need to watch safety settings for users.
                Some(Pill::new(&user, AvatarImageSafetySetting::None, None))
            }
            Self::Event(_) => None,
        }
    }

    /// Get this ID as a `matrix:` URI.
    pub(crate) fn as_matrix_uri(&self) -> MatrixUri {
        match self {
            MatrixIdUri::Room(room_uri) => match <&RoomId>::try_from(&*room_uri.id) {
                Ok(room_id) => room_id.matrix_uri_via(room_uri.via.clone(), false),
                Err(room_alias) => room_alias.matrix_uri(false),
            },
            MatrixIdUri::User(user_id) => user_id.matrix_uri(false),
            MatrixIdUri::Event(event_uri) => {
                let room_id = <&RoomId>::try_from(&*event_uri.room_uri.id)
                    .expect("room alias should not be used to construct event URI");

                room_id.matrix_event_uri_via(
                    event_uri.event_id.clone(),
                    event_uri.room_uri.via.clone(),
                )
            }
        }
    }
}

impl fmt::Display for MatrixIdUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_matrix_uri().fmt(f)
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

impl ToVariant for MatrixIdUri {
    fn to_variant(&self) -> glib::Variant {
        self.to_string().to_variant()
    }
}

impl FromVariant for MatrixIdUri {
    fn from_variant(variant: &glib::Variant) -> Option<Self> {
        Self::parse(&variant.get::<String>()?).ok()
    }
}

/// A URI for a Matrix room ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatrixRoomIdUri {
    /// The room ID.
    pub(crate) id: OwnedRoomOrAliasId,
    /// Matrix servers usable to route a `RoomId`.
    pub(crate) via: Vec<OwnedServerName>,
}

impl MatrixRoomIdUri {
    /// Try parsing a `&str` into a `MatrixRoomIdUri`.
    pub(crate) fn parse(s: &str) -> Option<MatrixRoomIdUri> {
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
pub(crate) struct MatrixEventIdUri {
    /// The event ID.
    pub event_id: OwnedEventId,
    /// The event's room ID URI.
    pub room_uri: MatrixRoomIdUri,
}

/// Errors encountered when parsing a Matrix ID URI.
#[derive(Debug, Clone, Error)]
pub(crate) enum MatrixIdUriParseError {
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

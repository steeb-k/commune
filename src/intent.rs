use std::borrow::Cow;

use gtk::{glib, prelude::*};
use ruma::{OwnedRoomId, OwnedUserId, RoomId};

use crate::utils::matrix::{MatrixIdUri, MatrixRoomIdUri};

/// An intent when opening or activating the application.
///
/// This can be received either via D-Bus or via the command line.
///
/// It cannot be cloned intentionnally, so it is handled only once.
#[derive(Debug)]
pub enum AppIntent {
    /// An intent for a given session.
    WithSession(SessionIntent),
    /// Show the target of a Matrix ID URI.
    ShowMatrixId(MatrixIdUri),
}

impl From<SessionIntent> for AppIntent {
    fn from(value: SessionIntent) -> Self {
        Self::WithSession(value)
    }
}

impl From<MatrixIdUri> for AppIntent {
    fn from(value: MatrixIdUri) -> Self {
        Self::ShowMatrixId(value)
    }
}

/// An intent for a given session.
///
/// It cannot be cloned intentionnally, so it is handled only once.
#[derive(Debug)]
pub enum SessionIntent {
    /// Show an existing room.
    ShowRoom(ShowRoomPayload),
    /// Join a room.
    JoinRoom(JoinRoomPayload),
    /// Show a user.
    ShowUser(ShowUserPayload),
}

impl SessionIntent {
    /// Constructs an `AppIntent` with the given Matrix ID URI and session ID.
    pub fn with_matrix_uri(session_id: String, matrix_uri: MatrixIdUri) -> Self {
        match matrix_uri {
            MatrixIdUri::Room(room_uri) => Self::JoinRoom(JoinRoomPayload {
                session_id,
                room_uri,
            }),
            MatrixIdUri::User(user_id) => Self::ShowUser(ShowUserPayload {
                session_id,
                user_id,
            }),
            // FIXME: We don't support showing specific events in the room history.
            MatrixIdUri::Event(event_uri) => Self::JoinRoom(JoinRoomPayload {
                session_id,
                room_uri: event_uri.room_uri,
            }),
        }
    }

    /// The ID of the session that should process this intent.
    pub fn session_id(&self) -> &str {
        match self {
            Self::ShowRoom(p) => &p.session_id,
            Self::JoinRoom(p) => &p.session_id,
            Self::ShowUser(p) => &p.session_id,
        }
    }
}

/// The payload to show a room.
#[derive(Debug)]
pub struct ShowRoomPayload {
    pub session_id: String,
    pub room_id: OwnedRoomId,
}

impl StaticVariantType for ShowRoomPayload {
    fn static_variant_type() -> Cow<'static, glib::VariantTy> {
        <(String, String)>::static_variant_type()
    }
}

impl ToVariant for ShowRoomPayload {
    fn to_variant(&self) -> glib::Variant {
        (&self.session_id, self.room_id.as_str()).to_variant()
    }
}

impl FromVariant for ShowRoomPayload {
    fn from_variant(variant: &glib::Variant) -> Option<Self> {
        let (session_id, room_id) = variant.get::<(String, String)>()?;
        let room_id = RoomId::parse(room_id).ok()?;
        Some(Self {
            session_id,
            room_id,
        })
    }
}

/// The payload to join a room.
#[derive(Debug)]
pub struct JoinRoomPayload {
    pub session_id: String,
    pub room_uri: MatrixRoomIdUri,
}

/// The payload to show a user.
#[derive(Debug)]
pub struct ShowUserPayload {
    pub session_id: String,
    pub user_id: OwnedUserId,
}

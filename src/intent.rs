use std::borrow::Cow;

use gtk::glib;
use ruma::{OwnedRoomId, RoomId};

/// An intent when opening or activating the application.
///
/// This can be received either via D-Bus or via the command line.
///
/// It cannot be cloned intentionnally, so it is handled only once.
#[derive(Debug)]
pub enum AppIntent {
    /// Show a room.
    ShowRoom(ShowRoomPayload),
}

impl AppIntent {
    /// The ID of the session that should process this intent.
    pub fn session_id(&self) -> &str {
        match self {
            AppIntent::ShowRoom(p) => &p.session_id,
        }
    }
}

/// The payload to show a room.
#[derive(Debug)]
pub struct ShowRoomPayload {
    pub session_id: String,
    pub room_id: OwnedRoomId,
}

impl glib::StaticVariantType for ShowRoomPayload {
    fn static_variant_type() -> Cow<'static, glib::VariantTy> {
        <(String, String)>::static_variant_type()
    }
}

impl glib::ToVariant for ShowRoomPayload {
    fn to_variant(&self) -> glib::Variant {
        (&self.session_id, self.room_id.as_str()).to_variant()
    }
}

impl glib::FromVariant for ShowRoomPayload {
    fn from_variant(variant: &glib::Variant) -> Option<Self> {
        let (session_id, room_id) = variant.get::<(String, String)>()?;
        let room_id = RoomId::parse(room_id).ok()?;
        Some(Self {
            session_id,
            room_id,
        })
    }
}

use ruma::events::macros::EventContent;
use serde::{Deserialize, Serialize};

/// The content of an `org.gnome.fractal.language` event.
///
/// The language used in a room.
///
/// It is used to change the spell checker's language per room.
#[derive(Clone, Debug, Deserialize, Serialize, EventContent)]
#[ruma_event(type = "org.gnome.fractal.language", kind = RoomAccountData)]
pub struct LanguageEventContent {
    /// The language to spell check.
    pub input_language: String,
}

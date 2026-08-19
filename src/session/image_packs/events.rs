//! Event content types for [image packs].
//!
//! Matrix 1.19 defines two image pack events, `m.room.image_pack` and
//! `m.image_pack.rooms`. Clients in the wild still use the `im.ponies.*` names
//! from MSC2545. The content is identical under both names, so it is defined
//! once, in [`PackContent`] and [`EnabledPacks`], and each event type is a
//! thin wrapper around it.
//!
//! There is one type per name rather than one type with an `alias`, because an
//! alias only affects deserialization: both the state store and the event
//! handlers of the SDK key events on the single type string of a content type.
//! We send the unstable names and read both.
//!
//! MSC2545 also defined a personal image pack in the global account data,
//! `im.ponies.user_emotes`. It was not carried into the stable specification,
//! which expects a personal pack to be a room pack enabled globally instead,
//! and it is not supported here either.
//!
//! ruma has its own types for the two stable events, but they do not cover the
//! unstable names, and they drop the unknown
//! properties that the specification requires clients to preserve. Only
//! [`ImageInfo`] is reused, because `m.sticker` is defined in terms of it.
//!
//! [image packs]: https://spec.matrix.org/v1.19/client-server-api/#image-packs

// The `EventContent` derive checks a cfg that only ruma sets for itself.
#![allow(unexpected_cfgs)]

use std::collections::{BTreeMap, BTreeSet};

use ruma::{
    OwnedMxcUri, OwnedRoomId,
    events::{macros::EventContent, room::ImageInfo},
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// The maximum length of a shortcode, in bytes.
pub(crate) const SHORTCODE_MAX_LEN: usize = 100;

/// Whether the given string is a valid shortcode.
///
/// The grammar is `1*100shortcode_char`, where a `shortcode_char` is an ASCII
/// alphanumeric character, `-` or `_`.
///
/// Shortcodes that do not match are still rendered, so that users can identify
/// and correct them. This is only used when creating or editing a pack.
pub(crate) fn is_valid_shortcode(shortcode: &str) -> bool {
    !shortcode.is_empty()
        && shortcode.len() <= SHORTCODE_MAX_LEN
        && shortcode
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// An image in an image pack.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackImage {
    /// The `mxc://` URI of the image.
    pub url: OwnedMxcUri,

    /// A textual representation or description of the image.
    ///
    /// Used as the `alt` attribute when the image is sent as an emote, and as
    /// the `body` when it is sent as a sticker. Defaults to the shortcode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,

    /// Metadata about the image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<Box<ImageInfo>>,

    /// The properties that we do not know about.
    ///
    /// Kept so that editing a pack that another client created does not drop
    /// what it put there.
    #[serde(flatten)]
    unknown: BTreeMap<String, JsonValue>,
}

impl PackImage {
    /// Create a new `PackImage` with the given `mxc://` URI.
    pub(crate) fn new(url: OwnedMxcUri) -> Self {
        Self {
            url,
            body: None,
            info: None,
            unknown: BTreeMap::new(),
        }
    }
}

/// The intended usage of an image pack.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackUsage {
    /// The images are meant to be sent inline in messages.
    Emoticon,

    /// The images are meant to be sent as sticker events.
    Sticker,

    /// A usage that we do not know about.
    #[serde(untagged)]
    Unknown(String),
}

/// Metadata about an image pack.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PackMeta {
    /// A display name for the pack.
    ///
    /// If this is not set and the pack is defined in a room, the name of the
    /// room is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// The `mxc://` URI of an avatar for the pack.
    ///
    /// If this is not set and the pack is defined in a room, the avatar of the
    /// room is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<OwnedMxcUri>,

    /// The intended usages of the pack.
    ///
    /// If this is empty, all the usages are assumed.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub usage: BTreeSet<PackUsage>,

    /// Who to credit for the pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,

    /// The properties that we do not know about.
    ///
    /// Kept so that editing a pack that another client created does not drop
    /// what it put there.
    #[serde(flatten)]
    unknown: BTreeMap<String, JsonValue>,
}

impl PackMeta {
    /// Whether the pack can be used for the given usage.
    ///
    /// A pack that declares no usage can be used for all of them.
    pub(crate) fn has_usage(&self, usage: &PackUsage) -> bool {
        self.usage.is_empty() || self.usage.contains(usage)
    }

    /// Whether none of the fields are set.
    fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.avatar_url.is_none()
            && self.usage.is_empty()
            && self.attribution.is_none()
            && self.unknown.is_empty()
    }
}

/// The images of an image pack, and the metadata about it.
///
/// This is the content of both the personal and the room image pack events,
/// which only differ by where they are stored.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PackContent {
    /// A map from a shortcode to an image.
    ///
    /// Required by the specification, but defaulted so that a redacted pack
    /// deserializes to an empty one instead of failing.
    #[serde(default)]
    pub images: BTreeMap<String, PackImage>,

    /// Metadata about the pack as a whole.
    #[serde(default, skip_serializing_if = "PackMeta::is_empty")]
    pub pack: PackMeta,
}

/// The content of an `im.ponies.room_emotes` event.
///
/// An image pack defined in the state of a room. The state key is the
/// identifier of the pack within that room.
#[derive(Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[ruma_event(type = "im.ponies.room_emotes", kind = State, state_key_type = String)]
pub struct RoomEmotesEventContent {
    /// The pack.
    #[serde(default, flatten)]
    pub pack: PackContent,
}

/// The content of an `m.room.image_pack` event.
///
/// The stable counterpart of [`RoomEmotesEventContent`], which we read but do
/// not send.
#[derive(Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[ruma_event(type = "m.room.image_pack", kind = State, state_key_type = String)]
pub struct RoomImagePackEventContent {
    /// The pack.
    #[serde(default, flatten)]
    pub pack: PackContent,
}

/// The content of an `org.gnome.Fractal.image_packs_room` event.
///
/// The room that Fractal creates image packs in. No specification defines
/// this: it is configuration for this client, which is what account data is
/// for. Another client is unaffected by it, and removing it only means that
/// the next pack goes to a new room.
#[derive(Clone, Debug, Deserialize, Serialize, EventContent)]
#[ruma_event(type = "org.gnome.Fractal.image_packs_room", kind = GlobalAccountData)]
pub struct ImagePacksRoomEventContent {
    /// The room that image packs are created in.
    pub room_id: OwnedRoomId,
}

/// Metadata about a room image pack that is enabled globally.
///
/// The specification reserves this object for future use and requires clients
/// to preserve the properties that they do not know about, so it keeps
/// everything that it was deserialized from.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EnabledPackMeta {
    #[serde(flatten)]
    unknown: BTreeMap<String, JsonValue>,
}

/// The room image packs that are enabled globally.
///
/// Maps a room ID to the state keys of the packs that are enabled in it.
pub type EnabledPacks = BTreeMap<OwnedRoomId, BTreeMap<String, EnabledPackMeta>>;

/// The content of an `im.ponies.emote_rooms` event.
///
/// Lists the room image packs that the user enabled globally.
#[derive(Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[ruma_event(type = "im.ponies.emote_rooms", kind = GlobalAccountData)]
pub struct EmoteRoomsEventContent {
    /// The enabled packs, per room.
    pub rooms: EnabledPacks,
}

/// The content of an `m.image_pack.rooms` event.
///
/// The stable counterpart of [`EmoteRoomsEventContent`], which we read but do
/// not send.
#[derive(Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[ruma_event(type = "m.image_pack.rooms", kind = GlobalAccountData)]
pub struct ImagePackRoomsEventContent {
    /// The enabled packs, per room.
    pub rooms: EnabledPacks,
}

#[cfg(test)]
mod tests {
    use ruma::{
        events::{GlobalAccountDataEventContent as _, StateEventContent as _},
        room_id,
        serde::Raw,
    };
    use serde_json::{from_value, json, to_value};

    use super::*;

    #[test]
    fn shortcode_grammar() {
        assert!(is_valid_shortcode("cat_wave"));
        assert!(is_valid_shortcode("Cat-Wave-2"));
        assert!(is_valid_shortcode(&"a".repeat(SHORTCODE_MAX_LEN)));

        assert!(!is_valid_shortcode(""));
        assert!(!is_valid_shortcode(&"a".repeat(SHORTCODE_MAX_LEN + 1)));
        // The delimiters of the completion syntax, and spaces.
        assert!(!is_valid_shortcode("cat:wave"));
        assert!(!is_valid_shortcode("cat/wave"));
        assert!(!is_valid_shortcode("cat wave"));
        // Not ASCII.
        assert!(!is_valid_shortcode("café"));
    }

    #[test]
    fn pack_deserializes() {
        let content = json!({
            "images": {
                "cat_wave": {
                    "url": "mxc://example.org/abc123",
                    "body": "a waving cat",
                },
            },
            "pack": {
                "display_name": "Cats",
                "usage": ["sticker"],
            },
        });

        let content = from_value::<RoomEmotesEventContent>(content).unwrap();

        let image = content.pack.images.get("cat_wave").unwrap();
        assert_eq!(image.url, "mxc://example.org/abc123");
        assert_eq!(image.body.as_deref(), Some("a waving cat"));

        let pack = &content.pack.pack;
        assert_eq!(pack.display_name.as_deref(), Some("Cats"));
        assert!(pack.has_usage(&PackUsage::Sticker));
        assert!(!pack.has_usage(&PackUsage::Emoticon));
    }

    /// A pack that declares no usage can be used for everything.
    #[test]
    fn pack_without_usage_has_every_usage() {
        let pack = PackMeta::default();
        assert!(pack.has_usage(&PackUsage::Sticker));
        assert!(pack.has_usage(&PackUsage::Emoticon));
    }

    /// A usage that we do not know about is preserved.
    #[test]
    fn unknown_pack_usage_is_kept() {
        let usage = json!(["sticker", "org.example.custom"]);
        let usage = from_value::<BTreeSet<PackUsage>>(usage).unwrap();

        assert!(usage.contains(&PackUsage::Sticker));
        assert!(usage.contains(&PackUsage::Unknown("org.example.custom".to_owned())));

        assert_eq!(
            to_value(usage).unwrap(),
            json!(["sticker", "org.example.custom"])
        );
    }

    /// Empty pack metadata is not serialized, and a missing one deserializes.
    #[test]
    fn empty_pack_meta_is_omitted() {
        let mut images = BTreeMap::new();
        images.insert(
            "cat".to_owned(),
            PackImage::new("mxc://example.org/abc123".into()),
        );

        let content = RoomEmotesEventContent {
            pack: PackContent {
                images,
                pack: PackMeta::default(),
            },
        };

        let serialized = to_value(content).unwrap();
        assert_eq!(
            serialized,
            json!({ "images": { "cat": { "url": "mxc://example.org/abc123" } } })
        );

        let deserialized = from_value::<RoomEmotesEventContent>(serialized).unwrap();
        assert!(deserialized.pack.pack.is_empty());
    }

    /// The unknown properties of an enabled pack survive a deserialization
    /// followed by a serialization, as the specification requires.
    #[test]
    fn enabled_pack_meta_keeps_unknown_properties() {
        let content = json!({
            "rooms": {
                "!a:example.org": {
                    "": {},
                    "stickers": { "future_property": 42 },
                },
            },
        });

        let deserialized = from_value::<EmoteRoomsEventContent>(content.clone()).unwrap();

        let packs = deserialized.rooms.get(room_id!("!a:example.org")).unwrap();
        assert!(packs.contains_key(""));
        assert!(packs.contains_key("stickers"));

        assert_eq!(to_value(deserialized).unwrap(), content);
    }

    /// The properties of a pack that we do not know about survive an edit.
    #[test]
    fn pack_keeps_unknown_properties() {
        let content = json!({
            "images": {
                "cat": {
                    "url": "mxc://example.org/abc",
                    "org.example.tag": "animals",
                },
            },
            "pack": {
                "display_name": "Cats",
                "org.example.license": "CC0",
            },
        });

        let mut deserialized = from_value::<RoomEmotesEventContent>(content.clone()).unwrap();

        // Rename the shortcode, which is what an edit does.
        let image = deserialized.pack.images.remove("cat").unwrap();
        deserialized.pack.images.insert("kitten".to_owned(), image);

        let expected = json!({
            "images": {
                "kitten": {
                    "url": "mxc://example.org/abc",
                    "org.example.tag": "animals",
                },
            },
            "pack": {
                "display_name": "Cats",
                "org.example.license": "CC0",
            },
        });
        assert_eq!(to_value(deserialized).unwrap(), expected);
    }

    /// The event types that we send are the unstable ones.
    #[test]
    fn sent_event_types_are_unstable() {
        assert_eq!(
            EmoteRoomsEventContent::default().event_type().to_string(),
            "im.ponies.emote_rooms"
        );
        assert_eq!(
            RoomEmotesEventContent::default().event_type().to_string(),
            "im.ponies.room_emotes"
        );
    }

    /// Both names of a room image pack carry the same content.
    #[test]
    fn both_room_pack_names_deserialize() {
        let content = json!({ "images": { "cat": { "url": "mxc://example.org/abc" } } });

        let unstable = from_value::<RoomEmotesEventContent>(content.clone()).unwrap();
        let stable = from_value::<RoomImagePackEventContent>(content).unwrap();

        assert_eq!(unstable.pack.images.len(), 1);
        assert_eq!(stable.pack.images.len(), 1);
    }

    /// A whole event of each room image pack type deserializes.
    #[test]
    fn room_pack_events_deserialize() {
        let event = |event_type| {
            json!({
                "type": event_type,
                "state_key": "stickers",
                "sender": "@alice:example.org",
                "event_id": "$a",
                "origin_server_ts": 1,
                "room_id": "!a:example.org",
                "content": { "images": { "cat": { "url": "mxc://example.org/abc" } } },
            })
        };

        let unstable = from_value::<Raw<RoomEmotesEvent>>(event("im.ponies.room_emotes"))
            .unwrap()
            .deserialize()
            .unwrap();
        assert_eq!(unstable.as_original().unwrap().content.pack.images.len(), 1);

        let stable = from_value::<Raw<RoomImagePackEvent>>(event("m.room.image_pack"))
            .unwrap()
            .deserialize()
            .unwrap();
        assert_eq!(stable.as_original().unwrap().content.pack.images.len(), 1);
    }

    /// The stable event types are the ones we only read.
    #[test]
    fn read_event_types_are_stable() {
        assert_eq!(
            ImagePackRoomsEventContent::default()
                .event_type()
                .to_string(),
            "m.image_pack.rooms"
        );
        assert_eq!(
            RoomImagePackEventContent::default()
                .event_type()
                .to_string(),
            "m.room.image_pack"
        );
    }
}

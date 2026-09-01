//! Collection of methods related to the Matrix specification.

use std::borrow::Cow;

use gettextrs::gettext;
use gtk::{glib, prelude::*};
use ruma::{
    MilliSecondsSinceUnixEpoch,
    html::{Children, Html, NodeRef, StrTendril, matrix::MatrixElement},
};

pub(crate) mod ext_traits;
mod media_message;

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
    AT_ROOM, AnySyncOrStrippedTimelineEvent, ClientSetupError, MatrixEventIdUri, MatrixIdUri,
    MatrixRoomIdUri, MessageCacheKey, client_with_stored_session, fetch_mutual_rooms, find_at_room,
    original_message_event_from_raw, previewable_url, raw_eq, validate_password,
};

pub(crate) use self::media_message::*;
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

/// What the application can do with a [`MatrixIdUri`] that the core cannot.
///
/// The type is the core's, so these cannot be inherent methods, and two of
/// the three cannot be trait implementations either: `ToVariant` and friends
/// are `glib`'s traits and `MatrixIdUri` is now a foreign type, which the
/// orphan rule forbids. The variant form was only ever the string form —
/// the implementations this replaces were `self.to_string().to_variant()` and
/// `Self::parse(&variant.get::<String>()?)` — so nothing is lost by naming
/// the conversion instead of deriving it, and the `GAction` parameter type it
/// travels as is now visible at the call site rather than hidden behind a
/// trait.
pub(crate) trait MatrixIdUriExt {
    /// The `GVariant` type a `MatrixIdUri` travels as.
    fn variant_type() -> Cow<'static, glib::VariantTy>;

    /// This URI as a `GVariant`, for a `GAction` parameter.
    ///
    /// Named `as_variant` rather than `to_variant` so that it cannot be
    /// confused with `glib`'s trait method of that name.
    fn as_variant(&self) -> glib::Variant;

    /// Read a URI back out of a `GAction` parameter.
    fn from_variant(variant: &glib::Variant) -> Option<MatrixIdUri>;

    /// Try to construct a [`Pill`] from this ID in the given room.
    fn into_pill(self, room: &Room) -> Option<Pill>;
}

impl MatrixIdUriExt for MatrixIdUri {
    fn variant_type() -> Cow<'static, glib::VariantTy> {
        String::static_variant_type()
    }

    fn as_variant(&self) -> glib::Variant {
        self.to_string().to_variant()
    }

    fn from_variant(variant: &glib::Variant) -> Option<MatrixIdUri> {
        MatrixIdUri::parse(&variant.get::<String>()?).ok()
    }

    fn into_pill(self, room: &Room) -> Option<Pill> {
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

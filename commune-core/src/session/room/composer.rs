//! The message composer, headless.
//!
//! The application's `ComposerParser` walks a `GtkTextBuffer` and turns what
//! it finds — text, mention pills, emoticon pills — into the content of a
//! message event. The walk is the widget's; what the chunks become on the
//! wire is not, and that is what lives here. An embedder hands over the
//! chunks in order and gets the application's exact content back: the
//! plain body with the names and shortcodes, the formatted body with the
//! links and image tags, the emote command stripped, and `m.mentions`
//! always present, empty or not.

use std::fmt::Write;

use ruma::{
    OwnedUserId, UserId,
    events::{
        Mentions,
        room::message::{
            EmoteMessageEventContent, FormattedBody, MessageType,
            RoomMessageEventContentWithoutRelation,
        },
    },
};

use crate::matrix::AT_ROOM;

/// The height of an emoticon in the formatted body of a message, in pixels.
const EMOTICON_HTML_HEIGHT: u32 = 32;

/// A chunk of content in a message composer.
#[derive(Debug, Clone)]
pub enum ComposerChunk {
    /// Some text.
    Text(String),
    /// A mention with an HTML representation: a user or a room.
    Mention {
        /// The string representation of the mention.
        name: String,
        /// The `matrix.to` URI of the mention.
        uri: String,
        /// The user ID, if this is a user mention.
        user_id: Option<OwnedUserId>,
    },
    /// An `@room` mention.
    AtRoom,
    /// An image of a pack, to send inline in the message.
    Emoticon {
        /// The shortcode that identifies the image in its pack.
        shortcode: String,
        /// The MXC URI of the image.
        uri: String,
        /// The description of the image.
        body: String,
    },
}

impl ComposerChunk {
    /// A mention of the given user, presented by the given display name
    /// when they have one and by their ID otherwise.
    #[must_use]
    pub fn user_mention(user_id: &UserId, display_name: Option<&str>) -> Self {
        let name = display_name
            .filter(|name| !name.is_empty())
            .map_or_else(|| user_id.to_string(), ToOwned::to_owned);

        Self::Mention {
            name,
            uri: user_id.matrix_to_uri().to_string(),
            user_id: Some(user_id.to_owned()),
        }
    }
}

/// The text an emoticon leaves in the plain body of a message.
#[must_use]
pub fn emoticon_plain(shortcode: &str) -> String {
    format!(":{shortcode}:")
}

/// The HTML an emoticon leaves in the formatted body of a message.
fn emoticon_html(shortcode: &str, uri: &str, body: &str) -> String {
    format!(
        r#"<img data-mx-emoticon src="{src}" alt="{alt}" title="{title}" height="{EMOTICON_HTML_HEIGHT}">"#,
        src = escape_markup(uri),
        alt = escape_markup(body),
        title = escape_markup(shortcode),
    )
}

/// Escape a string for an HTML attribute or text position, as
/// `g_markup_escape_text` does.
fn escape_markup(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\'' => escaped.push_str("&apos;"),
            '"' => escaped.push_str("&quot;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// Parse the given chunks of the message composer into the content of a
/// message event.
///
/// Returns `None` if the message is empty, which is not sent.
#[must_use]
pub fn compose_message(
    chunks: &[ComposerChunk],
    markdown_enabled: bool,
) -> Option<RoomMessageEventContentWithoutRelation> {
    let mut has_rich_mentions = false;
    let mut has_emoticons = false;
    let mut plain_body = String::new();
    // This is Markdown if markdown is enabled, otherwise it is HTML.
    let mut formatted_body = String::new();
    let mut mentions = Mentions::new();

    for chunk in chunks {
        match chunk {
            ComposerChunk::Text(text) => {
                plain_body.push_str(text);
                formatted_body.push_str(text);
            }
            ComposerChunk::Mention { name, uri, user_id } => {
                has_rich_mentions = true;
                plain_body.push_str(name);
                if markdown_enabled {
                    let _ = write!(formatted_body, "[{name}]({uri})");
                } else {
                    let _ = write!(formatted_body, "<a href=\"{uri}\">{name}</a>");
                }

                if let Some(user_id) = user_id {
                    mentions.user_ids.insert(user_id.clone());
                }
            }
            ComposerChunk::AtRoom => {
                plain_body.push_str(AT_ROOM);
                formatted_body.push_str(AT_ROOM);

                mentions.room = true;
            }
            ComposerChunk::Emoticon {
                shortcode,
                uri,
                body,
            } => {
                // There is no markdown for an image with attributes, so the
                // HTML is written as-is in both cases.
                has_emoticons = true;
                plain_body.push_str(&emoticon_plain(shortcode));
                formatted_body.push_str(&emoticon_html(shortcode, uri, body));
            }
        }
    }

    // Remove the command of the emote.
    let is_emote = plain_body.starts_with("/me ");
    if is_emote {
        plain_body.replace_range(.."/me ".len(), "");
        formatted_body.replace_range(.."/me ".len(), "");
    }

    if plain_body.trim().is_empty() {
        // Do not send empty message.
        return None;
    }

    let html_body = if markdown_enabled {
        FormattedBody::markdown(formatted_body).map(|b| b.body)
    } else if has_rich_mentions || has_emoticons {
        // Already formatted with HTML.
        Some(formatted_body)
    } else {
        None
    };

    let mut content = if is_emote {
        MessageType::Emote(if let Some(html_body) = html_body {
            EmoteMessageEventContent::html(plain_body, html_body)
        } else {
            EmoteMessageEventContent::plain(plain_body)
        })
        .into()
    } else if let Some(html_body) = html_body {
        RoomMessageEventContentWithoutRelation::text_html(plain_body, html_body)
    } else {
        RoomMessageEventContentWithoutRelation::text_plain(plain_body)
    };

    // To avoid triggering legacy pushrules, we must always include the
    // mentions, even if they are empty.
    content = content.add_mentions(mentions);

    Some(content)
}

#[cfg(test)]
mod tests {
    use ruma::{owned_user_id, user_id};

    use super::*;

    fn text(s: &str) -> ComposerChunk {
        ComposerChunk::Text(s.to_owned())
    }

    fn message_of(content: &RoomMessageEventContentWithoutRelation) -> &MessageType {
        &content.msgtype
    }

    /// Plain text carries empty mentions, as the application always sends
    /// them.
    #[test]
    fn plain_text_always_carries_mentions() {
        let content = compose_message(&[text("Hello")], true).expect("a message");

        let MessageType::Text(text) = message_of(&content) else {
            panic!("a text message");
        };
        assert_eq!(text.body, "Hello");
        assert!(text.formatted.is_none());
        let mentions = content.mentions.expect("mentions");
        assert!(mentions.user_ids.is_empty());
        assert!(!mentions.room);
    }

    /// An empty message, whitespace included, is not sent.
    #[test]
    fn empty_message_is_not_sent() {
        assert!(compose_message(&[], true).is_none());
        assert!(compose_message(&[text("  \n ")], true).is_none());
    }

    /// A user mention is the name in the plain body, a link in the
    /// formatted body, and an entry in the mentions.
    #[test]
    fn user_mention_is_a_link_and_a_mention() {
        let alice = user_id!("@alice:example.org");
        let chunks = [
            text("Hi "),
            ComposerChunk::user_mention(alice, Some("Alice")),
            text("!"),
        ];
        let content = compose_message(&chunks, true).expect("a message");

        let MessageType::Text(text) = message_of(&content) else {
            panic!("a text message");
        };
        assert_eq!(text.body, "Hi Alice!");
        let formatted = text.formatted.as_ref().expect("a formatted body");
        assert_eq!(
            formatted.body,
            format!(r#"Hi <a href="{}">Alice</a>!"#, alice.matrix_to_uri())
        );
        let mentions = content.mentions.expect("mentions");
        assert!(
            mentions
                .user_ids
                .contains(&owned_user_id!("@alice:example.org"))
        );
        assert!(!mentions.room);
    }

    /// Without a display name, a user is mentioned by ID.
    #[test]
    fn user_without_display_name_is_mentioned_by_id() {
        let alice = user_id!("@alice:example.org");
        let ComposerChunk::Mention { name, .. } = ComposerChunk::user_mention(alice, None) else {
            panic!("a mention");
        };
        assert_eq!(name, "@alice:example.org");
    }

    /// `@room` sets the room flag of the mentions.
    #[test]
    fn at_room_mentions_the_room() {
        let content =
            compose_message(&[text("Everyone: "), ComposerChunk::AtRoom], true).expect("a message");

        let MessageType::Text(text) = message_of(&content) else {
            panic!("a text message");
        };
        assert_eq!(text.body, "Everyone: @room");
        assert!(content.mentions.expect("mentions").room);
    }

    /// The emote command is stripped and the message becomes an emote.
    #[test]
    fn emote_command_makes_an_emote() {
        let content = compose_message(&[text("/me waves")], true).expect("a message");

        let MessageType::Emote(emote) = message_of(&content) else {
            panic!("an emote");
        };
        assert_eq!(emote.body, "waves");
    }

    /// An emoticon is its shortcode in the plain body and the exact image
    /// tag in the formatted body, with its attributes escaped.
    #[test]
    fn emoticon_is_shortcode_and_image_tag() {
        let chunks = [
            text("Hello "),
            ComposerChunk::Emoticon {
                shortcode: "cat_wave".to_owned(),
                uri: "mxc://example.org/abc".to_owned(),
                body: "a \"waving\" cat".to_owned(),
            },
        ];
        let content = compose_message(&chunks, true).expect("a message");

        let MessageType::Text(text) = message_of(&content) else {
            panic!("a text message");
        };
        assert_eq!(text.body, "Hello :cat_wave:");
        let formatted = text.formatted.as_ref().expect("a formatted body");
        assert_eq!(
            formatted.body,
            r#"Hello <img data-mx-emoticon src="mxc://example.org/abc" alt="a &quot;waving&quot; cat" title="cat_wave" height="32">"#
        );
    }

    /// With markdown disabled, a mention is still HTML.
    #[test]
    fn mention_without_markdown_is_html() {
        let alice = user_id!("@alice:example.org");
        let content = compose_message(&[ComposerChunk::user_mention(alice, Some("Alice"))], false)
            .expect("a message");

        let MessageType::Text(text) = message_of(&content) else {
            panic!("a text message");
        };
        let formatted = text.formatted.as_ref().expect("a formatted body");
        assert!(formatted.body.starts_with("<a href="));
    }
}

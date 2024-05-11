//! Helper traits and methods for strings.

use std::fmt::{self, Write};

use gtk::glib::markup_escape_text;
use linkify::{LinkFinder, LinkKind};

use super::matrix::{find_at_room, MatrixIdUri, AT_ROOM};
use crate::{
    components::{LabelWithWidgets, Pill},
    prelude::*,
    session::model::Room,
};

/// Common extensions to strings.
pub trait StrExt {
    /// Escape markup for compatibility with Pango.
    fn escape_markup(&self) -> String;

    /// Remove newlines from the string.
    fn remove_newlines(&self) -> String;
}

impl<T> StrExt for T
where
    T: AsRef<str>,
{
    fn escape_markup(&self) -> String {
        markup_escape_text(self.as_ref()).into()
    }

    fn remove_newlines(&self) -> String {
        self.as_ref().replace('\n', "")
    }
}

/// Common extensions to mutable strings.
pub trait StrMutExt {
    /// Truncate this string at the first newline.
    ///
    /// Appends an ellipsis if the string was truncated.
    ///
    /// Returns `true` if the string was truncated.
    fn truncate_newline(&mut self) -> bool;

    /// Truncate whitespaces at the end of the string.
    fn truncate_end_whitespaces(&mut self);

    /// Append an ellipsis, except if this string already ends with an ellipsis.
    fn append_ellipsis(&mut self);
}

impl StrMutExt for String {
    fn truncate_newline(&mut self) -> bool {
        let newline = self.find(|c: char| c == '\n');

        if let Some(newline) = newline {
            self.truncate(newline);
            self.append_ellipsis();
        }

        newline.is_some()
    }

    fn truncate_end_whitespaces(&mut self) {
        if self.is_empty() {
            return;
        }

        let rspaces_idx = self
            .rfind(|c: char| !c.is_whitespace())
            .map(|idx| {
                // We have the position of the last non-whitespace character, so the first
                // whitespace character is the next one.
                let mut idx = idx + 1;

                while !self.is_char_boundary(idx) {
                    idx += 1;
                }

                idx
            })
            // 0 means that there are only whitespaces in the string.
            .unwrap_or_default();

        if rspaces_idx < self.len() {
            self.truncate(rspaces_idx);
        }
    }

    fn append_ellipsis(&mut self) {
        if !self.ends_with('…') && !self.ends_with("..") {
            self.push('…');
        }
    }
}

/// Common extensions for adding Pango markup to mutable strings.
pub trait PangoStrMutExt {
    /// Append the opening Pango markup link tag of the given URI parts.
    ///
    /// The URI is also used as a title, so users can preview the link on hover.
    fn append_link_opening_tag(&mut self, uri: impl AsRef<str>);

    /// Append the given emote's sender name and consumes it, if it is set.
    fn maybe_append_emote_name(&mut self, name: &mut Option<&str>);

    /// Append the given URI as a mention, if it is one.
    ///
    /// Returns the created [`Pill`], it the URI was added as a mention.
    fn maybe_append_mention(&mut self, uri: impl TryInto<MatrixIdUri>, room: &Room)
        -> Option<Pill>;

    /// Append the given string and replace `@room` with a mention.
    ///
    /// Returns the created [`Pill`], it `@room` was found.
    fn append_and_replace_at_room(&mut self, s: &str, room: &Room) -> Option<Pill>;
}

impl PangoStrMutExt for String {
    fn append_link_opening_tag(&mut self, uri: impl AsRef<str>) {
        let uri = uri.escape_markup();
        // We need to escape the title twice because GTK doesn't take care of it.
        let title = uri.escape_markup();

        let _ = write!(self, r#"<a href="{uri}" title="{title}">"#);
    }

    fn maybe_append_emote_name(&mut self, name: &mut Option<&str>) {
        if let Some(name) = name.take() {
            let _ = write!(self, "<b>{}</b> ", name.escape_markup());
        }
    }

    fn maybe_append_mention(
        &mut self,
        uri: impl TryInto<MatrixIdUri>,
        room: &Room,
    ) -> Option<Pill> {
        let pill = uri.try_into().ok().and_then(|uri| uri.into_pill(room))?;

        self.push_str(LabelWithWidgets::DEFAULT_PLACEHOLDER);

        Some(pill)
    }

    fn append_and_replace_at_room(&mut self, s: &str, room: &Room) -> Option<Pill> {
        if let Some(pos) = find_at_room(s) {
            self.push_str(&(&s[..pos]).escape_markup());
            self.push_str(LabelWithWidgets::DEFAULT_PLACEHOLDER);
            self.push_str(&(&s[pos + AT_ROOM.len()..]).escape_markup());

            Some(room.at_room().to_pill())
        } else {
            self.push_str(&s.escape_markup());
            None
        }
    }
}

/// Linkify the given text.
///
/// The text will also be escaped with [`StrExt::escape_markup()`].
pub fn linkify(text: &str) -> String {
    let mut linkified = String::with_capacity(text.len());
    Linkifier::new(&mut linkified).linkify(text);
    linkified
}

/// A helper type to linkify text.
pub struct Linkifier<'a> {
    /// The string containing the result.
    inner: &'a mut String,
    /// The mentions detection setting and results.
    mentions: MentionsMode<'a>,
}

impl<'a> Linkifier<'a> {
    /// Construct a new linkifier that will add text in the given string.
    pub fn new(inner: &'a mut String) -> Self {
        Self {
            inner,
            mentions: MentionsMode::NoMentions,
        }
    }

    /// Enable mentions detection in the given room and add pills to the given
    /// list.
    ///
    /// If `detect_at_room` is `true`, it will also try to detect `@room`
    /// mentions.
    pub fn detect_mentions(
        mut self,
        room: &'a Room,
        pills: &'a mut Vec<Pill>,
        detect_at_room: bool,
    ) -> Self {
        self.mentions = MentionsMode::WithMentions {
            pills,
            room,
            detect_at_room,
        };
        self
    }

    /// Search and replace links in the given text.
    ///
    /// Returns the list of mentions, if any where found.
    pub fn linkify(mut self, text: &str) {
        let finder = LinkFinder::new();

        for span in finder.spans(text) {
            let span_text = span.as_str();

            let uri = match span.kind() {
                Some(LinkKind::Url) => {
                    if let MentionsMode::WithMentions { pills, room, .. } = &mut self.mentions {
                        if let Some(pill) = self.inner.maybe_append_mention(span_text, room) {
                            pills.push(pill);

                            continue;
                        }
                    }

                    Some(UriParts {
                        prefix: None,
                        uri: span_text,
                    })
                }
                Some(LinkKind::Email) => Some(UriParts {
                    prefix: Some("mailto:"),
                    uri: span_text,
                }),
                _ => {
                    if let MentionsMode::WithMentions {
                        pills,
                        room,
                        detect_at_room: true,
                    } = &mut self.mentions
                    {
                        if let Some(pill) = self.inner.append_and_replace_at_room(span_text, room) {
                            pills.push(pill);
                        }

                        continue;
                    }

                    None
                }
            };

            if let Some(uri) = uri {
                self.inner.append_link_opening_tag(uri.to_string());
            }

            self.inner.push_str(&span_text.escape_markup());

            if uri.is_some() {
                self.inner.push_str("</a>");
            }
        }
    }
}

/// The mentions mode of the [`Linkifier`].
#[derive(Debug, Default)]
enum MentionsMode<'a> {
    /// The builder will not detect mentions.
    #[default]
    NoMentions,
    /// The builder will detect mentions.
    WithMentions {
        /// The pills for the detected mentions.
        pills: &'a mut Vec<Pill>,
        /// The room containing the mentions.
        room: &'a Room,
        /// Whether to detect `@room` mentions.
        detect_at_room: bool,
    },
}

/// A URI that is possibly into parts.
#[derive(Debug, Clone, Copy)]
struct UriParts<'a> {
    prefix: Option<&'a str>,
    uri: &'a str,
}

impl<'a> fmt::Display for UriParts<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = self.prefix {
            f.write_str(prefix)?;
        }

        f.write_str(self.uri)
    }
}

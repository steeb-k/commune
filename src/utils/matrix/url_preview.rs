//! Finding the link in a message that a preview should be requested for.

use std::sync::LazyLock;

use linkify::{LinkFinder, LinkKind};
use ruma::{
    events::room::message::{FormattedBody, MessageFormat},
    html::{
        Children, Html, SanitizerConfig,
        matrix::{AnchorUri, MatrixElement},
    },
};
use url::Url;

#[cfg(test)]
mod tests;

/// The host of `matrix.to` URLs.
///
/// A link to one is a Matrix identifier rather than a page, and is rendered as
/// a pill, so it is never previewed.
const MATRIX_TO_HOST: &str = "matrix.to";

/// The scheme assumed for a link written without one.
const ASSUMED_SCHEME: &str = "https://";

/// The sanitizer used before looking for anchors.
///
/// The SDK already removes the reply fallback from the content it hands us, so
/// this is belt and braces: without it, the link of a replied-to message would
/// be previewed again under every reply to it.
static SANITIZER_CONFIG: LazyLock<SanitizerConfig> =
    LazyLock::new(|| SanitizerConfig::compat().remove_reply_fallback());

/// Find the link in the given message that a preview should be requested for.
///
/// Only the first link is returned: one card per message keeps both the number
/// of requests and the height of the message predictable.
///
/// Returns `None` if the message has no link that can be previewed.
pub(crate) fn previewable_url(body: &str, formatted: Option<&FormattedBody>) -> Option<Url> {
    if let Some(formatted) = formatted.filter(|f| f.format == MessageFormat::Html) {
        let html = Html::parse(&formatted.body);
        html.sanitize_with(&SANITIZER_CONFIG);

        return first_anchor_url(html.children());
    }

    first_plain_url(body)
}

/// Find the first previewable URL among the anchors of the given nodes.
fn first_anchor_url(children: Children) -> Option<Url> {
    for node in children {
        if let Some(element) = node.as_element() {
            match element.to_matrix().element {
                // A URL inside one of these was written to be read, not
                // followed.
                MatrixElement::Code(_) | MatrixElement::Pre => continue,
                MatrixElement::A(anchor) => {
                    // `AnchorUri` parses `matrix:` and `matrix.to` URIs into
                    // their own variants, so a mention never gets this far.
                    // Every other allowed scheme arrives as `Other`, including
                    // `ftp:`, `mailto:` and `magnet:`, which is what
                    // `is_previewable()` is for.
                    if let Some(AnchorUri::Other(uri)) = anchor.href
                        && let Some(url) = Url::parse(uri.as_ref()).ok().filter(is_previewable)
                    {
                        return Some(url);
                    }

                    // Nesting an anchor inside an anchor is not valid HTML, so
                    // there is nothing to find in the children.
                    continue;
                }
                _ => {}
            }
        }

        if let Some(url) = first_anchor_url(node.children()) {
            return Some(url);
        }
    }

    None
}

/// Find the first previewable URL in the given plain text.
fn first_plain_url(text: &str) -> Option<Url> {
    let mut finder = LinkFinder::new();
    // Match what the linkifier turns into a link, so that the card is always
    // for something the message actually shows as one.
    finder.url_must_have_scheme(false);

    let mut prev_span = None;

    for span in finder.spans(text) {
        let span_text = span.as_str();

        if span.kind() != Some(&LinkKind::Url) {
            prev_span = Some(span_text);
            continue;
        }

        if let Some(url) = span_url(span_text, prev_span) {
            return Some(url);
        }

        prev_span = Some(span_text);
    }

    None
}

/// The previewable URL of the given span detected as one, if it has one.
///
/// `prev_span` is the text that precedes it, which is what tells a link apart
/// from the homeserver part of a Matrix identifier.
fn span_url(span_text: &str, prev_span: Option<&str>) -> Option<Url> {
    if let Ok(url) = Url::parse(span_text) {
        // This is a full URL with a scheme, we can trust that it is valid.
        return Some(url).filter(is_previewable);
    }

    // It has no scheme. Only treat it as a link if its top-level domain is a
    // real one, like the linkifier does, or `1.5` and `e.g.` would both be
    // sent to the homeserver.
    if !has_known_tld(span_text) {
        return None;
    }

    // The link finder detects the homeserver part of Matrix identifiers and
    // `matrix:` URIs, e.g. `example.org` in `@alice:example.org`. Both put a
    // `:` right before it, and nothing else that reaches here does.
    if prev_span.is_some_and(|s| s.ends_with(':')) {
        return None;
    }

    Url::parse(&format!("{ASSUMED_SCHEME}{span_text}"))
        .ok()
        .filter(is_previewable)
}

/// Whether the domain of the given scheme-less link has a known top-level
/// domain.
fn has_known_tld(link: &str) -> bool {
    let domain = link
        .split_once(['/', '?', '#'])
        .map_or(link, |(domain, _)| domain);

    domain
        .rsplit_once('.')
        .is_some_and(|(_, tld)| tld::exist(tld))
}

/// Whether a preview can be requested for the given URL.
fn is_previewable(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }

    // A `matrix.to` link is a Matrix identifier, which the homeserver has
    // nothing to say about.
    url.host_str() != Some(MATRIX_TO_HOST)
}

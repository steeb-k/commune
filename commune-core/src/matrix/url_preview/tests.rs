use ruma::events::room::message::FormattedBody;

use super::previewable_url;

/// Find the previewable URL of a plain text message.
fn plain(body: &str) -> Option<String> {
    previewable_url(body, None).map(String::from)
}

/// Find the previewable URL of an HTML message.
fn html(body: &str) -> Option<String> {
    previewable_url("", Some(&FormattedBody::html(body))).map(String::from)
}

#[test]
fn plain_text_link() {
    assert_eq!(
        plain("look at https://example.org/post"),
        Some("https://example.org/post".to_owned())
    );
}

#[test]
fn plain_text_first_link_only() {
    assert_eq!(
        plain("https://example.org/one and https://example.com/two"),
        Some("https://example.org/one".to_owned())
    );
}

#[test]
fn plain_text_without_scheme() {
    assert_eq!(
        plain("see example.org/post"),
        Some("https://example.org/post".to_owned())
    );
}

#[test]
fn plain_text_false_positive_is_not_a_link() {
    // A version number and an abbreviation both look like domains to the link
    // finder, and neither has a real top-level domain.
    assert_eq!(plain("version 1.5 released"), None);
    assert_eq!(plain("e.g. this one"), None);
}

#[test]
fn plain_text_without_a_link() {
    assert_eq!(plain("nothing to see here"), None);
}

#[test]
fn other_schemes_are_not_previewed() {
    assert_eq!(plain("mailto:someone@example.org"), None);
    assert_eq!(plain("ftp://example.org/file"), None);
    assert_eq!(plain("magnet:?xt=urn:btih:abcdef"), None);
}

#[test]
fn matrix_identifiers_are_not_previewed() {
    assert_eq!(plain("https://matrix.to/#/@alice:example.org"), None);
    assert_eq!(plain("matrix:u/alice:example.org"), None);
    assert_eq!(plain("say hi to @alice:example.org"), None);
}

#[test]
fn html_anchor() {
    assert_eq!(
        html(r#"read <a href="https://example.org/post">this post</a>"#),
        Some("https://example.org/post".to_owned())
    );
}

#[test]
fn html_anchor_inside_a_block() {
    assert_eq!(
        html(r#"<blockquote><p>see <a href="https://example.org/post">here</a></p></blockquote>"#),
        Some("https://example.org/post".to_owned())
    );
}

#[test]
fn html_mention_is_not_previewed() {
    assert_eq!(
        html(r#"<a href="https://matrix.to/#/@alice:example.org">Alice</a>"#),
        None
    );
    assert_eq!(
        html(r#"<a href="matrix:u/alice:example.org">Alice</a>"#),
        None
    );
}

#[test]
fn html_mention_before_a_link() {
    assert_eq!(
        html(
            r#"<a href="https://matrix.to/#/@alice:example.org">Alice</a> wrote <a href="https://example.org/post">this</a>"#
        ),
        Some("https://example.org/post".to_owned())
    );
}

#[test]
fn html_link_in_code_is_not_previewed() {
    assert_eq!(html(r"<code>https://example.org/post</code>"), None);
    assert_eq!(
        html(r#"<pre><code><a href="https://example.org/post">x</a></code></pre>"#),
        None
    );
}

#[test]
fn html_reply_fallback_is_not_previewed() {
    assert_eq!(
        html(
            r#"<mx-reply><blockquote><a href="https://example.org/quoted">quoted</a></blockquote></mx-reply>agreed"#
        ),
        None
    );
}

#[test]
fn html_body_is_preferred_over_plain_body() {
    // The plain body is the fallback, and its link text can be something the
    // HTML never linked.
    let formatted = FormattedBody::html(r#"<a href="https://example.org/real">a page</a>"#);
    assert_eq!(
        previewable_url("https://example.com/fallback", Some(&formatted)).map(String::from),
        Some("https://example.org/real".to_owned())
    );
}

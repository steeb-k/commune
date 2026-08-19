use ruma::html::Html;

use super::{HTML_MESSAGE_SANITIZER_CONFIG, inline_html::InlineHtmlBuilder};

#[test]
fn text_with_no_markup() {
    let html = Html::parse("A simple text");
    let (s, pills) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(s, "A simple text");
    assert!(pills.is_none());
}

#[test]
fn single_line() {
    let html = Html::parse("A simple text<br>on several lines");
    let (s, pills) = InlineHtmlBuilder::new(true, false, false).build_with_nodes(html.children());

    assert_eq!(s, "A simple text…");
    assert!(pills.is_none());

    let html = Html::parse("\nThis is a paragraph<br />\n\nThis is another paragraph\n");
    let (s, pills) = InlineHtmlBuilder::new(true, false, false).build_with_nodes(html.children());

    assert_eq!(s, "This is a paragraph…");
    assert!(pills.is_none());
}

#[test]
fn add_ellipsis() {
    let html = Html::parse("A simple text");
    let (s, pills) = InlineHtmlBuilder::new(false, true, false).build_with_nodes(html.children());

    assert_eq!(s, "A simple text…");
    assert!(pills.is_none());
}

#[test]
fn no_duplicate_ellipsis() {
    let html = Html::parse("A simple text...<br>...on several lines");
    let (s, pills) = InlineHtmlBuilder::new(true, false, false).build_with_nodes(html.children());

    assert_eq!(s, "A simple text...");
    assert!(pills.is_none());
}

#[test]
fn trim_end_spaces() {
    let html = Html::parse("A high-altitude text 🗻   ");
    let (s, pills) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(s, "A high-altitude text 🗻");
    assert!(pills.is_none());
}

#[test]
fn collapse_whitespace() {
    let original = "Hello \nyou! \nYou are <b>my \nfriend</b>.";
    let html = Html::parse(original);

    let (s, pills) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());
    assert_eq!(s, "Hello you! You are <b>my friend</b>.");
    assert!(pills.is_none());

    let (s, pills) = InlineHtmlBuilder::new(false, false, true).build_with_nodes(html.children());
    assert_eq!(s, original);
    assert!(pills.is_none());

    let original = " Hello    \nyou! \n\nYou are \n<b>   my \nfriend   </b>.  ";
    let html = Html::parse(original);

    let (s, pills) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());
    assert_eq!(s, "Hello you! You are <b>my friend</b>.");
    assert!(pills.is_none());

    let (s, pills) = InlineHtmlBuilder::new(false, false, true).build_with_nodes(html.children());
    assert_eq!(s, original);
    assert!(pills.is_none());
}

#[test]
fn sanitize_inline_html() {
    let html = Html::parse(
        r#"A <strong>text</strong> with <a href="https://docs.local/markup"><i>markup</i></a>"#,
    );
    let (s, pills) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(
        s,
        r#"A <b>text</b> with <a href="https://docs.local/markup" title="https://docs.local/markup"><i>markup</i></a>"#
    );
    assert!(pills.is_none());
}

#[test]
fn escape_markup() {
    let html = Html::parse(
        r#"Go to <a href="https://docs.local?this=this&that=that">this &amp; that docs</a>"#,
    );
    let (s, pills) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(
        s,
        r#"Go to <a href="https://docs.local?this=this&amp;that=that" title="https://docs.local?this=this&amp;amp;that=that">this &amp; that docs</a>"#
    );
    assert!(pills.is_none());
}

#[test]
fn linkify() {
    let html = Html::parse(
        "The homepage is https://gnome.org, and you can contact me at contact@me.local",
    );
    let (s, pills) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(
        s,
        r#"The homepage is <a href="https://gnome.org" title="https://gnome.org">https://gnome.org</a>, and you can contact me at <a href="mailto:contact@me.local" title="mailto:contact@me.local">contact@me.local</a>"#
    );
    assert!(pills.is_none());
}

#[test]
fn do_not_linkify_inside_anchor() {
    let html = Html::parse(r#"The homepage is <a href="https://gnome.org">https://gnome.org</a>"#);
    let (s, pills) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(
        s,
        r#"The homepage is <a href="https://gnome.org" title="https://gnome.org">https://gnome.org</a>"#
    );
    assert!(pills.is_none());
}

#[test]
fn do_not_linkify_inside_code() {
    let html = Html::parse("The homepage is <code>https://gnome.org</code>");
    let (s, pills) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(s, "The homepage is <tt>https://gnome.org</tt>");
    assert!(pills.is_none());
}

#[test]
fn emote_name() {
    let html = Html::parse("sent a beautiful picture.");
    let (s, pills) = InlineHtmlBuilder::new(false, false, false)
        .append_emote_with_name(&mut Some("Jun"))
        .build_with_nodes(html.children());

    assert_eq!(s, "<b>Jun</b> sent a beautiful picture.");
    assert!(pills.is_none());
}

/// A custom emoticon survives the sanitizer.
///
/// Without a room, it cannot be loaded, so it falls back to its description.
#[test]
fn custom_emoticon() {
    let html = Html::parse(
        r#"Hello <img data-mx-emoticon src="mxc://example.org/abc" alt="a waving cat" title="cat_wave" height="32">"#,
    );
    html.sanitize_with(&HTML_MESSAGE_SANITIZER_CONFIG);

    let (s, widgets) =
        InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(s, "Hello a waving cat");
    assert!(widgets.is_none());
}

/// The description of a custom emoticon falls back to its shortcode.
#[test]
fn custom_emoticon_without_alt() {
    let html = Html::parse(
        r#"Hello <img data-mx-emoticon src="mxc://example.org/abc" title="cat_wave" height="32">"#,
    );
    html.sanitize_with(&HTML_MESSAGE_SANITIZER_CONFIG);

    let (s, _) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(s, "Hello cat_wave");
}

/// An image that is not served by the homeserver is never loaded, so that it
/// cannot be used to know when a message is read.
///
/// Only an `mxc:` URI ends up in the source of the image, so it falls back to
/// its description.
#[test]
fn custom_emoticon_with_remote_source() {
    let html = Html::parse(
        r#"Hello <img data-mx-emoticon src="https://example.org/abc" alt="a waving cat">"#,
    );
    html.sanitize_with(&HTML_MESSAGE_SANITIZER_CONFIG);

    let (s, widgets) =
        InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(s, "Hello a waving cat");
    assert!(widgets.is_none());
}

/// An image without the attribute is still presented as a custom emoticon,
/// because the SDK removes the attribute before we are given the message.
#[test]
fn image_without_the_emoticon_attribute() {
    let html = Html::parse(r#"Hello <img src="mxc://example.org/abc" alt="a waving cat">"#);
    html.sanitize_with(&HTML_MESSAGE_SANITIZER_CONFIG);

    // Without a room it cannot be loaded, so it still falls back here, but it
    // takes the same path as one that has the attribute.
    let (s, widgets) =
        InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(s, "Hello a waving cat");
    assert!(widgets.is_none());
}

/// The description of a custom emoticon is escaped like any other text.
#[test]
fn custom_emoticon_description_is_escaped() {
    let html =
        Html::parse(r#"<img data-mx-emoticon src="mxc://example.org/abc" alt="<b>bold</b>">"#);
    html.sanitize_with(&HTML_MESSAGE_SANITIZER_CONFIG);

    let (s, _) = InlineHtmlBuilder::new(false, false, false).build_with_nodes(html.children());

    assert_eq!(s, "&lt;b&gt;bold&lt;/b&gt;");
}

/// The sanitizer keeps what identifies a custom emoticon and where its image
/// is, which is what tells one apart from any other image.
#[test]
fn custom_emoticon_keeps_its_attributes() {
    use ruma::html::matrix::{MatrixElement, MatrixElementData};

    let html = Html::parse(
        r#"<img data-mx-emoticon src="mxc://example.org/abc" alt="a waving cat" title="cat_wave" height="32">"#,
    );
    html.sanitize_with(&HTML_MESSAGE_SANITIZER_CONFIG);

    let node = html
        .children()
        .find(|node| node.as_element().is_some())
        .expect("the image should not be removed");
    let MatrixElementData { element, attrs } = node
        .as_element()
        .expect("the node is an element")
        .to_matrix();

    let MatrixElement::Img(image) = element else {
        panic!("the element should be an image");
    };

    assert_eq!(
        image.src.as_deref().map(ToString::to_string),
        Some("mxc://example.org/abc".to_owned())
    );
    assert_eq!(
        image.alt.as_ref().map(ToString::to_string).as_deref(),
        Some("a waving cat")
    );
    assert!(
        attrs
            .iter()
            .any(|attr| attr.name.local.as_ref() == super::CUSTOM_EMOTICON_ATTRIBUTE),
        "the attribute marking a custom emoticon should be kept"
    );
}

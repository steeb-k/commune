//! A client for the [KLIPY] GIF API, used by the GIF search of the sticker
//! picker.
//!
//! The API key is set at build time with the `klipy-api-key` Meson option. When
//! it is empty this whole feature is inert: [`is_available()`] returns `false`
//! and the GIF tab of the sticker picker is never built.
//!
//! Only the GIF endpoints are used. The API mixes sponsored items into its
//! results, marked by their `type`; [`GifPage`] drops everything that is not a
//! GIF, so no advertising reaches the composer.
//!
//! No `customer_id` is sent. It is the identifier that the API uses to build a
//! profile of a user across requests, it is optional, and searching works
//! without it.
//!
//! [KLIPY]: https://klipy.com/

use gettextrs::gettext;
use gtk::glib;
use serde::Deserialize;

use crate::{
    config::KLIPY_API_KEY,
    utils::http::{self, CLIENT, HttpError},
};

/// The base URL of the API, without the key.
const API_BASE_URL: &str = "https://api.klipy.com/api/v1";

/// The number of GIFs requested per page.
///
/// Kept small because the results only appear once their previews are
/// downloaded, and the host that serves those is slow.
const PAGE_LENGTH: u32 = 16;

/// The maximum size of a response of the API, 4 MB.
///
/// A page of results is a few hundred kilobytes; this is only here so that a
/// misbehaving host cannot make us read forever.
const MAX_RESPONSE_SIZE: u64 = 4 * 1024 * 1024;

/// The maximum size of a GIF that we are willing to send, 4 MB.
///
/// The largest variant under this size is the one that is sent. Homeservers
/// enforce their own, usually larger, limit; this one is about not making
/// everyone in the room download 15 MB because a GIF was available in that
/// size, and about not making the sender wait for it either: the host these
/// come from serves about 300 KB/s.
const MAX_SEND_FILESIZE: u64 = 4 * 1024 * 1024;

/// Whether the GIF search is available in this build.
pub(crate) fn is_available() -> bool {
    !KLIPY_API_KEY.is_empty()
}

/// An error that occurred while talking to the API.
#[derive(Debug, thiserror::Error)]
pub(crate) enum KlipyError {
    /// The request could not be performed.
    #[error("Could not reach the GIF service: {0}")]
    Http(#[from] HttpError),
    /// The body of the response could not be parsed.
    #[error("Could not read the answer of the GIF service: {0}")]
    Deserialize(#[from] serde_json::Error),
    /// The service answered, but reported a failure.
    #[error("The GIF service reported an error")]
    Api,
}

/// Perform a `GET` request on the given path of the API, with the given query.
async fn get<T: for<'de> Deserialize<'de>>(
    path: &str,
    query: &[(&str, &str)],
) -> Result<T, KlipyError> {
    // The `query` feature of reqwest is not enabled, so the query string is
    // built here.
    let query_string = query
        .iter()
        .fold(
            url::form_urlencoded::Serializer::new(String::new()),
            |mut serializer, (key, value)| {
                serializer.append_pair(key, value);
                serializer
            },
        )
        .finish();
    let url = format!("{API_BASE_URL}/{KLIPY_API_KEY}/{path}?{query_string}");

    let body = http::fetch(&url, MAX_RESPONSE_SIZE).await?;
    let response: ApiResponse<T> = serde_json::from_slice(&body)?;

    response.data.ok_or(KlipyError::Api)
}

/// Search the GIFs matching the given query.
///
/// Pages are 1-indexed.
pub(crate) async fn search(query: &str, page: u32) -> Result<GifPage, KlipyError> {
    get(
        "gifs/search",
        &[
            ("q", query),
            ("page", &page.to_string()),
            ("per_page", &PAGE_LENGTH.to_string()),
        ],
    )
    .await
}

/// Tell the API that the GIF with the given slug was sent.
///
/// This is how the service counts a GIF as shared. It is best-effort: a
/// failure is not worth reporting to the user, who did send their GIF.
///
/// The slug is only valid for the response it came in, so this must be called
/// with the slug of the GIF that was presented, not a stored one.
pub(crate) async fn report_share(slug: &str) {
    let url = format!("{API_BASE_URL}/{KLIPY_API_KEY}/gifs/share/{slug}");

    if let Err(error) = CLIENT.post(url).send().await {
        tracing::debug!("Could not report a shared GIF: {error}");
    }
}

/// The envelope that every response of the API comes in.
#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    /// Whether the request succeeded.
    #[expect(dead_code, reason = "the absence of `data` is what we act on")]
    result: bool,
    /// The payload, absent when the request failed.
    data: Option<T>,
}

/// A page of GIFs.
#[derive(Debug, Default, Deserialize)]
#[serde(from = "GifPageDeser")]
pub(crate) struct GifPage {
    /// The GIFs of this page, with the sponsored items removed.
    pub(crate) gifs: Vec<Gif>,
    /// Whether another page can be requested.
    pub(crate) has_next: bool,
}

impl From<GifPageDeser> for GifPage {
    fn from(value: GifPageDeser) -> Self {
        Self {
            // Drop everything that is not a GIF. The API mixes in items with a
            // `type` of `ad`, which have no place in a composer.
            gifs: value
                .data
                .into_iter()
                .filter(|item| item.item_type == "gif")
                .collect(),
            has_next: value.has_next,
        }
    }
}

/// A page of GIFs, as it is on the wire.
#[derive(Debug, Deserialize)]
struct GifPageDeser {
    /// The items of this page, of every type.
    #[serde(default)]
    data: Vec<Gif>,
    /// Whether another page can be requested.
    #[serde(default)]
    has_next: bool,
}

/// A single GIF.
///
/// This is a boxed type so that it can be a property of the button that
/// presents it.
#[derive(Debug, Clone, Deserialize, glib::Boxed)]
#[boxed_type(name = "KlipyGif")]
// This is `pub` rather than `pub(crate)` so that it can be a property of a
// widget; the module itself is `pub(crate)`, so nothing escapes the crate.
pub struct Gif {
    /// The identifier of the GIF, stable across requests.
    pub(crate) id: i64,
    /// The identifier of the GIF for this response, used to report a share.
    #[serde(default)]
    pub(crate) slug: String,
    /// A human-readable description of the GIF.
    #[serde(default)]
    pub(crate) title: String,
    /// A blurred thumbnail of the GIF, as a `data:` URI.
    ///
    /// The API ships this inline with the results so that a client has
    /// something to present while the real preview is downloaded.
    #[serde(default)]
    blur_preview: Option<String>,
    /// The variants of the file, by size then by format.
    file: GifSizes,
    /// The kind of item this is. Only `gif` is presented.
    #[serde(rename = "type", default)]
    item_type: String,
}

impl Gif {
    /// The variant to present in the picker.
    ///
    /// The smallest one, and WebP over GIF. The host that serves these is
    /// slow — a page of `sm` variants is four megabytes and takes over ten
    /// seconds — and `xs` is about the size we present at anyway.
    pub(crate) fn preview(&self) -> Option<&GifFile> {
        [&self.file.xs, &self.file.sm, &self.file.md, &self.file.hd]
            .into_iter()
            .flatten()
            .find_map(|size| size.webp.as_ref().or(size.gif.as_ref()))
    }

    /// The blurred thumbnail to present until the preview arrives.
    pub(crate) fn blur_preview(&self) -> Option<&str> {
        self.blur_preview.as_deref()
    }

    /// The description of this GIF for the accessibility layer and the
    /// fallback body of the event.
    ///
    /// The API does not guarantee a title.
    pub(crate) fn title(&self) -> String {
        if self.title.is_empty() {
            gettext("GIF")
        } else {
            self.title.clone()
        }
    }

    /// The data needed to send this GIF, if it has a variant that can be sent.
    pub(crate) fn to_selection(&self) -> Option<SelectedGif> {
        let file = self.to_send()?;

        Some(SelectedGif {
            url: file.url.clone(),
            width: file.width,
            height: file.height,
            size: file.size,
            slug: self.slug.clone(),
            title: self.title(),
        })
    }

    /// The variant to upload and send, and its size.
    ///
    /// This is the largest GIF that is not bigger than [`MAX_SEND_FILESIZE`],
    /// so that a room is not made to download an enormous file when a smaller
    /// one is available.
    pub(crate) fn to_send(&self) -> Option<&GifFile> {
        let sizes = [&self.file.hd, &self.file.md, &self.file.sm, &self.file.xs];
        let gifs = sizes.into_iter().flatten().filter_map(|s| s.gif.as_ref());

        // The variants are in decreasing order of size, so the first one that
        // fits is the biggest one that fits.
        gifs.clone()
            .find(|file| file.size <= MAX_SEND_FILESIZE)
            // Every variant is too big. Send the smallest one and let the
            // homeserver be the judge.
            .or_else(|| gifs.last())
    }
}

/// A GIF that the user chose to send.
///
/// This is what the picker hands to the composer. It is a boxed type so that
/// it can travel through a signal.
#[derive(Debug, Clone, glib::Boxed)]
#[boxed_type(name = "SelectedGif")]
pub(crate) struct SelectedGif {
    /// Where the GIF can be downloaded.
    pub(crate) url: String,
    /// The width of the GIF, in pixels.
    pub(crate) width: u32,
    /// The height of the GIF, in pixels.
    pub(crate) height: u32,
    /// The size of the GIF, in bytes.
    pub(crate) size: u64,
    /// The identifier used to report the GIF as shared.
    pub(crate) slug: String,
    /// A description of the GIF.
    pub(crate) title: String,
}

/// The variants of a GIF, by size.
#[derive(Debug, Clone, Deserialize)]
struct GifSizes {
    /// The largest variant.
    hd: Option<GifFormats>,
    /// A medium variant.
    md: Option<GifFormats>,
    /// A small variant.
    sm: Option<GifFormats>,
    /// The smallest variant.
    xs: Option<GifFormats>,
}

/// The formats that one size of a GIF is available in.
///
/// The API also offers `mp4`, `webm` and `jpg`, which are not used.
#[derive(Debug, Clone, Deserialize)]
struct GifFormats {
    /// The GIF format.
    gif: Option<GifFile>,
    /// The WebP format.
    webp: Option<GifFile>,
}

/// One file of a GIF, in one size and one format.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GifFile {
    /// Where the file can be downloaded.
    pub(crate) url: String,
    /// The width of the file, in pixels.
    pub(crate) width: u32,
    /// The height of the file, in pixels.
    pub(crate) height: u32,
    /// The size of the file, in bytes.
    pub(crate) size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response with one GIF and one sponsored item, trimmed to the fields
    /// that are read.
    const PAGE_JSON: &str = r#"{
        "data": [
            {
                "id": 1,
                "slug": "a-cat--abcdef",
                "title": "A Cat",
                "type": "gif",
                "file": {
                    "hd": {
                        "gif": {"url": "https://e.test/hd.gif", "width": 498, "height": 456, "size": 727891},
                        "webp": {"url": "https://e.test/hd.webp", "width": 498, "height": 456, "size": 84000}
                    },
                    "sm": {
                        "gif": {"url": "https://e.test/sm.gif", "width": 220, "height": 201, "size": 271080},
                        "webp": {"url": "https://e.test/sm.webp", "width": 220, "height": 201, "size": 137582}
                    },
                    "xs": {
                        "gif": {"url": "https://e.test/xs.gif", "width": 87, "height": 80, "size": 99848},
                        "webp": {"url": "https://e.test/xs.webp", "width": 87, "height": 80, "size": 29218}
                    }
                }
            },
            {
                "id": 2,
                "slug": "an-ad--abcdef",
                "title": "Buy This",
                "type": "ad",
                "file": {"hd": {"gif": {"url": "https://e.test/ad.gif", "width": 300, "height": 250, "size": 1000}}}
            }
        ],
        "has_next": true
    }"#;

    #[test]
    fn deserialize_page_drops_ads() {
        let page: GifPage = serde_json::from_str(PAGE_JSON).unwrap();

        assert!(page.has_next);
        assert_eq!(page.gifs.len(), 1);
        assert_eq!(page.gifs[0].id, 1);
        assert_eq!(page.gifs[0].slug, "a-cat--abcdef");
    }

    #[test]
    fn preview_prefers_the_smallest_webp() {
        let page: GifPage = serde_json::from_str(PAGE_JSON).unwrap();
        let preview = page.gifs[0].preview().unwrap();

        assert_eq!(preview.url, "https://e.test/xs.webp");
    }

    #[test]
    fn preview_falls_back_to_a_bigger_size() {
        let mut page: GifPage = serde_json::from_str(PAGE_JSON).unwrap();
        page.gifs[0].file.xs = None;

        let preview = page.gifs[0].preview().unwrap();
        assert_eq!(preview.url, "https://e.test/sm.webp");
    }

    #[test]
    fn preview_falls_back_to_gif_without_webp() {
        let mut page: GifPage = serde_json::from_str(PAGE_JSON).unwrap();
        page.gifs[0].file.xs.as_mut().unwrap().webp = None;

        let preview = page.gifs[0].preview().unwrap();
        assert_eq!(preview.url, "https://e.test/xs.gif");
    }

    #[test]
    fn send_uses_the_biggest_gif_that_fits() {
        let page: GifPage = serde_json::from_str(PAGE_JSON).unwrap();
        let file = page.gifs[0].to_send().unwrap();

        assert_eq!(file.url, "https://e.test/hd.gif");
        assert_eq!(file.width, 498);
    }

    #[test]
    fn send_falls_back_to_a_smaller_gif() {
        let mut page: GifPage = serde_json::from_str(PAGE_JSON).unwrap();
        // Make every variant but the smallest one too big to send.
        let sizes = &mut page.gifs[0].file;
        sizes.hd.as_mut().unwrap().gif.as_mut().unwrap().size = MAX_SEND_FILESIZE + 1;
        sizes.sm.as_mut().unwrap().gif.as_mut().unwrap().size = MAX_SEND_FILESIZE + 1;

        let file = page.gifs[0].to_send().unwrap();
        assert_eq!(file.url, "https://e.test/xs.gif");
    }

    #[test]
    fn send_falls_back_to_the_smallest_when_nothing_fits() {
        let mut page: GifPage = serde_json::from_str(PAGE_JSON).unwrap();
        let sizes = &mut page.gifs[0].file;
        for size in [&mut sizes.hd, &mut sizes.sm, &mut sizes.xs] {
            size.as_mut().unwrap().gif.as_mut().unwrap().size = MAX_SEND_FILESIZE + 1;
        }

        let file = page.gifs[0].to_send().unwrap();
        assert_eq!(file.url, "https://e.test/xs.gif");
    }

    #[test]
    fn deserialize_empty_page() {
        let page: GifPage = serde_json::from_str(r#"{"data": [], "has_next": false}"#).unwrap();

        assert!(page.gifs.is_empty());
        assert!(!page.has_next);
    }
}

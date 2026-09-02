//! A preview of a URL, as the homeserver describes it.
//!
//! The response of the endpoint is free-form `OpenGraph` data: every
//! property is optional, and none of them can be trusted to have the type
//! we expect.
//!
//! The value half of the application's `RemoteUrlPreview`
//! (`src/session/remote/url_preview.rs`): the version check that decides
//! whether to ask at all, the refusal remembered for the whole session, and
//! the reading of the properties. What stayed in the application is the
//! `GObject` a card binds to.

use std::sync::{Arc, Mutex};

use ruma::{
    OwnedMxcUri, UInt,
    api::{MatrixVersion, client::authenticated_media::get_media_preview, error::ErrorKind},
    events::room::ImageInfo,
};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tracing::{debug, warn};
use url::Url;

use crate::{UserFacingError, session::Session, spawn_tokio};

/// The first Matrix version where the endpoint we use is stable.
///
/// Before that it only existed as MSC3916, under an unstable path. We never
/// send a request to an unstable path, so on an older homeserver this feature
/// is simply off.
const PREVIEW_URL_STABLE_SINCE: MatrixVersion = MatrixVersion::V1_11;

/// Whether the homeserver of a session can answer a URL preview request.
///
/// `None` means that we have not found out yet. This is shared between every
/// preview of a session, so that neither the version check nor a refusal is
/// paid for more than once.
#[derive(Debug, Clone, Default)]
pub struct UrlPreviewSupport {
    inner: Arc<Mutex<Option<bool>>>,
}

impl UrlPreviewSupport {
    /// Whether the homeserver can answer, if that is known.
    #[must_use]
    pub fn get(&self) -> Option<bool> {
        *self.inner.lock().expect("mutex is not poisoned")
    }

    /// Remember whether the homeserver can answer.
    fn set(&self, is_supported: bool) {
        *self.inner.lock().expect("mutex is not poisoned") = Some(is_supported);
    }
}

/// What can go wrong while asking for a preview.
#[derive(Debug, thiserror::Error)]
pub enum UrlPreviewError {
    /// The homeserver does not offer URL previews, by version or by
    /// refusal, and is not asked again.
    #[error("the homeserver does not offer URL previews")]
    Unsupported,
    /// Whether the homeserver offers URL previews could not be found out.
    ///
    /// Not knowing is not the same as knowing that it is missing, so this
    /// answer is not remembered.
    #[error("could not find out whether the homeserver offers URL previews")]
    UnknownSupport,
    /// The homeserver could not answer.
    ///
    /// Boxed because `matrix_sdk::HttpError` is large enough that carrying
    /// it by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(Box<matrix_sdk::HttpError>),
    /// The homeserver looked and found nothing to put on a card.
    ///
    /// An empty response is a valid answer, and so is one with none of the
    /// properties a card is made of.
    #[error("the homeserver found nothing to show")]
    Nothing,
    /// The homeserver's answer could not be read.
    #[error("the preview could not be read")]
    Malformed,
}

impl UserFacingError for UrlPreviewError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::Unsupported => "This homeserver does not offer link previews.".to_owned(),
            Self::UnknownSupport => {
                "Could not find out whether this homeserver offers link previews.".to_owned()
            }
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
            Self::Nothing => "There is nothing to show for this link.".to_owned(),
            Self::Malformed => "The preview of this link could not be read.".to_owned(),
        }
    }
}

/// The image of a URL preview.
#[derive(Debug, Clone)]
pub struct UrlPreviewImage {
    /// The `mxc:` URI of the image.
    pub uri: OwnedMxcUri,
    /// What the homeserver said about the image.
    pub info: ImageInfo,
}

/// A preview of a URL, as the homeserver describes it.
#[derive(Debug, Clone)]
pub struct UrlPreview {
    /// The URL that this is a preview of.
    pub url: Url,
    /// The title of the page, if it has one.
    pub title: Option<String>,
    /// The description of the page, if it has one.
    pub description: Option<String>,
    /// The name of the site the page belongs to.
    ///
    /// This falls back to the host of the URL, so it is never empty.
    pub site_name: String,
    /// The image of the page, if it has one.
    pub image: Option<UrlPreviewImage>,
}

/// The host of the given URL, which is what a card shows for the site
/// until the homeserver answers, and when it does not name the site.
#[must_use]
pub fn url_host(url: &Url) -> String {
    url.host_str().unwrap_or_default().to_owned()
}

impl Session {
    /// Ask the homeserver for a preview of the given URL.
    ///
    /// `support` is the session's memory of whether the homeserver can
    /// answer at all; it is filled in by the first answer.
    pub async fn url_preview(
        &self,
        url: &Url,
        support: &UrlPreviewSupport,
    ) -> Result<UrlPreview, UrlPreviewError> {
        if !self.supports_url_previews(support).await? {
            return Err(UrlPreviewError::Unsupported);
        }

        let client = self.client();
        let request = get_media_preview::v1::Request::new(url.to_string());
        let handle = spawn_tokio!(async move { client.send(request).await });

        let response = match handle.await.expect("task was not aborted") {
            Ok(response) => response,
            Err(preview_error) => {
                // A homeserver is allowed to switch previews off, and one
                // that has refuses every request the same way. Take the
                // first refusal for the whole session rather than asking
                // again for every link in the timeline.
                if preview_error.client_api_error_kind() == Some(&ErrorKind::Unrecognized) {
                    debug!("Homeserver does not offer URL previews, not asking again");
                    support.set(false);
                    return Err(UrlPreviewError::Unsupported);
                }

                warn!("Could not get a preview for `{url}`: {preview_error}");
                return Err(UrlPreviewError::Server(Box::new(preview_error)));
            }
        };

        let Some(data) = response.data else {
            // An empty response is a valid answer: the homeserver looked
            // and found nothing to show.
            return Err(UrlPreviewError::Nothing);
        };

        let data = serde_json::from_str::<JsonMap<String, JsonValue>>(data.get()).map_err(
            |deserialize_error| {
                warn!("Could not deserialize the preview for `{url}`: {deserialize_error}");
                UrlPreviewError::Malformed
            },
        )?;

        preview_from_data(url, &data).ok_or(UrlPreviewError::Nothing)
    }

    /// Whether the homeserver can answer a preview request.
    async fn supports_url_previews(
        &self,
        support: &UrlPreviewSupport,
    ) -> Result<bool, UrlPreviewError> {
        if let Some(is_supported) = support.get() {
            return Ok(is_supported);
        }

        let client = self.client();
        let handle = spawn_tokio!(async move { client.server_versions().await });

        let is_supported = match handle.await.expect("task was not aborted") {
            Ok(versions) => versions
                .iter()
                .any(|version| *version >= PREVIEW_URL_STABLE_SINCE),
            Err(versions_error) => {
                // Not knowing is not the same as knowing that it is
                // missing, so this answer is not remembered.
                warn!("Could not get the versions supported by the homeserver: {versions_error}");
                return Err(UrlPreviewError::UnknownSupport);
            }
        };

        if !is_supported {
            debug!("Homeserver does not support Matrix 1.11, URL previews are off");
        }

        support.set(is_supported);
        Ok(is_supported)
    }
}

/// The preview described by the given `OpenGraph` properties, if there is
/// anything in them to put on a card.
fn preview_from_data(url: &Url, data: &JsonMap<String, JsonValue>) -> Option<UrlPreview> {
    let title = string_property(data, "og:title");
    let description = string_property(data, "og:description");
    let image = image_property(data);

    if title.is_none() && description.is_none() && image.is_none() {
        // There is nothing to put on a card.
        return None;
    }

    let site_name = string_property(data, "og:site_name").unwrap_or_else(|| url_host(url));

    Some(UrlPreview {
        url: url.clone(),
        title,
        description,
        site_name,
        image,
    })
}

/// The value of the given property, if it is a non-empty string.
fn string_property(data: &JsonMap<String, JsonValue>, name: &str) -> Option<String> {
    let value = data.get(name)?.as_str()?.trim();

    (!value.is_empty()).then(|| value.to_owned())
}

/// The value of the given property, if it is a number that fits in a `UInt`.
fn uint_property(data: &JsonMap<String, JsonValue>, name: &str) -> Option<UInt> {
    let value = data.get(name)?;

    // Some homeservers send these as strings, so accept both.
    let number = value
        .as_u64()
        .or_else(|| value.as_str()?.parse::<u64>().ok())?;

    UInt::try_from(number).ok()
}

/// The image of the preview, if the given properties describe one.
fn image_property(data: &JsonMap<String, JsonValue>) -> Option<UrlPreviewImage> {
    let uri = OwnedMxcUri::from(string_property(data, "og:image")?);

    // The spec says that this is an `mxc:` URI. A homeserver that sent an
    // ordinary URL instead would have us fetch it ourselves, straight from
    // whoever the page links to, which is the one thing a preview is meant to
    // avoid.
    if uri.parts().is_err() {
        debug!("Ignoring a preview image that is not an `mxc:` URI");
        return None;
    }

    let mut info = ImageInfo::new();
    info.width = uint_property(data, "og:image:width");
    info.height = uint_property(data, "og:image:height");
    info.mimetype = string_property(data, "og:image:type");
    info.size = uint_property(data, "matrix:image:size");

    Some(UrlPreviewImage { uri, info })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(json: &str) -> JsonMap<String, JsonValue> {
        serde_json::from_str(json).expect("test JSON is valid")
    }

    #[test]
    fn a_title_alone_makes_a_card() {
        let url = Url::parse("https://example.org/page").expect("valid URL");
        let preview = preview_from_data(&url, &data(r#"{"og:title": " Hello "}"#))
            .expect("a title is enough");

        assert_eq!(preview.title.as_deref(), Some("Hello"));
        assert_eq!(preview.description, None);
        assert!(preview.image.is_none());
        // The site falls back to the host.
        assert_eq!(preview.site_name, "example.org");
    }

    #[test]
    fn nothing_to_show_is_no_card() {
        let url = Url::parse("https://example.org/page").expect("valid URL");

        assert!(preview_from_data(&url, &data(r#"{"og:site_name": "Example"}"#)).is_none());
        assert!(preview_from_data(&url, &data(r#"{"og:title": "   "}"#)).is_none());
    }

    #[test]
    fn an_image_that_is_not_mxc_is_ignored() {
        let url = Url::parse("https://example.org/page").expect("valid URL");
        let preview = preview_from_data(
            &url,
            &data(r#"{"og:title": "Hello", "og:image": "https://example.org/a.png"}"#),
        )
        .expect("the title is enough");

        assert!(preview.image.is_none());
    }

    #[test]
    fn image_dimensions_are_read_as_numbers_or_strings() {
        let url = Url::parse("https://example.org/page").expect("valid URL");
        let preview = preview_from_data(
            &url,
            &data(
                r#"{"og:image": "mxc://example.org/abc", "og:image:width": 640, "og:image:height": "480", "matrix:image:size": "12345"}"#,
            ),
        )
        .expect("an image is enough");

        let image = preview.image.expect("the image was read");
        assert_eq!(image.uri.as_str(), "mxc://example.org/abc");
        assert_eq!(image.info.width, Some(UInt::from(640u32)));
        assert_eq!(image.info.height, Some(UInt::from(480u32)));
        assert_eq!(image.info.size, Some(UInt::from(12345u32)));
    }
}

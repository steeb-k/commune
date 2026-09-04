//! Fetching media into files the UI can display.
//!
//! The SDK's media API hands back bytes (cached in the store); the UIs want
//! files — Compose decodes a path, and so will any other front end. This
//! writes each fetched media to a stable file under the cache directory,
//! keyed by the request's own unique key, and returns the path. A file that
//! is already there is returned without a fetch.

use std::path::PathBuf;

use matrix_sdk::{
    Client,
    media::{MediaEventContent, MediaFormat, MediaRequestParameters, MediaThumbnailSettings},
};
use ruma::{
    MxcUri, UInt,
    events::{
        room::{
            MediaSource,
            message::{
                AudioMessageEventContent, FileMessageEventContent, FormattedBody,
                ImageMessageEventContent, MessageType, VideoMessageEventContent,
            },
        },
        sticker::StickerEventContent,
    },
};
use tracing::error;

use crate::{matrix::ext_traits::FormattedBodyExt, paths::DataType, spawn_tokio, utils::StrMutExt};

/// A media message: a message whose content is a file to fetch.
///
/// The application's `MediaMessage` (`src/utils/matrix/media_message.rs`)
/// since Phase 4's module 8: the variants, the caption and the fetch, as
/// bytes or as a file. What stayed behind, as an extension trait over
/// this, is every method that renders a name — "Voice Message", the
/// generated filename of a voice message — because they are sentences, and
/// the save dialog, because it is a widget.
#[derive(Debug, Clone)]
pub enum MediaMessage {
    /// An audio.
    Audio(AudioMessageEventContent),
    /// A file.
    File(FileMessageEventContent),
    /// An image.
    Image(ImageMessageEventContent),
    /// A video.
    Video(VideoMessageEventContent),
    /// A sticker.
    Sticker(Box<StickerEventContent>),
}

impl MediaMessage {
    /// Construct a `MediaMessage` from the given message.
    #[must_use]
    pub fn from_message(msgtype: &MessageType) -> Option<Self> {
        match msgtype {
            MessageType::Audio(c) => Some(Self::Audio(c.clone())),
            MessageType::File(c) => Some(Self::File(c.clone())),
            MessageType::Image(c) => Some(Self::Image(c.clone())),
            MessageType::Video(c) => Some(Self::Video(c.clone())),
            _ => None,
        }
    }

    /// The source of the file of this media.
    ///
    /// An encrypted source carries its keys, which is why a media message
    /// is taken from the event rather than rebuilt from a URI.
    #[must_use]
    pub fn source(&self) -> Option<MediaSource> {
        match self {
            Self::Audio(c) => c.source(),
            Self::File(c) => c.source(),
            Self::Image(c) => c.source(),
            Self::Video(c) => c.source(),
            Self::Sticker(c) => c.source(),
        }
    }

    /// The caption of the media, if any.
    ///
    /// Returns `Some((body, formatted_body))` if the media includes a
    /// caption: a body that is not only whitespace, and its formatted
    /// counterpart with the same rule.
    #[must_use]
    pub fn caption(&self) -> Option<(String, Option<FormattedBody>)> {
        let mut caption = match self {
            Self::Audio(c) => c
                .caption()
                .map(|caption| (caption.to_owned(), c.formatted.clone())),
            Self::File(c) => c
                .caption()
                .map(|caption| (caption.to_owned(), c.formatted.clone())),
            Self::Image(c) => c
                .caption()
                .map(|caption| (caption.to_owned(), c.formatted.clone())),
            Self::Video(c) => c
                .caption()
                .map(|caption| (caption.to_owned(), c.formatted.clone())),
            Self::Sticker(_) => None,
        };

        caption.take_if(|(caption, formatted)| {
            caption.clean_string();
            formatted.clean_string();

            caption.is_empty()
        });

        caption
    }

    /// Fetch the content of this media with the given client.
    ///
    /// Returns an error if something occurred while fetching the content.
    pub async fn into_content(self, client: &Client) -> Result<Vec<u8>, matrix_sdk::Error> {
        let media = client.media();

        macro_rules! content {
            ($event_content:expr) => {{
                Ok(
                    spawn_tokio!(async move { media.get_file(&$event_content, true).await })
                        .await
                        .expect("task was not aborted")?
                        .expect("All media message types have a file"),
                )
            }};
        }

        match self {
            Self::Audio(c) => content!(c),
            Self::File(c) => content!(c),
            Self::Image(c) => content!(c),
            Self::Video(c) => content!(c),
            Self::Sticker(c) => content!(*c),
        }
    }

    /// Fetch the content of this media with the given client into a file,
    /// returning its path.
    ///
    /// Returns `None` if the fetch failed, as [`get_media_file()`] does.
    pub async fn into_file(self, client: &Client) -> Option<PathBuf> {
        let source = self.source()?;
        let request = MediaRequestParameters {
            source,
            format: MediaFormat::File,
        };

        get_media_file(client, request).await
    }

    /// The source of the still picture the sender attached to this media,
    /// if any: the `thumbnail_source` of its info.
    ///
    /// This is what the application's history viewer draws for a video
    /// (`VisualMediaMessage::thumbnail`): the picture the event carries,
    /// never a frame of the video, which would mean fetching the video.
    #[must_use]
    pub fn thumbnail_source(&self) -> Option<MediaSource> {
        match self {
            Self::Image(c) => c.info.as_deref()?.thumbnail_source.clone(),
            Self::Video(c) => c.info.as_deref()?.thumbnail_source.clone(),
            Self::Sticker(c) => c.info.thumbnail_source.clone(),
            Self::Audio(_) | Self::File(_) => None,
        }
    }

    /// Fetch the picture the sender attached to this media into a file,
    /// scaled by the media repo to fit `size` when it can, returning the
    /// path.
    ///
    /// The application's `ThumbnailDownloader` asks the media repo for a
    /// scaled copy of a plain source, since it cannot scale an encrypted
    /// one, and falls back to the whole source; so does this. A media
    /// without an attached picture yields `None`, as the application
    /// leaves its placeholder.
    pub async fn into_thumbnail_file(self, client: &Client, size: u32) -> Option<PathBuf> {
        let source = self.thumbnail_source()?;

        if !matches!(source, MediaSource::Encrypted(_)) {
            let request = MediaRequestParameters {
                source: source.clone(),
                format: MediaFormat::Thumbnail(MediaThumbnailSettings::new(
                    UInt::from(size),
                    UInt::from(size),
                )),
            };
            if let Some(path) = get_media_file(client, request).await {
                return Some(path);
            }
        }

        let request = MediaRequestParameters {
            source,
            format: MediaFormat::File,
        };
        get_media_file(client, request).await
    }
}

impl From<AudioMessageEventContent> for MediaMessage {
    fn from(value: AudioMessageEventContent) -> Self {
        Self::Audio(value)
    }
}

impl From<FileMessageEventContent> for MediaMessage {
    fn from(value: FileMessageEventContent) -> Self {
        Self::File(value)
    }
}

impl From<StickerEventContent> for MediaMessage {
    fn from(value: StickerEventContent) -> Self {
        Self::Sticker(value.into())
    }
}

/// Fetch the media for the given request, returning the path of a file
/// holding it.
///
/// Returns `None` if the fetch failed.
pub async fn get_media_file(client: &Client, request: MediaRequestParameters) -> Option<PathBuf> {
    use matrix_sdk::media::UniqueKey;

    let dir = DataType::Cache.dir_path().join("media");
    let file_name: String = request
        .unique_key()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(file_name);

    if path.exists() {
        return Some(path);
    }

    let client = client.clone();
    let bytes = spawn_tokio!(async move { client.media().get_media_content(&request, true).await })
        .await
        .expect("task was not aborted")
        .inspect_err(|fetch_error| {
            error!("Could not fetch media: {fetch_error}");
        })
        .ok()?;

    let path_clone = path.clone();
    let written = spawn_tokio!(async move {
        tokio::fs::create_dir_all(path_clone.parent().expect("media dir has a parent")).await?;
        tokio::fs::write(&path_clone, bytes).await
    })
    .await
    .expect("task was not aborted");

    match written {
        Ok(()) => Some(path),
        Err(write_error) => {
            error!("Could not write media file: {write_error}");
            None
        }
    }
}

/// Fetch a square thumbnail of the media at the given `mxc:` URI — an
/// avatar — returning the path of a file holding it.
pub async fn get_avatar_file(client: &Client, uri: &MxcUri, size: u32) -> Option<PathBuf> {
    let request = MediaRequestParameters {
        source: MediaSource::Plain(uri.to_owned()),
        format: MediaFormat::Thumbnail(MediaThumbnailSettings::new(
            UInt::from(size),
            UInt::from(size),
        )),
    };

    get_media_file(client, request).await
}

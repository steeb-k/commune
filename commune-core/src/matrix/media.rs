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
                AudioMessageEventContent, FileMessageEventContent, ImageMessageEventContent,
                MessageType, VideoMessageEventContent,
            },
        },
        sticker::StickerEventContent,
    },
};
use tracing::error;

use crate::{paths::DataType, spawn_tokio};

/// A media message: a message whose content is a file to fetch.
///
/// The portable half of the application's `MediaMessage`
/// (`src/utils/matrix/media_message.rs`): the variants and the fetch. What
/// stayed behind is every method that renders a name — "Voice Message",
/// the generated filename of a voice message — because they are sentences,
/// and the save dialog, because it is a widget.
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

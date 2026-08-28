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
    media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings},
};
use ruma::{MxcUri, UInt, events::room::MediaSource};
use tracing::error;

use crate::{paths::DataType, spawn_tokio};

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

use gettextrs::gettext;
use gtk::{gio, glib, prelude::*};
use matrix_sdk::Client;
use ruma::{
    events::{
        room::message::{
            AudioMessageEventContent, FileMessageEventContent, FormattedBody,
            ImageMessageEventContent, MessageType, VideoMessageEventContent,
        },
        sticker::StickerEventContent,
    },
    UInt,
};
use tracing::{debug, error};

use crate::{
    prelude::*,
    toast,
    utils::{
        media::image::{ImageSource, ThumbnailDownloader, ThumbnailSettings},
        save_data_to_tmp_file,
    },
};

/// Get the filename of a media message.
macro_rules! filename {
    ($message:ident, $mime_fallback:expr) => {{
        let mut filename = match &$message.filename {
            Some(filename) if *filename != $message.body => filename.clone(),
            _ => $message.body.clone(),
        };

        if filename.is_empty() {
            let mimetype = $message
                .info
                .as_ref()
                .and_then(|info| info.mimetype.as_deref());

            filename = $crate::utils::media::filename_for_mime(mimetype, $mime_fallback);
        }

        filename
    }};
}

/// Get the caption of a media message.
macro_rules! caption {
    ($message:ident) => {{
        $message
            .filename
            .as_deref()
            .filter(|filename| *filename != $message.body)
            .map(|_| ($message.body.clone(), $message.formatted.clone()))
    }};
}

/// A media message.
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
    Sticker(StickerEventContent),
}

impl MediaMessage {
    /// Construct a `MediaMessage` from the given message.
    pub fn from_message(msgtype: &MessageType) -> Option<Self> {
        match msgtype {
            MessageType::Audio(c) => Some(Self::Audio(c.clone())),
            MessageType::File(c) => Some(Self::File(c.clone())),
            MessageType::Image(c) => Some(Self::Image(c.clone())),
            MessageType::Video(c) => Some(Self::Video(c.clone())),
            _ => None,
        }
    }

    /// The filename of the media.
    ///
    /// For a sticker, this returns the description of the sticker.
    pub fn filename(&self) -> String {
        match self {
            Self::Audio(c) => filename!(c, Some(mime::AUDIO)),
            Self::File(c) => filename!(c, None),
            Self::Image(c) => filename!(c, Some(mime::IMAGE)),
            Self::Video(c) => filename!(c, Some(mime::VIDEO)),
            Self::Sticker(c) => c.body.clone(),
        }
    }

    /// The caption of the media, if any.
    ///
    /// Returns `Some((body, formatted_body))` if the media includes a caption.
    pub fn caption(&self) -> Option<(String, Option<FormattedBody>)> {
        match self {
            Self::Audio(c) => caption!(c),
            Self::File(c) => caption!(c),
            Self::Image(c) => caption!(c),
            Self::Video(c) => caption!(c),
            Self::Sticker(_) => None,
        }
    }

    /// Fetch the content of the media with the given client.
    ///
    /// Returns an error if something occurred while fetching the content.
    pub async fn into_content(self, client: &Client) -> Result<Vec<u8>, matrix_sdk::Error> {
        let media = client.media();

        macro_rules! content {
            ($event_content:ident) => {{
                Ok(
                    $crate::spawn_tokio!(
                        async move { media.get_file(&$event_content, true).await }
                    )
                    .await
                    .unwrap()?
                    .expect("All media message types have a file"),
                )
            }};
        }

        match self {
            Self::Audio(c) => content!(c),
            Self::File(c) => content!(c),
            Self::Image(c) => content!(c),
            Self::Video(c) => content!(c),
            Self::Sticker(c) => content!(c),
        }
    }

    /// Fetch the content of the media with the given client and write it to a
    /// temporary file.
    ///
    /// Returns an error if something occurred while fetching the content.
    pub async fn into_tmp_file(self, client: &Client) -> Result<gio::File, MediaFileError> {
        let data = self.into_content(client).await?;
        Ok(save_data_to_tmp_file(&data)?)
    }

    /// Save the content of the media to a file selected by the user.
    ///
    /// Shows a dialog to the user to select a file on the system.
    pub async fn save_to_file(self, client: &Client, parent: &impl IsA<gtk::Widget>) {
        let filename = self.filename();

        let data = match self.into_content(client).await {
            Ok(data) => data,
            Err(error) => {
                error!("Could not retrieve media file: {error}");
                toast!(parent, error.to_user_facing());

                return;
            }
        };

        let dialog = gtk::FileDialog::builder()
            .title(gettext("Save File"))
            .modal(true)
            .accept_label(gettext("Save"))
            .initial_name(filename)
            .build();

        match dialog
            .save_future(parent.root().and_downcast_ref::<gtk::Window>())
            .await
        {
            Ok(file) => {
                if let Err(error) = file.replace_contents(
                    &data,
                    None,
                    false,
                    gio::FileCreateFlags::REPLACE_DESTINATION,
                    gio::Cancellable::NONE,
                ) {
                    error!("Could not save file: {error}");
                    toast!(parent, gettext("Could not save file"));
                }
            }
            Err(error) => {
                if error.matches(gtk::DialogError::Dismissed) {
                    debug!("File dialog dismissed by user");
                } else {
                    error!("Could not access file: {error}");
                    toast!(parent, gettext("Could not access file"));
                }
            }
        };
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
        Self::Sticker(value)
    }
}

/// A visual media message.
#[derive(Debug, Clone)]
pub enum VisualMediaMessage {
    /// An image.
    Image(ImageMessageEventContent),
    /// A video.
    Video(VideoMessageEventContent),
    /// A sticker.
    Sticker(StickerEventContent),
}

impl VisualMediaMessage {
    /// Construct a `VisualMediaMessage` from the given message.
    pub fn from_message(msgtype: &MessageType) -> Option<Self> {
        match msgtype {
            MessageType::Image(c) => Some(Self::Image(c.clone())),
            MessageType::Video(c) => Some(Self::Video(c.clone())),
            _ => None,
        }
    }

    /// The filename of the media.
    ///
    /// For a sticker, this returns the description of the sticker.
    pub fn filename(&self) -> String {
        match self {
            Self::Image(c) => filename!(c, Some(mime::IMAGE)),
            Self::Video(c) => filename!(c, Some(mime::VIDEO)),
            Self::Sticker(c) => c.body.clone(),
        }
    }

    /// The caption of the media, if any.
    ///
    /// Returns `Some((body, formatted_body))` if the media includes a caption.
    pub fn caption(&self) -> Option<(String, Option<FormattedBody>)> {
        match self {
            Self::Image(c) => caption!(c),
            Self::Video(c) => caption!(c),
            Self::Sticker(_) => None,
        }
    }

    /// The dimensions of the media, if any.
    ///
    /// Returns a `(width, height)` tuple.
    pub fn dimensions(&self) -> Option<(UInt, UInt)> {
        match self {
            Self::Image(c) => c.info.as_ref().and_then(|i| i.width.zip(i.height)),
            Self::Video(c) => c.info.as_ref().and_then(|i| i.width.zip(i.height)),
            Self::Sticker(c) => c.info.width.zip(c.info.height),
        }
    }

    /// Fetch a thumbnail of the media with the given client and thumbnail
    /// settings.
    ///
    /// This might not return a thumbnail at the requested size, depending on
    /// the message and the homeserver.
    ///
    /// Returns `Ok(None)` if no thumbnail could be retrieved and no fallback
    /// could be downloaded. This only applies to video messages.
    ///
    /// Returns an error if something occurred while fetching the content.
    pub async fn thumbnail(
        &self,
        client: &Client,
        settings: ThumbnailSettings,
    ) -> Result<Option<Vec<u8>>, matrix_sdk::Error> {
        let downloader = match &self {
            Self::Image(c) => {
                let image_info = c.info.as_deref();
                ThumbnailDownloader {
                    thumbnail: image_info.and_then(|i| {
                        i.thumbnail_source.as_ref().map(|s| ImageSource {
                            source: s.into(),
                            info: i.thumbnail_info.as_deref().map(Into::into),
                        })
                    }),
                    original: Some(ImageSource {
                        source: (&c.source).into(),
                        info: image_info.map(Into::into),
                    }),
                }
            }
            Self::Video(c) => {
                let video_info = c.info.as_deref();
                ThumbnailDownloader {
                    thumbnail: video_info.and_then(|i| {
                        i.thumbnail_source.as_ref().map(|s| ImageSource {
                            source: s.into(),
                            info: i.thumbnail_info.as_deref().map(Into::into),
                        })
                    }),
                    original: None,
                }
            }
            Self::Sticker(c) => {
                let image_info = &c.info;
                ThumbnailDownloader {
                    thumbnail: image_info.thumbnail_source.as_ref().map(|s| ImageSource {
                        source: s.into(),
                        info: image_info.thumbnail_info.as_deref().map(Into::into),
                    }),
                    original: Some(ImageSource {
                        source: (&c.source).into(),
                        info: Some(image_info.into()),
                    }),
                }
            }
        };

        downloader.download(client, settings).await
    }

    /// Fetch a thumbnail of the media with the given client and thumbnail
    /// settings and write it to a temporary file.
    ///
    /// This might not return a thumbnail at the requested size, depending on
    /// the message and the homeserver.
    ///
    /// Returns `Ok(None)` if no thumbnail could be retrieved and no fallback
    /// could be downloaded. This only applies to video messages.
    ///
    /// Returns an error if something occurred while fetching the content or
    /// saving the content to a file.
    pub async fn thumbnail_tmp_file(
        &self,
        client: &Client,
        settings: ThumbnailSettings,
    ) -> Result<Option<gio::File>, MediaFileError> {
        let data = self.thumbnail(client, settings).await?;

        let Some(data) = data else {
            return Ok(None);
        };

        Ok(Some(save_data_to_tmp_file(&data)?))
    }

    /// Fetch the content of the media with the given client.
    ///
    /// Returns an error if something occurred while fetching the content.
    pub async fn into_content(self, client: &Client) -> Result<Vec<u8>, matrix_sdk::Error> {
        MediaMessage::from(self).into_content(client).await
    }

    /// Fetch the content of the media with the given client and write it to a
    /// temporary file.
    ///
    /// Returns an error if something occurred while fetching the content or
    /// saving the content to a file.
    pub async fn into_tmp_file(self, client: &Client) -> Result<gio::File, MediaFileError> {
        MediaMessage::from(self).into_tmp_file(client).await
    }

    /// Save the content of the media to a file selected by the user.
    ///
    /// Shows a dialog to the user to select a file on the system.
    pub async fn save_to_file(self, client: &Client, parent: &impl IsA<gtk::Widget>) {
        MediaMessage::from(self).save_to_file(client, parent).await
    }
}

impl From<ImageMessageEventContent> for VisualMediaMessage {
    fn from(value: ImageMessageEventContent) -> Self {
        Self::Image(value)
    }
}

impl From<VideoMessageEventContent> for VisualMediaMessage {
    fn from(value: VideoMessageEventContent) -> Self {
        Self::Video(value)
    }
}

impl From<StickerEventContent> for VisualMediaMessage {
    fn from(value: StickerEventContent) -> Self {
        Self::Sticker(value)
    }
}

impl From<VisualMediaMessage> for MediaMessage {
    fn from(value: VisualMediaMessage) -> Self {
        match value {
            VisualMediaMessage::Image(c) => Self::Image(c),
            VisualMediaMessage::Video(c) => Self::Video(c),
            VisualMediaMessage::Sticker(c) => Self::Sticker(c),
        }
    }
}

/// All errors that can occur when downloading a media to a file.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub enum MediaFileError {
    /// An error occurred when downloading the media.
    Sdk(#[from] matrix_sdk::Error),
    /// An error occurred when writing the media to a file.
    File(#[from] glib::Error),
}

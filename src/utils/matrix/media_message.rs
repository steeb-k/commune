use matrix_sdk::Client;
use ruma::events::room::message::{
    AudioMessageEventContent, FileMessageEventContent, FormattedBody, ImageMessageEventContent,
    MessageType, VideoMessageEventContent,
};

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
    pub fn filename(&self) -> String {
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

        match self {
            Self::Audio(c) => filename!(c, Some(mime::AUDIO)),
            Self::File(c) => filename!(c, None),
            Self::Image(c) => filename!(c, Some(mime::IMAGE)),
            Self::Video(c) => filename!(c, Some(mime::VIDEO)),
        }
    }

    /// The caption of the media, if any.
    ///
    /// Returns `Some((body, formatted_body))` if the media includes a caption.
    pub fn caption(&self) -> Option<(String, Option<FormattedBody>)> {
        macro_rules! caption {
            ($message:ident) => {{
                $message
                    .filename
                    .as_deref()
                    .filter(|filename| *filename != $message.body)
                    .map(|_| ($message.body.clone(), $message.formatted.clone()))
            }};
        }

        match self {
            Self::Audio(c) => caption!(c),
            Self::File(c) => caption!(c),
            Self::Image(c) => caption!(c),
            Self::Video(c) => caption!(c),
        }
    }

    /// Fetch the content of the media with the given client.
    ///
    /// Returns an error if something occurred while fetching the content.
    pub async fn content(self, client: Client) -> Result<Vec<u8>, matrix_sdk::Error> {
        let media = client.media();

        macro_rules! data {
            ($content:ident) => {{
                Ok(
                    $crate::spawn_tokio!(async move { media.get_file(&$content, true).await })
                        .await
                        .unwrap()?
                        .expect("All media message types have a file"),
                )
            }};
        }

        match self {
            Self::Audio(c) => data!(c),
            Self::File(c) => data!(c),
            Self::Image(c) => data!(c),
            Self::Video(c) => data!(c),
        }
    }
}

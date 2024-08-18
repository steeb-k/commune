//! Collection of methods for images.

use std::str::FromStr;

use gtk::{gdk, gio, prelude::*};
use image::{ColorType, DynamicImage, ImageDecoder, ImageResult};
use matrix_sdk::{
    attachment::{BaseImageInfo, BaseThumbnailInfo, Thumbnail},
    media::{MediaFormat, MediaRequest, MediaThumbnailSettings, MediaThumbnailSize},
    Client,
};
use ruma::{
    api::client::media::get_content_thumbnail::v3::Method,
    events::{
        room::{
            avatar::ImageInfo as AvatarImageInfo, ImageInfo, MediaSource as CommonMediaSource,
            ThumbnailInfo,
        },
        sticker::StickerMediaSource,
    },
    OwnedMxcUri,
};
use tracing::warn;

use crate::{
    components::AnimatedImagePaintable,
    spawn_tokio,
    utils::{matrix::MediaFileError, save_data_to_tmp_file},
    DISABLE_GLYCIN_SANDBOX,
};

/// The default width of a generated thumbnail.
const THUMBNAIL_DEFAULT_WIDTH: u32 = 800;
/// The default height of a generated thumbnail.
const THUMBNAIL_DEFAULT_HEIGHT: u32 = 600;
/// The content type of SVG.
const SVG_CONTENT_TYPE: &str = "image/svg+xml";
/// The content type of WebP.
const WEBP_CONTENT_TYPE: &str = "image/webp";
/// The default WebP quality used for a generated thumbnail.
const WEBP_DEFAULT_QUALITY: f32 = 60.0;
/// The maximum file size threshold in bytes for generating a thumbnail.
///
/// If the file size of the original image is larger than this, we assume it is
/// worth it to generate a thumbnail, even if its dimensions are smaller than
/// wanted. This is particularly helpful for some image formats that can take up
/// a lot of space.
///
/// This is 1MB.
const THUMBNAIL_MAX_FILESIZE_THRESHOLD: u32 = 1024 * 1024;
/// The dimension threshold in pixels before we start to generate a thumbnail.
///
/// If the original image is larger than thumbnail_dimensions + threshold, we
/// assume it's worth it to generate a thumbnail.
const THUMBNAIL_DIMENSIONS_THRESHOLD: u32 = 200;

/// Get an image reader for the given file.
async fn image_reader(file: gio::File) -> Result<glycin::Image<'static>, glycin::ErrorCtx> {
    let mut loader = glycin::Loader::new(file);

    if DISABLE_GLYCIN_SANDBOX {
        loader.sandbox_selector(glycin::SandboxSelector::NotSandboxed);
    }

    spawn_tokio!(async move { loader.load().await })
        .await
        .unwrap()
}

/// Load the given file as an image into a `GdkPaintable`.
pub async fn load_image(file: gio::File) -> Result<gdk::Paintable, glycin::ErrorCtx> {
    let image = image_reader(file).await?;

    let (image, first_frame) = spawn_tokio!(async move {
        let first_frame = image.next_frame().await?;
        Ok((image, first_frame))
    })
    .await
    .unwrap()?;

    let paintable = if first_frame.delay().is_some() {
        AnimatedImagePaintable::new(image, first_frame).upcast()
    } else {
        first_frame.texture().upcast()
    };

    Ok(paintable)
}

/// An API to load image information.
pub enum ImageInfoLoader {
    /// An image file.
    File(gio::File),
    /// A texture in memory.
    Texture(gdk::Texture),
}

impl ImageInfoLoader {
    /// Load the first frame for this source.
    ///
    /// We need to load the first frame of an image so that EXIF rotation is
    /// applied and we get the proper dimensions.
    async fn into_first_frame(self) -> Option<Frame> {
        match self {
            Self::File(file) => {
                let image_reader = image_reader(file).await.ok()?;
                let handle = spawn_tokio!(async move { image_reader.next_frame().await });
                Some(Frame::Glycin(handle.await.unwrap().ok()?))
            }
            Self::Texture(texture) => Some(Frame::Texture(gdk::TextureDownloader::new(&texture))),
        }
    }

    /// Load the information for this image.
    pub async fn load_info(self) -> BaseImageInfo {
        self.into_first_frame()
            .await
            .map(|f| f.dimensions())
            .unwrap_or_default()
            .into()
    }

    /// Load the information for this image and try to generate a thumbnail
    /// given the filesize of the original image.
    pub async fn load_info_and_thumbnail(
        self,
        filesize: Option<u32>,
    ) -> (BaseImageInfo, Option<Thumbnail>) {
        let Some(frame) = self.into_first_frame().await else {
            return (ImageDimensions::default().into(), None);
        };

        let dimensions = frame.dimensions();
        let info = dimensions.into();

        if !filesize.is_some_and(|s| s >= THUMBNAIL_MAX_FILESIZE_THRESHOLD)
            && !dimensions
                .width
                .is_some_and(|w| w > (THUMBNAIL_DEFAULT_WIDTH + THUMBNAIL_DIMENSIONS_THRESHOLD))
            && !dimensions
                .height
                .is_some_and(|h| h > (THUMBNAIL_DEFAULT_HEIGHT + THUMBNAIL_DIMENSIONS_THRESHOLD))
        {
            // It is not worth it to generate a thumbnail.
            return (info, None);
        }

        let thumbnail = frame.generate_thumbnail();

        (info, thumbnail)
    }
}

impl From<gio::File> for ImageInfoLoader {
    fn from(value: gio::File) -> Self {
        Self::File(value)
    }
}

impl From<gdk::Texture> for ImageInfoLoader {
    fn from(value: gdk::Texture) -> Self {
        Self::Texture(value)
    }
}

/// A frame of an image.
enum Frame {
    /// A frame loaded via glycin.
    Glycin(glycin::Frame),
    /// A downloader for a texture in memory,
    Texture(gdk::TextureDownloader),
}

impl Frame {
    /// The dimensions of the frame.
    fn dimensions(&self) -> ImageDimensions {
        match self {
            Self::Glycin(frame) => ImageDimensions {
                width: Some(frame.width()),
                height: Some(frame.height()),
            },
            Self::Texture(downloader) => {
                let texture = downloader.texture();
                ImageDimensions {
                    width: texture.width().try_into().ok(),
                    height: texture.height().try_into().ok(),
                }
            }
        }
    }

    /// Whether the memory format of the frame is supported by the image crate.
    fn is_supported(&self) -> bool {
        match self {
            Self::Glycin(frame) => {
                matches!(
                    frame.memory_format(),
                    glycin::MemoryFormat::G8
                        | glycin::MemoryFormat::G8a8
                        | glycin::MemoryFormat::R8g8b8
                        | glycin::MemoryFormat::R8g8b8a8
                        | glycin::MemoryFormat::G16
                        | glycin::MemoryFormat::G16a16
                        | glycin::MemoryFormat::R16g16b16
                        | glycin::MemoryFormat::R16g16b16a16
                        | glycin::MemoryFormat::R32g32b32Float
                        | glycin::MemoryFormat::R32g32b32a32Float
                )
            }
            Self::Texture(downloader) => {
                matches!(
                    downloader.format(),
                    gdk::MemoryFormat::G8
                        | gdk::MemoryFormat::G8a8
                        | gdk::MemoryFormat::R8g8b8
                        | gdk::MemoryFormat::R8g8b8a8
                        | gdk::MemoryFormat::G16
                        | gdk::MemoryFormat::G16a16
                        | gdk::MemoryFormat::R16g16b16
                        | gdk::MemoryFormat::R16g16b16a16
                        | gdk::MemoryFormat::R32g32b32Float
                        | gdk::MemoryFormat::R32g32b32a32Float
                )
            }
        }
    }

    /// Generate a thumbnail of this frame.
    fn generate_thumbnail(self) -> Option<Thumbnail> {
        if !self.is_supported() {
            return None;
        }

        let image = DynamicImage::from_decoder(self).ok()?;
        let thumbnail = image.thumbnail(THUMBNAIL_DEFAULT_WIDTH, THUMBNAIL_DEFAULT_HEIGHT);

        prepare_thumbnail_for_sending(thumbnail)
    }
}

impl ImageDecoder for Frame {
    fn dimensions(&self) -> (u32, u32) {
        let dimensions = self.dimensions();
        (
            dimensions.width.unwrap_or(0),
            dimensions.height.unwrap_or(0),
        )
    }

    fn color_type(&self) -> ColorType {
        match self {
            Self::Glycin(frame) => match frame.memory_format() {
                glycin::MemoryFormat::G8 => ColorType::L8,
                glycin::MemoryFormat::G8a8 => ColorType::La8,
                glycin::MemoryFormat::R8g8b8 => ColorType::Rgb8,
                glycin::MemoryFormat::R8g8b8a8 => ColorType::Rgba8,
                glycin::MemoryFormat::G16 => ColorType::L16,
                glycin::MemoryFormat::G16a16 => ColorType::La16,
                glycin::MemoryFormat::R16g16b16 => ColorType::Rgb16,
                glycin::MemoryFormat::R16g16b16a16 => ColorType::Rgba16,
                glycin::MemoryFormat::R32g32b32Float => ColorType::Rgb32F,
                glycin::MemoryFormat::R32g32b32a32Float => ColorType::Rgba32F,
                _ => unimplemented!(),
            },
            Self::Texture(downloader) => match downloader.format() {
                gdk::MemoryFormat::G8 => ColorType::L8,
                gdk::MemoryFormat::G8a8 => ColorType::La8,
                gdk::MemoryFormat::R8g8b8 => ColorType::Rgb8,
                gdk::MemoryFormat::R8g8b8a8 => ColorType::Rgba8,
                gdk::MemoryFormat::G16 => ColorType::L16,
                gdk::MemoryFormat::G16a16 => ColorType::La16,
                gdk::MemoryFormat::R16g16b16 => ColorType::Rgb16,
                gdk::MemoryFormat::R16g16b16a16 => ColorType::Rgba16,
                gdk::MemoryFormat::R32g32b32Float => ColorType::Rgb32F,
                gdk::MemoryFormat::R32g32b32a32Float => ColorType::Rgba32F,
                _ => unimplemented!(),
            },
        }
    }

    fn read_image(self, buf: &mut [u8]) -> ImageResult<()>
    where
        Self: Sized,
    {
        let bytes = match &self {
            Self::Glycin(frame) => frame.buf_bytes(),
            Self::Texture(texture) => texture.download_bytes().0,
        };
        buf.copy_from_slice(&bytes);

        Ok(())
    }

    fn read_image_boxed(self: Box<Self>, _buf: &mut [u8]) -> ImageResult<()> {
        unimplemented!()
    }
}

/// Dimensions of an image.
#[derive(Debug, Clone, Copy, Default)]
struct ImageDimensions {
    /// The width of the image.
    width: Option<u32>,
    /// The height of the image.
    height: Option<u32>,
}

/// Compute the dimensions of the thumbnail while preserving the aspect ratio of
/// the image.
///
/// Returns `None` if the dimensions are smaller than the wanted dimensions.
pub(super) fn thumbnail_dimensions(width: u32, height: u32) -> Option<(u32, u32)> {
    if width <= (THUMBNAIL_DEFAULT_WIDTH + THUMBNAIL_DIMENSIONS_THRESHOLD)
        && height <= (THUMBNAIL_DEFAULT_HEIGHT + THUMBNAIL_DIMENSIONS_THRESHOLD)
    {
        return None;
    }

    let w_ratio = width as f64 / THUMBNAIL_DEFAULT_WIDTH as f64;
    let h_ratio = height as f64 / THUMBNAIL_DEFAULT_HEIGHT as f64;

    if w_ratio > h_ratio {
        let new_height = height as f64 / w_ratio;
        Some((THUMBNAIL_DEFAULT_WIDTH, new_height as u32))
    } else {
        let new_width = width as f64 / h_ratio;
        Some((new_width as u32, THUMBNAIL_DEFAULT_HEIGHT))
    }
}

/// Prepare the given thumbnail to send it.
pub(super) fn prepare_thumbnail_for_sending(thumbnail: image::DynamicImage) -> Option<Thumbnail> {
    // Convert to RGB8/RGBA8 since those are the only formats supported by WebP.
    let thumbnail: DynamicImage = match &thumbnail {
        DynamicImage::ImageLuma8(_)
        | DynamicImage::ImageRgb8(_)
        | DynamicImage::ImageLuma16(_)
        | DynamicImage::ImageRgb16(_)
        | DynamicImage::ImageRgb32F(_) => thumbnail.into_rgb8().into(),
        DynamicImage::ImageLumaA8(_)
        | DynamicImage::ImageRgba8(_)
        | DynamicImage::ImageLumaA16(_)
        | DynamicImage::ImageRgba16(_)
        | DynamicImage::ImageRgba32F(_) => thumbnail.into_rgba8().into(),
        _ => return None,
    };

    // Encode to WebP.
    let encoder = webp::Encoder::from_image(&thumbnail).ok()?;
    let thumbnail_bytes = encoder.encode(WEBP_DEFAULT_QUALITY).to_vec();

    let thumbnail_content_type =
        mime::Mime::from_str(WEBP_CONTENT_TYPE).expect("image should provide a valid content type");

    let thumbnail_info = BaseThumbnailInfo {
        width: Some(thumbnail.width().into()),
        height: Some(thumbnail.height().into()),
        size: thumbnail_bytes.len().try_into().ok(),
    };

    Some(Thumbnail {
        data: thumbnail_bytes,
        content_type: thumbnail_content_type,
        info: Some(thumbnail_info),
    })
}

impl From<ImageDimensions> for BaseImageInfo {
    fn from(value: ImageDimensions) -> Self {
        let ImageDimensions { width, height } = value;
        BaseImageInfo {
            height: height.map(Into::into),
            width: width.map(Into::into),
            size: None,
            blurhash: None,
        }
    }
}

/// An API to download a thumbnail for a media.
#[derive(Debug, Clone, Copy)]
pub struct ThumbnailDownloader<'a> {
    /// The main source of the image.
    ///
    /// This should be the source with the best quality.
    pub main: ImageSource<'a>,
    /// An alternative source for the image.
    ///
    /// This should be a source with a lower quality.
    pub alt: Option<ImageSource<'a>>,
}

impl<'a> ThumbnailDownloader<'a> {
    /// Download the thumbnail of the media.
    ///
    /// This might not return a thumbnail at the requested size, depending on
    /// the sources and the homeserver.
    ///
    /// Returns `Ok(None)` if no thumbnail could be retrieved. Returns an error
    /// if something occurred while fetching the content.
    pub async fn download_to_file(
        self,
        client: &Client,
        settings: ThumbnailSettings,
    ) -> Result<gio::File, MediaFileError> {
        // First, select which source we are going to download from.
        let source = if let Some(alt) = self.alt {
            if !self.main.can_be_thumbnailed()
                && (self.main.filesize_is_too_big()
                    || alt.dimensions_are_too_big(settings.width, settings.height, false))
            {
                // Use the alternative as source to save bandwidth.
                alt
            } else {
                self.main
            }
        } else {
            self.main
        };

        let data = if source.should_thumbnail(
            settings.prefer_thumbnail,
            settings.width,
            settings.height,
        ) {
            // Try to get a thumbnail.
            let media = client.media();
            let request = MediaRequest {
                source: source.source.to_common_media_source(),
                format: MediaFormat::Thumbnail(settings.into()),
            };
            let handle = spawn_tokio!(async move { media.get_media_content(&request, true).await });

            match handle.await.unwrap() {
                Ok(data) => Some(data),
                Err(error) => {
                    warn!("Could not retrieve media thumbnail: {error}");
                    None
                }
            }
        } else {
            None
        };

        // Fallback to downloading the full source.
        let data = if let Some(data) = data {
            data
        } else {
            let media = client.media();
            let request = MediaRequest {
                source: source.source.to_common_media_source(),
                format: MediaFormat::File,
            };

            spawn_tokio!(async move { media.get_media_content(&request, true).await })
                .await
                .unwrap()?
        };

        Ok(save_data_to_tmp_file(&data)?)
    }
}

/// The source of an image.
#[derive(Debug, Clone, Copy)]
pub struct ImageSource<'a> {
    /// The source of the image.
    pub source: MediaSource<'a>,
    /// Information about the image.
    pub info: Option<ImageSourceInfo<'a>>,
}

impl<'a> ImageSource<'a> {
    /// Whether we should try to thumbnail this source for the given requested
    /// dimensions.
    fn should_thumbnail(
        &self,
        prefer_thumbnail: bool,
        requested_width: u32,
        requested_height: u32,
    ) -> bool {
        if !self.can_be_thumbnailed() {
            return false;
        }

        if prefer_thumbnail && !self.has_dimensions() {
            return true;
        }

        self.filesize_is_too_big()
            || self.dimensions_are_too_big(requested_width, requested_height, true)
    }

    /// Whether this source can be thumbnailed by the media repo.
    ///
    /// Returns `false` in these cases:
    ///
    /// - The image is encrypted, because it is not possible for the media repo
    ///   to make a thumbnail.
    /// - The image uses the SVG format, because media repos usually do not
    ///   accept to create a thumbnail of those.
    fn can_be_thumbnailed(&self) -> bool {
        !self.source.is_encrypted()
            && !self
                .info
                .is_some_and(|i| i.mimetype.is_some_and(|m| m == SVG_CONTENT_TYPE))
    }

    /// Whether the filesize of this source is too big.
    ///
    /// It means that it is worth it to download a thumbnail instead of the
    /// original file, even if its dimensions are smaller than requested.
    fn filesize_is_too_big(&self) -> bool {
        self.info
            .is_some_and(|i| i.size.is_some_and(|s| s > THUMBNAIL_MAX_FILESIZE_THRESHOLD))
    }

    /// Whether we have the dimensions of this source.
    fn has_dimensions(&self) -> bool {
        self.info
            .is_some_and(|i| i.width.is_some() && i.height.is_some())
    }

    /// Whether the dimensions of this source are too big for the given
    /// requested dimensions.
    fn dimensions_are_too_big(
        &self,
        requested_width: u32,
        requested_height: u32,
        increase_threshold: bool,
    ) -> bool {
        self.info.is_some_and(|i| {
            i.width.is_some_and(|w| {
                let threshold = if increase_threshold {
                    requested_width.saturating_add(THUMBNAIL_DIMENSIONS_THRESHOLD)
                } else {
                    requested_width
                };

                w > threshold
            }) || i.height.is_some_and(|h| {
                let threshold = if increase_threshold {
                    requested_height.saturating_add(THUMBNAIL_DIMENSIONS_THRESHOLD)
                } else {
                    requested_height
                };

                h > threshold
            })
        })
    }
}

/// The source of a media file.
#[derive(Debug, Clone, Copy)]
pub enum MediaSource<'a> {
    /// A common media source.
    Common(&'a CommonMediaSource),
    /// The media source of a sticker.
    Sticker(&'a StickerMediaSource),
    /// An MXC URI.
    Uri(&'a OwnedMxcUri),
}

impl<'a> MediaSource<'a> {
    /// Whether this source is encrypted.
    fn is_encrypted(&self) -> bool {
        match self {
            Self::Common(source) => matches!(source, CommonMediaSource::Encrypted(_)),
            Self::Sticker(source) => matches!(source, StickerMediaSource::Encrypted(_)),
            Self::Uri(_) => false,
        }
    }

    /// Get this source as a `CommonMediaSource`.
    fn to_common_media_source(self) -> CommonMediaSource {
        match self {
            Self::Common(source) => source.clone(),
            Self::Sticker(source) => source.clone().into(),
            Self::Uri(uri) => CommonMediaSource::Plain(uri.clone()),
        }
    }
}

impl<'a> From<&'a CommonMediaSource> for MediaSource<'a> {
    fn from(value: &'a CommonMediaSource) -> Self {
        Self::Common(value)
    }
}

impl<'a> From<&'a StickerMediaSource> for MediaSource<'a> {
    fn from(value: &'a StickerMediaSource) -> Self {
        Self::Sticker(value)
    }
}

impl<'a> From<&'a OwnedMxcUri> for MediaSource<'a> {
    fn from(value: &'a OwnedMxcUri) -> Self {
        Self::Uri(value)
    }
}

/// Information about the source of an image.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageSourceInfo<'a> {
    /// The width of the image.
    width: Option<u32>,
    /// The height of the image.
    height: Option<u32>,
    /// The MIME type of the image.
    mimetype: Option<&'a str>,
    /// The file size of the image.
    size: Option<u32>,
}

impl<'a> From<&'a ImageInfo> for ImageSourceInfo<'a> {
    fn from(value: &'a ImageInfo) -> Self {
        Self {
            width: value.width.and_then(|u| u.try_into().ok()),
            height: value.height.and_then(|u| u.try_into().ok()),
            mimetype: value.mimetype.as_deref(),
            size: value.size.and_then(|u| u.try_into().ok()),
        }
    }
}

impl<'a> From<&'a ThumbnailInfo> for ImageSourceInfo<'a> {
    fn from(value: &'a ThumbnailInfo) -> Self {
        Self {
            width: value.width.and_then(|u| u.try_into().ok()),
            height: value.height.and_then(|u| u.try_into().ok()),
            mimetype: value.mimetype.as_deref(),
            size: value.size.and_then(|u| u.try_into().ok()),
        }
    }
}

impl<'a> From<&'a AvatarImageInfo> for ImageSourceInfo<'a> {
    fn from(value: &'a AvatarImageInfo) -> Self {
        Self {
            width: value.width.and_then(|u| u.try_into().ok()),
            height: value.height.and_then(|u| u.try_into().ok()),
            mimetype: value.mimetype.as_deref(),
            size: value.size.and_then(|u| u.try_into().ok()),
        }
    }
}

/// The settings for downloading a thumbnail.
#[derive(Debug, Clone)]
pub struct ThumbnailSettings {
    /// The resquested width of the thumbnail.
    pub width: u32,
    /// The requested height of the thumbnail.
    pub height: u32,
    /// The method to use to resize the thumbnail.
    pub method: Method,
    /// Whether to request an animated thumbnail.
    pub animated: bool,
    /// Whether we should prefer to get a thumbnail if dimensions are unknown.
    ///
    /// This is particularly useful for avatars where we will prefer to save
    /// bandwidth and memory usage as we download a lot of them and they might
    /// appear several times on the screen. For media messages, we will on the
    /// contrary prefer to download the original content to reduce the space
    /// taken in the media cache.
    pub prefer_thumbnail: bool,
}

impl From<ThumbnailSettings> for MediaThumbnailSettings {
    fn from(value: ThumbnailSettings) -> Self {
        let ThumbnailSettings {
            width,
            height,
            method,
            animated,
            ..
        } = value;

        MediaThumbnailSettings {
            size: MediaThumbnailSize {
                method,
                width: width.into(),
                height: height.into(),
            },
            animated,
        }
    }
}

//! Collection of methods for images.

use std::{error::Error, fmt, str::FromStr, sync::Arc};

use gettextrs::gettext;
use gtk::{gdk, gio, prelude::*};
use image::{ColorType, DynamicImage, ImageDecoder, ImageResult};
use matrix_sdk::{
    attachment::{BaseImageInfo, BaseThumbnailInfo, Thumbnail},
    media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings},
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

mod queue;

pub(crate) use queue::{ImageRequestPriority, IMAGE_QUEUE};

use super::{FrameDimensions, MediaFileError};
use crate::{components::AnimatedImagePaintable, spawn_tokio, utils::File, DISABLE_GLYCIN_SANDBOX};

/// The maximum dimensions of a generated thumbnail.
const THUMBNAIL_MAX_DIMENSIONS: FrameDimensions = FrameDimensions {
    width: 800,
    height: 600,
};
/// The content type of SVG.
const SVG_CONTENT_TYPE: &str = "image/svg+xml";
/// The content type of WebP.
const WEBP_CONTENT_TYPE: &str = "image/webp";
/// The default WebP quality used for a generated thumbnail.
const WEBP_DEFAULT_QUALITY: f32 = 60.0;
/// The maximum file size threshold in bytes for requesting or generating a
/// thumbnail.
///
/// If the file size of the original image is larger than this, we assume it is
/// worth it to request or generate a thumbnail, even if its dimensions are
/// smaller than wanted. This is particularly helpful for some image formats
/// that can take up a lot of space.
///
/// This is 1MB.
const THUMBNAIL_MAX_FILESIZE_THRESHOLD: u32 = 1024 * 1024;
/// The size threshold in pixels for requesting or generating a thumbnail.
///
/// If the original image is larger than dimensions + threshold, we assume it is
/// worth it to request or generate a thumbnail.
const THUMBNAIL_DIMENSIONS_THRESHOLD: u32 = 200;

/// Get an image loader for the given file.
async fn image_loader(file: gio::File) -> Result<glycin::Image<'static>, glycin::ErrorCtx> {
    let mut loader = glycin::Loader::new(file);

    if DISABLE_GLYCIN_SANDBOX {
        loader.sandbox_selector(glycin::SandboxSelector::NotSandboxed);
    }

    spawn_tokio!(async move { loader.load().await })
        .await
        .unwrap()
}

/// Load the given file as an image into a `GdkPaintable`.
///
/// Set `request_dimensions` if the image will be shown at specific dimensions.
/// To show the image at its natural size, set it to `None`.
async fn load_image(
    file: File,
    request_dimensions: Option<FrameDimensions>,
) -> Result<Image, glycin::ErrorCtx> {
    let image_loader = image_loader(file.as_gfile()).await?;

    let frame_request = request_dimensions.map(|request| {
        let image_info = image_loader.info();

        let original_dimensions = FrameDimensions {
            width: image_info.width,
            height: image_info.height,
        };

        original_dimensions.to_image_loader_request(request)
    });

    spawn_tokio!(async move {
        let first_frame = if let Some(frame_request) = frame_request {
            image_loader.specific_frame(frame_request).await?
        } else {
            image_loader.next_frame().await?
        };
        Ok(Image {
            file,
            loader: image_loader.into(),
            first_frame: first_frame.into(),
        })
    })
    .await
    .expect("task was not aborted")
}

/// An image that was just loaded.
#[derive(Clone)]
pub struct Image {
    /// The file of the image.
    file: File,
    /// The image loader.
    loader: Arc<glycin::Image<'static>>,
    /// The first frame of the image.
    first_frame: Arc<glycin::Frame>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image").finish_non_exhaustive()
    }
}

impl From<Image> for gdk::Paintable {
    fn from(value: Image) -> Self {
        if value.first_frame.delay().is_some() {
            AnimatedImagePaintable::new(value.file, value.loader, value.first_frame).upcast()
        } else {
            value.first_frame.texture().upcast()
        }
    }
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
                let image_loader = image_loader(file).await.ok()?;
                let handle = spawn_tokio!(async move { image_loader.next_frame().await });
                Some(Frame::Glycin(handle.await.unwrap().ok()?))
            }
            Self::Texture(texture) => Some(Frame::Texture(gdk::TextureDownloader::new(&texture))),
        }
    }

    /// Load the information for this image.
    pub async fn load_info(self) -> BaseImageInfo {
        self.into_first_frame()
            .await
            .and_then(|f| f.dimensions())
            .map_or_else(default_base_image_info, Into::into)
    }

    /// Load the information for this image and try to generate a thumbnail
    /// given the filesize of the original image.
    pub async fn load_info_and_thumbnail(
        self,
        filesize: Option<u32>,
    ) -> (BaseImageInfo, Option<Thumbnail>) {
        let Some(frame) = self.into_first_frame().await else {
            return (default_base_image_info(), None);
        };

        let dimensions = frame.dimensions();
        let info = dimensions.map_or_else(default_base_image_info, Into::into);

        if !filesize_is_too_big(filesize)
            && !dimensions.is_some_and(|d| d.needs_thumbnail(THUMBNAIL_MAX_DIMENSIONS))
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
    fn dimensions(&self) -> Option<FrameDimensions> {
        let (width, height) = match self {
            Self::Glycin(frame) => (frame.width(), frame.height()),
            Self::Texture(downloader) => {
                let texture = downloader.texture();
                (
                    texture.width().try_into().ok()?,
                    texture.height().try_into().ok()?,
                )
            }
        };

        Some(FrameDimensions { width, height })
    }

    /// Whether the memory format of the frame is supported by the image crate.
    fn is_memory_format_supported(&self) -> bool {
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
        if !self.is_memory_format_supported() {
            return None;
        }

        let image = DynamicImage::from_decoder(self).ok()?;
        let thumbnail = image.thumbnail(
            THUMBNAIL_MAX_DIMENSIONS.width,
            THUMBNAIL_MAX_DIMENSIONS.height,
        );

        prepare_thumbnail_for_sending(thumbnail)
    }
}

impl ImageDecoder for Frame {
    fn dimensions(&self) -> (u32, u32) {
        let (width, height) = self.dimensions().map(|s| (s.width, s.height)).unzip();
        (width.unwrap_or(0), height.unwrap_or(0))
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

/// Extensions to `FrameDimensions` for computing thumbnail dimensions.
impl FrameDimensions {
    /// Whether we should generate or request a thumbnail for these dimensions,
    /// given the wanted thumbnail dimensions.
    pub(super) fn needs_thumbnail(self, thumbnail_dimensions: FrameDimensions) -> bool {
        self.ge(thumbnail_dimensions.increase_by(THUMBNAIL_DIMENSIONS_THRESHOLD))
    }

    /// Downscale these dimensions for a thumbnail while preserving the aspect
    /// ratio.
    ///
    /// Returns `None` if these dimensions are smaller than the dimensions of a
    /// thumbnail.
    pub(super) fn downscale_for_thumbnail(self) -> Option<Self> {
        if !self.needs_thumbnail(THUMBNAIL_MAX_DIMENSIONS) {
            // We do not need to generate a thumbnail.
            return None;
        }

        Some(self.scale_to_fit(THUMBNAIL_MAX_DIMENSIONS, gtk::ContentFit::ScaleDown))
    }

    /// Convert these dimensions to a request for the image loader with the
    /// requested dimensions.
    fn to_image_loader_request(self, requested: Self) -> glycin::FrameRequest {
        let scaled = self.scale_to_fit(requested, gtk::ContentFit::Cover);
        glycin::FrameRequest::new().scale(scaled.width, scaled.height)
    }
}

impl From<FrameDimensions> for BaseImageInfo {
    fn from(value: FrameDimensions) -> Self {
        let FrameDimensions { width, height } = value;
        BaseImageInfo {
            height: Some(height.into()),
            width: Some(width.into()),
            ..default_base_image_info()
        }
    }
}

/// The default value for `BaseImageInfo`.
fn default_base_image_info() -> BaseImageInfo {
    BaseImageInfo {
        height: None,
        width: None,
        size: None,
        blurhash: None,
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
        mime::Mime::from_str(WEBP_CONTENT_TYPE).expect("content type should be valid");

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
    /// This might not return a thumbnail at the requested dimensions, depending
    /// on the sources and the homeserver.
    pub async fn download(
        self,
        client: Client,
        settings: ThumbnailSettings,
        priority: ImageRequestPriority,
    ) -> Result<Image, ImageError> {
        let dimensions = settings.dimensions;

        // First, select which source we are going to download from.
        let source = if let Some(alt) = self.alt {
            if !self.main.can_be_thumbnailed()
                && (filesize_is_too_big(self.main.filesize())
                    || alt.dimensions().is_some_and(|s| s.ge(settings.dimensions)))
            {
                // Use the alternative source to save bandwidth.
                alt
            } else {
                self.main
            }
        } else {
            self.main
        };

        if source.should_thumbnail(settings.prefer_thumbnail, settings.dimensions) {
            // Try to get a thumbnail.
            let request = MediaRequestParameters {
                source: source.source.to_common_media_source(),
                format: MediaFormat::Thumbnail(settings.into()),
            };
            let handle = IMAGE_QUEUE
                .add_download_request(client.clone(), request, Some(dimensions), priority)
                .await;

            if let Ok(image) = handle.await {
                return Ok(image);
            }
        }

        // Fallback to downloading the full source.
        let request = MediaRequestParameters {
            source: source.source.to_common_media_source(),
            format: MediaFormat::File,
        };
        let handle = IMAGE_QUEUE
            .add_download_request(client, request, Some(dimensions), priority)
            .await;

        handle.await
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

impl ImageSource<'_> {
    /// Whether we should try to thumbnail this source for the given requested
    /// dimensions.
    fn should_thumbnail(
        &self,
        prefer_thumbnail: bool,
        thumbnail_dimensions: FrameDimensions,
    ) -> bool {
        if !self.can_be_thumbnailed() {
            return false;
        }

        let dimensions = self.dimensions();

        if prefer_thumbnail && dimensions.is_none() {
            return true;
        }

        dimensions.is_some_and(|d| d.needs_thumbnail(thumbnail_dimensions))
            || filesize_is_too_big(self.filesize())
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

    /// The filesize of this source.
    fn filesize(&self) -> Option<u32> {
        self.info.and_then(|i| i.filesize)
    }

    /// The dimensions of this source.
    fn dimensions(&self) -> Option<FrameDimensions> {
        self.info.and_then(|i| i.dimensions)
    }
}

/// Whether the given filesize is considered too big to be the preferred source
/// to download.
fn filesize_is_too_big(filesize: Option<u32>) -> bool {
    filesize.is_some_and(|s| s > THUMBNAIL_MAX_FILESIZE_THRESHOLD)
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

impl MediaSource<'_> {
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
    /// The dimensions of the image.
    dimensions: Option<FrameDimensions>,
    /// The MIME type of the image.
    mimetype: Option<&'a str>,
    /// The file size of the image.
    filesize: Option<u32>,
}

impl<'a> From<&'a ImageInfo> for ImageSourceInfo<'a> {
    fn from(value: &'a ImageInfo) -> Self {
        Self {
            dimensions: FrameDimensions::from_options(value.width, value.height),
            mimetype: value.mimetype.as_deref(),
            filesize: value.size.and_then(|u| u.try_into().ok()),
        }
    }
}

impl<'a> From<&'a ThumbnailInfo> for ImageSourceInfo<'a> {
    fn from(value: &'a ThumbnailInfo) -> Self {
        Self {
            dimensions: FrameDimensions::from_options(value.width, value.height),
            mimetype: value.mimetype.as_deref(),
            filesize: value.size.and_then(|u| u.try_into().ok()),
        }
    }
}

impl<'a> From<&'a AvatarImageInfo> for ImageSourceInfo<'a> {
    fn from(value: &'a AvatarImageInfo) -> Self {
        Self {
            dimensions: FrameDimensions::from_options(value.width, value.height),
            mimetype: value.mimetype.as_deref(),
            filesize: value.size.and_then(|u| u.try_into().ok()),
        }
    }
}

/// The settings for downloading a thumbnail.
#[derive(Debug, Clone)]
pub struct ThumbnailSettings {
    /// The requested dimensions of the thumbnail.
    pub dimensions: FrameDimensions,
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
            dimensions,
            method,
            animated,
            ..
        } = value;

        MediaThumbnailSettings {
            method,
            width: dimensions.width.into(),
            height: dimensions.height.into(),
            animated,
        }
    }
}

/// An error encountered when loading an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// Could not download the image.
    Download,
    /// Could not save the image to a temporary file.
    File,
    /// The image uses an unsupported format.
    UnsupportedFormat,
    /// An I/O error occurred when loading the image with glycin.
    Io,
    /// An unexpected error occurred.
    Unknown,
    /// The request for the image was aborted.
    Aborted,
}

impl Error for ImageError {}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Download => gettext("Could not retrieve media"),
            Self::UnsupportedFormat => gettext("Image format not supported"),
            Self::File | Self::Io | Self::Unknown | Self::Aborted => {
                gettext("An unexpected error occurred")
            }
        };

        f.write_str(&s)
    }
}

impl From<MediaFileError> for ImageError {
    fn from(value: MediaFileError) -> Self {
        match value {
            MediaFileError::Sdk(_) => Self::Download,
            MediaFileError::File(_) => Self::File,
        }
    }
}

impl From<glycin::ErrorCtx> for ImageError {
    fn from(value: glycin::ErrorCtx) -> Self {
        if value.unsupported_format().is_some() {
            Self::UnsupportedFormat
        } else if matches!(value.error(), glycin::Error::StdIoError { .. }) {
            Self::Io
        } else {
            Self::Unknown
        }
    }
}

//! Collection of methods for media files.

use std::{cell::Cell, str::FromStr, sync::Mutex};

use gettextrs::gettext;
use gtk::{gdk, gio, glib, prelude::*};
use image::{ColorType, DynamicImage, ImageDecoder, ImageResult};
use matrix_sdk::attachment::{
    BaseAudioInfo, BaseImageInfo, BaseThumbnailInfo, BaseVideoInfo, Thumbnail,
};
use mime::Mime;

use crate::{components::AnimatedImagePaintable, spawn_tokio, DISABLE_GLYCIN_SANDBOX};

/// The default width of a generated thumbnail.
const THUMBNAIL_DEFAULT_WIDTH: u32 = 800;
/// The default height of a generated thumbnail.
const THUMBNAIL_DEFAULT_HEIGHT: u32 = 600;
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

/// Get a default filename for a mime type.
///
/// Tries to guess the file extension, but it might not find it.
///
/// If the mime type is unknown, it uses the name for `fallback`. The fallback
/// mime types that are recognized are `mime::IMAGE`, `mime::VIDEO` and
/// `mime::AUDIO`, other values will behave the same as `None`.
pub fn filename_for_mime(mime_type: Option<&str>, fallback: Option<mime::Name>) -> String {
    let (type_, extension) =
        if let Some(mime) = mime_type.and_then(|m| m.parse::<mime::Mime>().ok()) {
            let extension =
                mime_guess::get_mime_extensions(&mime).map(|extensions| extensions[0].to_owned());

            (Some(mime.type_().as_str().to_owned()), extension)
        } else {
            (fallback.map(|type_| type_.as_str().to_owned()), None)
        };

    let name = match type_.as_deref() {
        // Translators: Default name for image files.
        Some("image") => gettext("image"),
        // Translators: Default name for video files.
        Some("video") => gettext("video"),
        // Translators: Default name for audio files.
        Some("audio") => gettext("audio"),
        // Translators: Default name for files.
        _ => gettext("file"),
    };

    extension
        .map(|extension| format!("{name}.{extension}"))
        .unwrap_or(name)
}

/// Information about a file
pub struct FileInfo {
    /// The mime type of the file.
    pub mime: Mime,
    /// The name of the file.
    pub filename: String,
    /// The size of the file in bytes.
    pub size: Option<u32>,
}

/// Load a file and return its content and some information
pub async fn load_file(file: &gio::File) -> Result<(Vec<u8>, FileInfo), glib::Error> {
    let attributes: &[&str] = &[
        gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
        gio::FILE_ATTRIBUTE_STANDARD_DISPLAY_NAME,
        gio::FILE_ATTRIBUTE_STANDARD_SIZE,
    ];

    // Read mime type.
    let info = file
        .query_info_future(
            &attributes.join(","),
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;

    let mime = info
        .content_type()
        .and_then(|content_type| Mime::from_str(&content_type).ok())
        .unwrap_or(mime::APPLICATION_OCTET_STREAM);

    let filename = info.display_name().to_string();

    let raw_size = info.size();
    let size = if raw_size >= 0 {
        Some(raw_size as u32)
    } else {
        None
    };

    let (data, _) = file.load_contents_future().await?;

    Ok((
        data.into(),
        FileInfo {
            mime,
            filename,
            size,
        },
    ))
}

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

        // Convert to RGB8/RGBA8 since it's the only format supported by webp.
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

        let encoder = webp::Encoder::from_image(&thumbnail).ok()?;
        let thumbnail_bytes = encoder.encode(WEBP_DEFAULT_QUALITY).to_vec();

        let thumbnail_content_type = mime::Mime::from_str(WEBP_CONTENT_TYPE)
            .expect("image should provide a valid content type");

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

async fn get_gstreamer_media_info(file: &gio::File) -> Option<gst_pbutils::DiscovererInfo> {
    let timeout = gst::ClockTime::from_seconds(15);
    let discoverer = gst_pbutils::Discoverer::new(timeout).ok()?;

    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = Mutex::new(Cell::new(Some(sender)));
    discoverer.connect_discovered(move |_, info, _| {
        if let Some(sender) = sender.lock().unwrap().take() {
            sender.send(info.clone()).unwrap();
        }
    });

    discoverer.start();
    discoverer.discover_uri_async(&file.uri()).ok()?;

    let media_info = receiver.await.unwrap();
    discoverer.stop();

    Some(media_info)
}

pub async fn get_video_info(file: &gio::File) -> BaseVideoInfo {
    let mut info = BaseVideoInfo {
        duration: None,
        width: None,
        height: None,
        size: None,
        blurhash: None,
    };

    let media_info = match get_gstreamer_media_info(file).await {
        Some(media_info) => media_info,
        None => return info,
    };

    info.duration = media_info.duration().map(Into::into);

    if let Some(stream_info) = media_info
        .video_streams()
        .first()
        .and_then(|s| s.downcast_ref::<gst_pbutils::DiscovererVideoInfo>())
    {
        info.width = Some(stream_info.width().into());
        info.height = Some(stream_info.height().into());
    }

    info
}

pub async fn get_audio_info(file: &gio::File) -> BaseAudioInfo {
    let mut info = BaseAudioInfo {
        duration: None,
        size: None,
    };

    let media_info = match get_gstreamer_media_info(file).await {
        Some(media_info) => media_info,
        None => return info,
    };

    info.duration = media_info.duration().map(Into::into);
    info
}

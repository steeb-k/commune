//! Image decoding with the pure-Rust [`image`] crate.
//!
//! This is the backend used wherever glycin is not available. glycin isolates
//! each format's decoder in a sandboxed subprocess; we cannot reproduce that
//! here, so the next best thing is a decoder with no unsafe C parsers in it at
//! all. That is what `image` gives us, and it also has the frame-by-frame
//! animation API this module needs.
//!
//! Decoding never happens on the main thread. A still image is decoded on the
//! Tokio blocking pool. An animated image gets a dedicated thread that owns its
//! decoder and hands out one frame per request, so a long animation is streamed
//! rather than held in memory — the same shape as glycin's subprocess, minus
//! the process boundary. The thread exits when the last [`Image`] handle is
//! dropped.
//!
//! # Supported formats
//!
//! BMP, GIF (animated), ICO, JPEG, PNG (including APNG), TIFF and WebP
//! (animated). **SVG, HEIC, AVIF and JXL are not supported** and report
//! [`Error::UnknownFormat`], which the UI surfaces as "Image format not
//! supported".

use std::{
    collections::VecDeque,
    fmt,
    io::Cursor,
    sync::{Arc, OnceLock, mpsc},
    thread,
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};
use image::{
    AnimationDecoder, DynamicImage, Frames, ImageDecoder, ImageError as ImageCrateError,
    ImageFormat, ImageReader, RgbaImage, metadata::Orientation,
};
use tokio::sync::oneshot;
use tracing::error;

use crate::RUNTIME;

/// The delay to use for an animation frame that does not declare a usable one.
///
/// GIFs in particular often declare a delay of 0, which every renderer is
/// expected to treat as "too fast, slow it down".
const DEFAULT_FRAME_DELAY: Duration = Duration::from_millis(100);

/// The source of the encoded image data.
enum Source {
    /// The bytes containing the encoded image.
    Bytes(glib::Bytes),
    /// The file containing the encoded image.
    File(gio::File),
}

/// A loader for a single image.
pub(crate) struct Loader(Source);

impl Loader {
    /// Construct a loader for the given encoded bytes.
    pub(crate) fn for_bytes(bytes: &glib::Bytes) -> Self {
        Self(Source::Bytes(bytes.clone()))
    }

    /// Construct a loader for the given file.
    pub(crate) fn for_file(file: &gio::File) -> Self {
        Self(Source::File(file.clone()))
    }

    /// Load the image, decoding enough of it to know its dimensions.
    pub(crate) async fn load(self) -> Result<Image, Error> {
        let data: Arc<[u8]> = match self.0 {
            Source::Bytes(bytes) => Arc::from(&*bytes),
            Source::File(file) => {
                let (contents, _etag) = file
                    .load_contents_future()
                    .await
                    .map_err(|error| Error::Read(error.to_string()))?;

                Arc::from(&*contents)
            }
        };

        RUNTIME
            .spawn_blocking(move || probe(data))
            .await
            .expect("task was not aborted")
    }
}

/// Read the header of the image to work out its format, orientation and
/// natural dimensions, and start streaming its frames if it is animated.
fn probe(data: Arc<[u8]>) -> Result<Image, Error> {
    let reader = ImageReader::new(Cursor::new(data.clone()))
        .with_guessed_format()
        .map_err(|error| Error::Read(error.to_string()))?;
    let format = reader.format().ok_or(Error::UnknownFormat)?;

    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let (width, height) = decoder.dimensions();
    drop(decoder);

    // The orientation is applied to every frame we decode, so the natural
    // dimensions of the image are the ones it has after rotation.
    let (width, height) = if orientation_swaps_axes(orientation) {
        (height, width)
    } else {
        (width, height)
    };

    let animation = animation_capable(&data, format)?
        .then(|| Animation::spawn(data.clone(), format, orientation));

    Ok(Image {
        inner: Arc::new(ImageInner {
            data,
            format,
            orientation,
            width,
            height,
            scale: OnceLock::new(),
            animation,
        }),
    })
}

/// Whether the given orientation exchanges the width and the height of the
/// image.
fn orientation_swaps_axes(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// Whether the image could carry an animation, and so has to have its frames
/// streamed.
///
/// A GIF always answers `true`: the container has no cheap frame count, so
/// whether it really is animated is only settled once the streaming decoder has
/// looked past the first frame.
fn animation_capable(data: &Arc<[u8]>, format: ImageFormat) -> Result<bool, Error> {
    match format {
        ImageFormat::Gif => Ok(true),
        ImageFormat::WebP => {
            Ok(image::codecs::webp::WebPDecoder::new(Cursor::new(data.clone()))?.has_animation())
        }
        ImageFormat::Png => {
            Ok(image::codecs::png::PngDecoder::new(Cursor::new(data.clone()))?.is_apng()?)
        }
        _ => Ok(false),
    }
}

/// A loaded image, which produces frames on demand.
#[derive(Clone)]
pub(crate) struct Image {
    inner: Arc<ImageInner>,
}

/// The shared state of a loaded image.
struct ImageInner {
    /// The encoded image.
    data: Arc<[u8]>,
    /// The format of the encoded image.
    format: ImageFormat,
    /// The orientation to apply to every decoded frame.
    orientation: Orientation,
    /// The natural width of the image, after orientation.
    width: u32,
    /// The natural height of the image, after orientation.
    height: u32,
    /// The dimensions every frame is scaled down to, once one has been asked
    /// for.
    ///
    /// A caller that requests a scaled first frame wants the whole animation at
    /// that size: the paintable takes its intrinsic size from whichever frame
    /// it currently holds, so handing it a thumbnail followed by full-size
    /// frames would resize the widget one frame in.
    scale: OnceLock<(u32, u32)>,
    /// The thread streaming the frames, if the image could be animated.
    animation: Option<Animation>,
}

impl Image {
    /// The natural width of the image, in pixels.
    pub(crate) fn width(&self) -> u32 {
        self.inner.width
    }

    /// The natural height of the image, in pixels.
    pub(crate) fn height(&self) -> u32 {
        self.inner.height
    }

    /// Decode the next frame of the image.
    ///
    /// For an animated image this cycles through the frames, looping back to
    /// the first one after the last.
    pub(crate) async fn next_frame(&self) -> Result<Frame, Error> {
        let scale = self.inner.scale.get().copied();

        let raw = if let Some(animation) = &self.inner.animation {
            let raw = animation.next_frame().await?;
            scale_frame(raw, scale).await?
        } else {
            self.decode_first_frame(scale).await?
        };

        Ok(Frame::new(raw))
    }

    /// Decode the frame matching the given request.
    ///
    /// This is the first frame of the image, scaled down if the request asks
    /// for less than its natural size.
    ///
    /// An animated image has to take that frame from the streaming decoder
    /// rather than decoding it separately, because only the streaming decoder
    /// knows the frame's delay — and the caller uses the delay of this frame to
    /// decide whether the image is animated at all.
    pub(crate) async fn specific_frame(&self, request: &FrameRequest) -> Result<Frame, Error> {
        if let Some(scale) = request.scale {
            // Remember it, so the rest of an animation comes back at the same
            // size as the frame we are about to hand out.
            let _ = self.inner.scale.set(scale);
        }

        let raw = if let Some(animation) = &self.inner.animation {
            let raw = animation.next_frame().await?;
            scale_frame(raw, request.scale).await?
        } else {
            self.decode_first_frame(request.scale).await?
        };

        Ok(Frame::new(raw))
    }

    /// Decode the first frame of the image on the blocking pool, scaled down to
    /// the given dimensions if it is bigger than them.
    async fn decode_first_frame(&self, scale: Option<(u32, u32)>) -> Result<RawFrame, Error> {
        let inner = self.inner.clone();

        RUNTIME
            .spawn_blocking(move || decode_first_frame(&inner, scale))
            .await
            .expect("task was not aborted")
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image").finish_non_exhaustive()
    }
}

/// Scale an already decoded frame down to the given dimensions, if it is
/// bigger than them and a scale was asked for.
///
/// The delay is carried over untouched: scaling must not turn an animation into
/// a still image.
async fn scale_frame(raw: RawFrame, scale: Option<(u32, u32)>) -> Result<RawFrame, Error> {
    let Some((width, height)) = scale else {
        return Ok(raw);
    };

    if width >= raw.width && height >= raw.height {
        return Ok(raw);
    }

    RUNTIME
        .spawn_blocking(move || {
            let delay = raw.delay;

            let Some(buffer) = RgbaImage::from_raw(raw.width, raw.height, raw.data) else {
                return Err(Error::Decode(
                    "the decoded frame does not match its dimensions".to_owned(),
                ));
            };

            Ok(RawFrame::new(
                DynamicImage::ImageRgba8(buffer).thumbnail(width, height),
                delay,
            ))
        })
        .await
        .expect("task was not aborted")
}

/// Decode the first frame of the given image, scaled down to the given
/// dimensions if it is bigger than them.
fn decode_first_frame(inner: &ImageInner, scale: Option<(u32, u32)>) -> Result<RawFrame, Error> {
    let decoder =
        ImageReader::with_format(Cursor::new(inner.data.clone()), inner.format).into_decoder()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(inner.orientation);

    if let Some((width, height)) = scale
        && (width < image.width() || height < image.height())
    {
        image = image.thumbnail(width, height);
    }

    Ok(RawFrame::new(image, None))
}

/// A handle to the thread streaming the frames of an animated image.
///
/// Dropping the last handle closes the request channel, which ends the thread.
struct Animation {
    /// The channel carrying a reply slot for each frame we ask for.
    requests: mpsc::Sender<oneshot::Sender<Result<RawFrame, Error>>>,
}

impl Animation {
    /// Start a thread streaming the frames of the given image.
    fn spawn(data: Arc<[u8]>, format: ImageFormat, orientation: Orientation) -> Self {
        let (requests, receiver) = mpsc::channel();

        // If the thread cannot be started, the channel's sender is the only
        // thing left holding it open; dropping the receiver here makes every
        // later request fail with `Error::Decode` rather than hang.
        if let Err(error) = thread::Builder::new()
            .name("image-animation".to_owned())
            .spawn(move || stream_frames(&data, format, orientation, &receiver))
        {
            error!("Could not start the image animation thread: {error}");
        }

        Self { requests }
    }

    /// Ask the thread for the next frame of the animation.
    async fn next_frame(&self) -> Result<RawFrame, Error> {
        let (sender, receiver) = oneshot::channel();

        self.requests
            .send(sender)
            .map_err(|_| Error::Decode("the image animation thread stopped".to_owned()))?;

        receiver
            .await
            .map_err(|_| Error::Decode("the image animation thread stopped".to_owned()))?
    }
}

/// Answer frame requests until the image is dropped.
fn stream_frames(
    data: &Arc<[u8]>,
    format: ImageFormat,
    orientation: Orientation,
    requests: &mpsc::Receiver<oneshot::Sender<Result<RawFrame, Error>>>,
) {
    let mut state = match Animator::new(data, format, orientation) {
        Ok(state) => state,
        Err(error) => {
            // Keep answering, so a caller gets the error rather than a closed
            // channel it would have to translate itself.
            while let Ok(reply) = requests.recv() {
                let _ = reply.send(Err(error.clone()));
            }

            return;
        }
    };

    while let Ok(reply) = requests.recv() {
        let _ = reply.send(state.next(data, format, orientation));
    }
}

/// The state of an animation being streamed.
enum Animator {
    /// The image declared an animation but only has one frame, so it is really
    /// a still image.
    Still(RawFrame),
    /// The image is animated.
    Animated {
        /// The remaining frames of the current pass.
        frames: Frames<'static>,
        /// Frames already decoded but not handed out yet.
        buffered: VecDeque<RawFrame>,
    },
}

impl Animator {
    /// Start decoding the given image, looking one frame ahead so that a
    /// single-frame image is recognised before anything is shown.
    fn new(data: &Arc<[u8]>, format: ImageFormat, orientation: Orientation) -> Result<Self, Error> {
        let mut frames = build_frames(data, format)?;

        let Some(first) = frames.next().transpose()? else {
            return Err(Error::Decode("the image has no frames".to_owned()));
        };
        let Some(second) = frames.next().transpose()? else {
            // One frame only. Report no delay, so the caller treats it as a
            // still image, matching what glycin does.
            return Ok(Self::Still(RawFrame::new(
                oriented(first, orientation),
                None,
            )));
        };

        Ok(Self::Animated {
            frames,
            buffered: [
                animation_frame(first, orientation),
                animation_frame(second, orientation),
            ]
            .into(),
        })
    }

    /// The next frame to show, rebuilding the decoder to loop once the last
    /// frame has been handed out.
    fn next(
        &mut self,
        data: &Arc<[u8]>,
        format: ImageFormat,
        orientation: Orientation,
    ) -> Result<RawFrame, Error> {
        let (frames, buffered) = match self {
            // A still image hands out the same frame however often it is asked
            // for it, which is what a caller that did not check the delay
            // expects.
            Self::Still(frame) => return Ok(frame.clone()),
            Self::Animated { frames, buffered } => (frames, buffered),
        };

        if let Some(frame) = buffered.pop_front() {
            return Ok(frame);
        }

        if let Some(frame) = frames.next().transpose()? {
            return Ok(animation_frame(frame, orientation));
        }

        // The animation ended, so start it over. glycin loops, and
        // `AnimatedImagePaintable` relies on that.
        *frames = build_frames(data, format)?;

        let Some(frame) = frames.next().transpose()? else {
            return Err(Error::Decode("the image has no frames".to_owned()));
        };

        Ok(animation_frame(frame, orientation))
    }
}

/// Construct a frame iterator over the given animated image.
fn build_frames(data: &Arc<[u8]>, format: ImageFormat) -> Result<Frames<'static>, Error> {
    let reader = Cursor::new(data.clone());

    let frames = match format {
        ImageFormat::Gif => image::codecs::gif::GifDecoder::new(reader)?.into_frames(),
        ImageFormat::WebP => image::codecs::webp::WebPDecoder::new(reader)?.into_frames(),
        ImageFormat::Png => image::codecs::png::PngDecoder::new(reader)?
            .apng()?
            .into_frames(),
        _ => return Err(Error::UnknownFormat),
    };

    Ok(frames)
}

/// Apply the given orientation to the given animation frame.
fn oriented(frame: image::Frame, orientation: Orientation) -> DynamicImage {
    let mut image = DynamicImage::ImageRgba8(frame.into_buffer());
    image.apply_orientation(orientation);
    image
}

/// Convert the given animation frame into a raw frame, keeping its delay.
fn animation_frame(frame: image::Frame, orientation: Orientation) -> RawFrame {
    let (numerator, denominator) = frame.delay().numer_denom_ms();

    let delay = if numerator == 0 || denominator == 0 {
        DEFAULT_FRAME_DELAY
    } else {
        Duration::from_micros(u64::from(numerator) * 1_000 / u64::from(denominator))
    };

    RawFrame::new(oriented(frame, orientation), Some(delay))
}

/// A decoded frame, before it is turned into a texture.
///
/// This is what crosses a thread boundary: a [`gdk::Texture`] cannot, so it is
/// only built once the pixels are back on the main thread.
#[derive(Clone)]
struct RawFrame {
    /// The pixels of the frame, as RGBA8.
    data: Vec<u8>,
    /// The width of the frame, in pixels.
    width: u32,
    /// The height of the frame, in pixels.
    height: u32,
    /// How long to show the frame for, if the image is animated.
    delay: Option<Duration>,
}

impl RawFrame {
    /// Construct a raw frame from the given decoded image.
    fn new(image: DynamicImage, delay: Option<Duration>) -> Self {
        let buffer = image.into_rgba8();
        let (width, height) = buffer.dimensions();

        Self {
            data: buffer.into_raw(),
            width,
            height,
            delay,
        }
    }
}

/// A decoded frame of an image.
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    /// The content of the frame.
    texture: gdk::Texture,
    /// How long to show the frame for, if the image is animated.
    delay: Option<Duration>,
}

impl Frame {
    /// Construct a frame from the given decoded pixels.
    fn new(raw: RawFrame) -> Self {
        let stride = usize::try_from(raw.width).unwrap_or_default() * 4;

        let texture = gdk::MemoryTexture::new(
            i32::try_from(raw.width).unwrap_or(i32::MAX),
            i32::try_from(raw.height).unwrap_or(i32::MAX),
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(raw.data),
            stride,
        );

        Self {
            texture: texture.upcast(),
            delay: raw.delay,
        }
    }

    /// The width of the frame, in pixels.
    pub(crate) fn width(&self) -> u32 {
        self.texture.width().try_into().unwrap_or_default()
    }

    /// The height of the frame, in pixels.
    pub(crate) fn height(&self) -> u32 {
        self.texture.height().try_into().unwrap_or_default()
    }

    /// How long to show this frame for, if the image is animated.
    ///
    /// Returns `None` for a still image.
    pub(crate) fn delay(&self) -> Option<Duration> {
        self.delay
    }

    /// The content of the frame, as a texture.
    pub(crate) fn texture(&self) -> gdk::Texture {
        self.texture.clone()
    }
}

/// A request for a frame at particular dimensions.
pub(crate) struct FrameRequest {
    /// The dimensions to scale the frame down to, if any.
    scale: Option<(u32, u32)>,
}

impl FrameRequest {
    /// Construct a request for a frame at its natural dimensions.
    pub(crate) fn new() -> Self {
        Self { scale: None }
    }

    /// Scale the requested frame to the given dimensions.
    pub(crate) fn with_scale(mut self, width: u32, height: u32) -> Self {
        self.scale = Some((width, height));
        self
    }
}

/// An error encountered while decoding an image.
#[derive(Debug, Clone)]
pub(crate) enum Error {
    /// The image format was not recognised, or we have no decoder for it.
    UnknownFormat,
    /// The image data could not be read.
    Read(String),
    /// The image could not be decoded.
    Decode(String),
}

impl Error {
    /// Whether the failure was that the image format was not recognised.
    pub(crate) fn is_unknown_format(&self) -> bool {
        matches!(self, Self::UnknownFormat)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat => f.write_str("unsupported image format"),
            Self::Read(error) | Self::Decode(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for Error {}

impl From<ImageCrateError> for Error {
    fn from(value: ImageCrateError) -> Self {
        match value {
            ImageCrateError::Unsupported(_) => Self::UnknownFormat,
            ImageCrateError::IoError(error) => Self::Read(error.to_string()),
            error => Self::Decode(error.to_string()),
        }
    }
}

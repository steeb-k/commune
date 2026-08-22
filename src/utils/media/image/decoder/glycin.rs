//! Image decoding with [glycin], used on Linux.
//!
//! glycin runs a format-specific loader in a sandboxed subprocess and hands
//! back decoded frames, so this backend is a thin wrapper: every type here
//! holds the corresponding glycin type and forwards to it. The only behaviour
//! that is not a straight forward is [`Frame::delay`], which turns glycin's
//! "0 µs means the image is not animated" convention into an [`Option`].
//!
//! [glycin]: https://gitlab.gnome.org/GNOME/glycin

use std::{fmt, time::Duration};

use gtk::{gdk, gio, glib};

use crate::DISABLE_GLYCIN_SANDBOX;

/// A loader for a single image.
pub(crate) struct Loader(::glycin::Loader);

impl Loader {
    /// Construct a loader for the given encoded bytes.
    pub(crate) fn for_bytes(bytes: &glib::Bytes) -> Self {
        Self::with_loader(::glycin::Loader::for_bytes(bytes))
    }

    /// Construct a loader for the given file.
    pub(crate) fn for_file(file: &gio::File) -> Self {
        Self::with_loader(::glycin::Loader::new(file))
    }

    /// Construct a loader from the given glycin loader, applying our sandbox
    /// preference.
    fn with_loader(loader: ::glycin::Loader) -> Self {
        if DISABLE_GLYCIN_SANDBOX {
            loader.set_sandbox_selector(::glycin::SandboxSelector::NotSandboxed);
        }

        Self(loader)
    }

    /// Load the image, decoding enough of it to know its dimensions.
    pub(crate) async fn load(self) -> Result<Image, Error> {
        self.0.load_future().await.map(Image).map_err(Error)
    }
}

/// A loaded image, which produces frames on demand.
#[derive(Clone)]
pub(crate) struct Image(::glycin::Image);

impl Image {
    /// The natural width of the image, in pixels.
    pub(crate) fn width(&self) -> u32 {
        self.0.width()
    }

    /// The natural height of the image, in pixels.
    pub(crate) fn height(&self) -> u32 {
        self.0.height()
    }

    /// Decode the next frame of the image.
    ///
    /// For an animated image this cycles through the frames, looping back to
    /// the first one after the last.
    pub(crate) async fn next_frame(&self) -> Result<Frame, Error> {
        self.0.next_frame_future().await.map(Frame).map_err(Error)
    }

    /// Decode the frame matching the given request.
    pub(crate) async fn specific_frame(&self, request: &FrameRequest) -> Result<Frame, Error> {
        self.0
            .specific_frame_future(&request.0)
            .await
            .map(Frame)
            .map_err(Error)
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image").finish_non_exhaustive()
    }
}

/// A decoded frame of an image.
#[derive(Clone, Debug)]
pub(crate) struct Frame(::glycin::Frame);

impl Frame {
    /// The width of the frame, in pixels.
    pub(crate) fn width(&self) -> u32 {
        self.0.width()
    }

    /// The height of the frame, in pixels.
    pub(crate) fn height(&self) -> u32 {
        self.0.height()
    }

    /// How long to show this frame for, if the image is animated.
    ///
    /// Returns `None` for a still image.
    pub(crate) fn delay(&self) -> Option<Duration> {
        // glycin always computes a suitable delay if the image is animated but its
        // delay is set to 0, so 0 should mean that the image is not animated.
        let delay = self.0.delay();

        (delay > 0)
            .then(|| u64::try_from(delay).ok())
            .flatten()
            .map(Duration::from_micros)
    }

    /// The content of the frame, as a texture.
    pub(crate) fn texture(&self) -> gdk::Texture {
        glycin_gtk4::frame_get_texture(&self.0)
    }
}

/// A request for a frame at particular dimensions.
pub(crate) struct FrameRequest(::glycin::FrameRequest);

impl FrameRequest {
    /// Construct a request for a frame at its natural dimensions.
    pub(crate) fn new() -> Self {
        Self(::glycin::FrameRequest::new())
    }

    /// Scale the requested frame to the given dimensions.
    pub(crate) fn with_scale(self, width: u32, height: u32) -> Self {
        self.0.set_scale(width, height);
        self
    }
}

/// An error encountered while decoding an image.
#[derive(Debug)]
pub(crate) struct Error(glib::Error);

impl Error {
    /// Whether the failure was that the image format was not recognised.
    pub(crate) fn is_unknown_format(&self) -> bool {
        matches!(
            self.0.kind::<::glycin::LoaderError>(),
            Some(::glycin::LoaderError::UnknownImageFormat)
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Error {}

mod animated_image_paintable;
mod audio_player;
mod content_viewer;
// GTK only has a media backend of its own where it was built against
// GStreamer. The conda-forge build we use on macOS was not, and neither is
// pixiewood's Android build: its cross file sets `media-gstreamer =
// 'disabled'`.
#[cfg(any(target_os = "macos", target_os = "android"))]
mod gst_media_stream;
// The map viewer is libshumate, which is not cross-built for Android; see
// `doc/android.md`.
#[cfg(not(target_os = "android"))]
mod location_viewer;
mod video_player;
mod video_player_renderer;

#[cfg(not(target_os = "android"))]
pub(crate) use self::location_viewer::LocationViewer;
pub(crate) use self::{
    animated_image_paintable::AnimatedImagePaintable,
    audio_player::*,
    content_viewer::{ContentType, MediaContentViewer},
    video_player::VideoPlayer,
};

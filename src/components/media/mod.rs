mod animated_image_paintable;
mod audio_player;
mod content_viewer;
// GTK only has a media backend of its own where it was built against
// GStreamer. None of the builds we use off Linux were: the conda-forge one
// on macOS, the MSYS2 one on Windows, and pixiewood's on Android, whose
// cross file sets `media-gstreamer = 'disabled'`. Where it is missing, we
// play media ourselves.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "android"))]
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

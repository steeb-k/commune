mod animated_image_paintable;
mod audio_player;
mod content_viewer;
// GTK only has a media backend of its own where it was built against
// GStreamer, which the conda-forge build we use on macOS was not.
#[cfg(target_os = "macos")]
mod gst_media_stream;
// The map viewer is libshumate and the video player is GStreamer. Neither is
// cross-built for Android, so both are absent there; see `doc/android.md`.
#[cfg(not(target_os = "android"))]
mod location_viewer;
#[cfg(not(target_os = "android"))]
mod video_player;
#[cfg(not(target_os = "android"))]
mod video_player_renderer;

pub(crate) use self::{
    animated_image_paintable::AnimatedImagePaintable,
    audio_player::*,
    content_viewer::{ContentType, MediaContentViewer},
};
#[cfg(not(target_os = "android"))]
pub(crate) use self::{location_viewer::LocationViewer, video_player::VideoPlayer};

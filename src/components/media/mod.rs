mod animated_image_paintable;
mod audio_player;
mod content_viewer;
// GTK only has a media backend of its own where it was built against
// GStreamer, which neither the conda-forge build we use on macOS nor the MSYS2
// one we use on Windows was. Where it is missing, we play media ourselves.
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod gst_media_stream;
mod location_viewer;
mod video_player;
mod video_player_renderer;

pub(crate) use self::{
    animated_image_paintable::AnimatedImagePaintable,
    audio_player::*,
    content_viewer::{ContentType, MediaContentViewer},
    location_viewer::LocationViewer,
    video_player::VideoPlayer,
};

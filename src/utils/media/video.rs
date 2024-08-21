//! Collection of methods for videos.

use std::sync::{Arc, Mutex};

use futures_channel::oneshot;
use gst::prelude::*;
use gst_video::prelude::*;
use gtk::{gio, glib, glib::clone, prelude::*};
use image::GenericImageView;
use matrix_sdk::attachment::{BaseVideoInfo, Thumbnail};
use tracing::warn;

use super::{
    image::{prepare_thumbnail_for_sending, ImageDimensions},
    load_gstreamer_media_info,
};

/// A channel sender to send the result of a video thumbnail.
type ThumbnailResultSender = oneshot::Sender<Result<Thumbnail, ()>>;

/// Load information and try to generate a thumbnail for the video in the given
/// file.
pub async fn load_video_info(file: &gio::File) -> (BaseVideoInfo, Option<Thumbnail>) {
    let mut info = BaseVideoInfo {
        duration: None,
        width: None,
        height: None,
        size: None,
        blurhash: None,
    };

    let Some(media_info) = load_gstreamer_media_info(file).await else {
        return (info, None);
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

    let thumbnail = generate_video_thumbnail(file).await;

    (info, thumbnail)
}

/// Generate a thumbnail for the video in the given file.
async fn generate_video_thumbnail(file: &gio::File) -> Option<Thumbnail> {
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));

    let pipeline = match create_thumbnailer_pipeline(&file.uri(), sender.clone()) {
        Ok(pipeline) => pipeline,
        Err(error) => {
            warn!("Could not create pipeline for video thumbnail: {error}");
            return None;
        }
    };

    if pipeline.set_state(gst::State::Paused).is_err() {
        warn!("Could not initialize pipeline for video thumbnail");
        return None;
    }

    let bus = pipeline.bus().expect("Pipeline has a bus");

    let mut started = false;
    let _bus_guard = bus
        .add_watch(clone!(
            #[weak]
            pipeline,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move |_, message| {
                match message.view() {
                    gst::MessageView::AsyncDone(_) => {
                        if !started {
                            // AsyncDone means that the pipeline has started now.
                            if pipeline.set_state(gst::State::Playing).is_err() {
                                warn!("Could not start pipeline for video thumbnail");
                                send_video_thumbnail_result(&sender, Err(()));

                                return glib::ControlFlow::Break;
                            };

                            started = true;
                        }

                        glib::ControlFlow::Continue
                    }
                    gst::MessageView::Eos(_) => {
                        // We have the thumbnail or we cannot have one.
                        glib::ControlFlow::Break
                    }
                    gst::MessageView::Error(error) => {
                        warn!("Could not generate video thumbnail: {error}");
                        send_video_thumbnail_result(&sender, Err(()));

                        glib::ControlFlow::Break
                    }
                    _ => glib::ControlFlow::Continue,
                }
            }
        ))
        .expect("Setting bus watch succeeds");

    let thumbnail = receiver.await;

    // Clean up.
    let _ = pipeline.set_state(gst::State::Null);
    bus.set_flushing(true);

    thumbnail.ok().transpose().ok().flatten()
}

/// Create a GStreamer pipeline to get a thumbnail of the first frame.
fn create_thumbnailer_pipeline(
    uri: &str,
    sender: Arc<Mutex<Option<ThumbnailResultSender>>>,
) -> Result<gst::Pipeline, glib::Error> {
    // Create our pipeline from a pipeline description string.
    let pipeline = gst::parse::launch(&format!(
        "uridecodebin uri={uri} ! videoconvert ! appsink name=sink"
    ))?
    .downcast::<gst::Pipeline>()
    .expect("Element is a pipeline");

    let appsink = pipeline
        .by_name("sink")
        .expect("Sink element is in the pipeline")
        .downcast::<gst_app::AppSink>()
        .expect("Sink element is an appsink");

    // Don't synchronize on the clock, we only want a snapshot asap.
    appsink.set_property("sync", false);

    // Tell the appsink what format we want, for simplicity we only accept 8-bit
    // RGB.
    appsink.set_caps(Some(
        &gst_video::VideoCapsBuilder::new()
            .format(gst_video::VideoFormat::Rgbx)
            .build(),
    ));

    let mut got_snapshot = false;

    // Listen to callbacks to get the data.
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                // Pull the sample out of the buffer.
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let Some(buffer) = sample.buffer() else {
                    warn!("Could not get buffer from appsink");
                    send_video_thumbnail_result(&sender, Err(()));

                    return Err(gst::FlowError::Error);
                };

                // Make sure that we only get a single buffer.
                if got_snapshot {
                    return Err(gst::FlowError::Eos);
                }
                got_snapshot = true;

                let Some(caps) = sample.caps() else {
                    warn!("Got video sample without caps");
                    send_video_thumbnail_result(&sender, Err(()));

                    return Err(gst::FlowError::Error);
                };
                let Ok(info) = gst_video::VideoInfo::from_caps(caps) else {
                    warn!("Could not parse video caps");
                    send_video_thumbnail_result(&sender, Err(()));

                    return Err(gst::FlowError::Error);
                };

                let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
                    .map_err(|_| {
                        warn!("Could not map video buffer readable");
                        send_video_thumbnail_result(&sender, Err(()));

                        gst::FlowError::Error
                    })?;

                // Create a FlatSamples around the borrowed video frame data from GStreamer with
                // the correct stride.
                let img = image::FlatSamples::<&[u8]> {
                    samples: frame.plane_data(0).unwrap(),
                    layout: image::flat::SampleLayout {
                        channels: 3,       // RGB
                        channel_stride: 1, // 1 byte from component to component
                        width: frame.width(),
                        width_stride: 4, // 4 bytes from pixel to pixel
                        height: frame.height(),
                        height_stride: frame.plane_stride()[0] as usize, // stride from line to line
                    },
                    color_hint: Some(image::ColorType::Rgb8),
                };

                let Ok(view) = img.as_view::<image::Rgb<u8>>() else {
                    warn!("Could not parse frame as view");
                    send_video_thumbnail_result(&sender, Err(()));

                    return Err(gst::FlowError::Error);
                };

                // Reduce the dimensions if the thumbnail is bigger than the wanted size.
                let dimensions = ImageDimensions {
                    width: frame.width() * info.par().numer() as u32,
                    height: frame.height() * info.par().denom() as u32,
                };

                let thumbnail = if let Some(target_dimensions) = dimensions.resize_for_thumbnail() {
                    image::imageops::thumbnail(
                        &view,
                        target_dimensions.width,
                        target_dimensions.height,
                    )
                } else {
                    image::ImageBuffer::from_fn(view.width(), view.height(), |x, y| {
                        view.get_pixel(x, y)
                    })
                };

                // Prepare it.
                if let Some(thumbnail) = prepare_thumbnail_for_sending(thumbnail.into()) {
                    send_video_thumbnail_result(&sender, Ok(thumbnail));

                    Err(gst::FlowError::Eos)
                } else {
                    warn!("Failed to convert video thumbnail");
                    send_video_thumbnail_result(&sender, Err(()));

                    Err(gst::FlowError::Error)
                }
            })
            .build(),
    );

    Ok(pipeline)
}

/// Try to send the given video thumbnail result through the given sender.
fn send_video_thumbnail_result(
    sender: &Mutex<Option<ThumbnailResultSender>>,
    result: Result<Thumbnail, ()>,
) {
    let mut sender = match sender.lock() {
        Ok(sender) => sender,
        Err(error) => {
            warn!("Failed to lock video thumbnail mutex: {error}");
            return;
        }
    };

    if let Some(sender) = sender.take() {
        if sender.send(result).is_err() {
            warn!("Failed to send video thumbnail result through channel");
        }
    }
}

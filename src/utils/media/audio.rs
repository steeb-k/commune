//! Collection of methods for audio.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_channel::oneshot;
use gst::prelude::*;
use gtk::{gio, glib, prelude::*};
use matrix_sdk::attachment::BaseAudioInfo;
use tracing::warn;

use super::load_gstreamer_media_info;
use crate::utils::resample_slice;

/// Load information for the audio in the given file.
pub(crate) async fn load_audio_info(file: &gio::File) -> BaseAudioInfo {
    let mut info = BaseAudioInfo::default();

    let Some(media_info) = load_gstreamer_media_info(file).await else {
        return info;
    };

    info.duration = media_info.duration().map(Into::into);
    info
}

/// Generate a waveform for the given audio file.
///
/// The returned waveform should contain between 30 and 110 samples with a value
/// between 0 and 1.
pub(crate) async fn generate_waveform(
    file: &gio::File,
    duration: Option<Duration>,
) -> Option<Vec<f32>> {
    // According to MSC3246, we want at least 30 values and at most 120 values. It
    // should also allow us to have enough samples for drawing our waveform.
    let interval = duration
        .and_then(|duration| {
            // Try to get around 1 sample per second, except if the duration is too short or
            // too long.
            match duration.as_secs() {
                0..30 => duration.checked_div(30),
                30..110 => Some(Duration::from_secs(1)),
                _ => duration.checked_div(110),
            }
        })
        .unwrap_or_else(|| Duration::from_secs(1));

    // Create our pipeline from a pipeline description string.
    let pipeline = match gst::parse::launch(&format!(
        "uridecodebin uri={} ! audioconvert ! audio/x-raw,channels=1 ! level name=level interval={} ! fakesink qos=false sync=false",
        file.uri(),
        interval.as_nanos()
    )) {
        Ok(pipeline) => pipeline
            .downcast::<gst::Pipeline>()
            .expect("GstElement should be a GstPipeline"),
        Err(error) => {
            warn!("Could not create GstPipeline for audio waveform: {error}");
            return None;
        }
    };

    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let samples = Arc::new(Mutex::new(vec![]));
    let bus = pipeline.bus().expect("GstPipeline should have a GstBus");

    let samples_clone = samples.clone();
    let _bus_guard = bus
        .add_watch(move |_, message| {
            match message.view() {
                gst::MessageView::Eos(_) => {
                    // We are done collecting the samples.
                    send_empty_signal(&sender);
                    glib::ControlFlow::Break
                }
                gst::MessageView::Error(error) => {
                    warn!("Could not generate audio waveform: {error}");
                    send_empty_signal(&sender);
                    glib::ControlFlow::Break
                }
                gst::MessageView::Element(element) => {
                    if let Some(structure) = element.structure()
                        && structure.has_name("level")
                    {
                        let peaks_array = structure
                            .get::<&glib::ValueArray>("peak")
                            .expect("peak value should be a GValueArray");
                        let peak = peaks_array[0]
                            .get::<f64>()
                            .expect("GValueArray value should be a double");

                        match samples_clone.lock() {
                            Ok(mut samples) => {
                                let value_db = if peak.is_nan() { 0.0 } else { peak };
                                // Convert the decibels to a relative amplitude, to get a value
                                // between 0 and 1.
                                let value = 10.0_f64.powf(value_db / 20.0);

                                samples.push(value);
                            }
                            Err(error) => {
                                warn!("Failed to lock audio waveform samples mutex: {error}");
                            }
                        }
                    }
                    glib::ControlFlow::Continue
                }
                _ => glib::ControlFlow::Continue,
            }
        })
        .expect("Adding GstBus watch should succeed");

    match pipeline.set_state(gst::State::Playing) {
        Ok(_) => {
            let _ = receiver.await;
        }
        Err(error) => {
            warn!("Could not start GstPipeline for audio waveform: {error}");
        }
    }

    // Clean up pipeline.
    let _ = pipeline.set_state(gst::State::Null);
    bus.set_flushing(true);

    let waveform = match samples.lock() {
        Ok(mut samples) => std::mem::take(&mut *samples),
        Err(error) => {
            warn!("Failed to lock audio waveform samples mutex: {error}");
            return None;
        }
    };

    Some(normalize_waveform(waveform)).filter(|waveform| !waveform.is_empty())
}

/// Try to send an empty signal through the given sender.
fn send_empty_signal(sender: &Mutex<Option<oneshot::Sender<()>>>) {
    let mut sender = match sender.lock() {
        Ok(sender) => sender,
        Err(error) => {
            warn!("Failed to lock audio waveform signal mutex: {error}");
            return;
        }
    };

    if let Some(sender) = sender.take()
        && sender.send(()).is_err()
    {
        warn!("Failed to send audio waveform end through channel");
    }
}

/// Normalize the given waveform to have between 30 and 120 samples with a value
/// between 0 and 1.
///
/// All the samples in the waveform must be positive or negative. If they are
/// mixed, this will change the waveform because it uses the absolute value of
/// the sample.
///
/// If the waveform was empty, returns an empty vec.
pub(crate) fn normalize_waveform(waveform: Vec<f64>) -> Vec<f32> {
    if waveform.is_empty() {
        return vec![];
    }

    let max = waveform
        .iter()
        .copied()
        .map(f64::abs)
        .reduce(f64::max)
        .expect("iterator should contain at least one value");

    // Normalize between 0 and 1, with the highest value as 1.
    let mut normalized = waveform
        .into_iter()
        .map(f64::abs)
        .map(|value| if max == 0.0 { value } else { value / max } as f32)
        .collect::<Vec<_>>();

    match normalized.len() {
        0..30 => normalized = resample_slice(&normalized, 30).into_owned(),
        30..120 => {}
        _ => normalized = resample_slice(&normalized, 120).into_owned(),
    }

    normalized
}

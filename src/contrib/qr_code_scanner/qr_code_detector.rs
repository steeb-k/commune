use futures_channel::mpsc;
use gst_video::{prelude::*, video_frame::VideoFrameRef, VideoInfo};
use matrix_sdk::encryption::verification::{DecodingError, QrVerificationData};
use thiserror::Error;
use tracing::trace;

use super::*;
use crate::contrib::qr_code_scanner::camera::Action;

const HEADER: &[u8] = b"MATRIX";

mod imp {
    use std::sync::{LazyLock, Mutex};

    use gst::subclass::prelude::*;
    use gst_video::subclass::prelude::*;

    use super::*;

    #[derive(Default)]
    pub struct QrCodeDetector {
        info: Mutex<Option<VideoInfo>>,
        pub(super) sender: Mutex<Option<mpsc::Sender<Action>>>,
        code: Mutex<Option<QrVerificationData>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for QrCodeDetector {
        const NAME: &'static str = "QrCodeDetector";
        type Type = super::QrCodeDetector;
        type ParentType = gst_video::VideoSink;
    }

    impl ObjectImpl for QrCodeDetector {}
    impl GstObjectImpl for QrCodeDetector {}
    impl ElementImpl for QrCodeDetector {
        fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
            static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> =
                LazyLock::new(|| {
                    gst::subclass::ElementMetadata::new(
                        "Matrix Qr Code detector Sink",
                        "Sink/Video/QrCode/Matrix",
                        "A Qr code detector for Matrix",
                        "Julian Sparber <julian@sparber.net>",
                    )
                });

            Some(&*ELEMENT_METADATA)
        }

        fn pad_templates() -> &'static [gst::PadTemplate] {
            static PAD_TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
                let caps = gst_video::video_make_raw_caps(&[gst_video::VideoFormat::Gray8])
                    .any_features()
                    .build();

                vec![gst::PadTemplate::new(
                    "sink",
                    gst::PadDirection::Sink,
                    gst::PadPresence::Always,
                    &caps,
                )
                .unwrap()]
            });

            PAD_TEMPLATES.as_ref()
        }
    }

    impl BaseSinkImpl for QrCodeDetector {
        fn set_caps(&self, caps: &gst::Caps) -> Result<(), gst::LoggableError> {
            let video_info = gst_video::VideoInfo::from_caps(caps).unwrap();
            let mut info = self.info.lock().unwrap();
            info.replace(video_info);

            Ok(())
        }
    }

    impl VideoSinkImpl for QrCodeDetector {
        fn show_frame(&self, buffer: &gst::Buffer) -> Result<gst::FlowSuccess, gst::FlowError> {
            let now = std::time::Instant::now();

            if let Some(info) = &*self.info.lock().unwrap() {
                let frame = VideoFrameRef::from_buffer_ref_readable(buffer, info).unwrap();

                if let Ok(code) = decode_qr(&frame) {
                    let mut previous_code = self.code.lock().unwrap();
                    if previous_code.as_ref() != Some(&code) {
                        previous_code.replace(code.clone());
                        let mut sender = self.sender.lock().unwrap();
                        sender
                            .as_mut()
                            .unwrap()
                            .try_send(Action::QrCodeDetected(code))
                            .unwrap();
                    }
                }
            }
            trace!("Spent {}ms to detect qr code", now.elapsed().as_millis());

            Ok(gst::FlowSuccess::Ok)
        }
    }
}

glib::wrapper! {
    pub struct QrCodeDetector(ObjectSubclass<imp::QrCodeDetector>)
        @extends gst_video::VideoSink, gst_base::BaseSink, gst::Element, gst::Object;
}

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for QrCodeDetector {}
unsafe impl Sync for QrCodeDetector {}

impl QrCodeDetector {
    pub fn new(sender: mpsc::Sender<Action>) -> Self {
        let sink = glib::Object::new::<Self>();
        sink.imp().sender.lock().unwrap().replace(sender);
        sink
    }
}

// From https://github.com/matrix-org/matrix-rust-sdk/blob/79d13148fbba58db0ff5f62b27e7856cbbbe13c2/crates/matrix-sdk-qrcode/src/utils.rs#L81-L104
fn decode_qr(
    frame: &VideoFrameRef<&gst::BufferRef>,
) -> Result<QrVerificationData, QrDecodingError> {
    let data = frame.comp_data(0).expect("data is present");
    let width = frame
        .comp_stride(0)
        .try_into()
        .expect("stride is a positive integer");
    let height = frame.height() as usize;

    let mut image =
        rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| data[x + (y * width)]);
    let grids = image.detect_grids();

    let mut error = None;

    for grid in grids {
        let mut decoded = Vec::new();

        match grid.decode_to(&mut decoded) {
            Ok(_) => {
                if decoded.starts_with(HEADER) {
                    return QrVerificationData::from_bytes(decoded).map_err(Into::into);
                }
            }
            Err(e) => error = Some(e),
        }
    }

    Err(error.map_or_else(|| DecodingError::Header.into(), Into::into))
}

/// All possible errors when decoding a QR Code.
#[derive(Debug, Error)]
pub enum QrDecodingError {
    /// An error occurred when decoding the QR data.
    #[error(transparent)]
    Matrix(#[from] DecodingError),

    /// An error occurred when decoding the QR image.
    #[error(transparent)]
    Rqrr(#[from] rqrr::DeQRError),
}

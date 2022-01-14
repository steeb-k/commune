// SPDX-License-Identifier: GPL-3.0-or-later
//
// Fancy Camera with QR code detection
//
// Pipeline:
//                            queue -- videoconvert -- QrCodeDetector sink
//                         /
//     pipewiresrc -- tee
//                         \
//                            queue -- videoconvert -- our fancy sink

use glib::{clone, Receiver, Sender};
use gst::prelude::*;
use gtk::gdk::prelude::TextureExt;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use once_cell::sync::Lazy;

use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex};

use crate::contrib::qr_code_scanner::qr_code_detector::QrCodeDetector;
use crate::contrib::qr_code_scanner::QrVerificationDataBoxed;
use gtk::{gdk, graphene};
use matrix_sdk::encryption::verification::QrVerificationData;

pub enum Action {
    FrameChanged,
    QrCodeDetected(QrVerificationData),
}

mod camera_sink {
    use std::convert::AsRef;

    #[derive(Debug)]
    pub struct Frame(pub gst_video::VideoFrame<gst_video::video_frame::Readable>);

    impl AsRef<[u8]> for Frame {
        fn as_ref(&self) -> &[u8] {
            self.0.plane_data(0).unwrap()
        }
    }

    impl From<Frame> for gdk::Texture {
        fn from(f: Frame) -> gdk::Texture {
            let format = match f.0.format() {
                gst_video::VideoFormat::Bgra => gdk::MemoryFormat::B8g8r8a8,
                gst_video::VideoFormat::Argb => gdk::MemoryFormat::A8r8g8b8,
                gst_video::VideoFormat::Rgba => gdk::MemoryFormat::R8g8b8a8,
                gst_video::VideoFormat::Abgr => gdk::MemoryFormat::A8b8g8r8,
                gst_video::VideoFormat::Rgb => gdk::MemoryFormat::R8g8b8,
                gst_video::VideoFormat::Bgr => gdk::MemoryFormat::B8g8r8,
                _ => unreachable!(),
            };
            let width = f.0.width() as i32;
            let height = f.0.height() as i32;
            let rowstride = f.0.plane_stride()[0] as usize;

            gdk::MemoryTexture::new(
                width,
                height,
                format,
                &glib::Bytes::from_owned(f),
                rowstride,
            )
            .upcast()
        }
    }

    impl Frame {
        pub fn new(buffer: &gst::Buffer, info: &gst_video::VideoInfo) -> Self {
            let video_frame =
                gst_video::VideoFrame::from_buffer_readable(buffer.clone(), info).unwrap();
            Self(video_frame)
        }

        pub fn width(&self) -> u32 {
            self.0.width()
        }

        pub fn height(&self) -> u32 {
            self.0.height()
        }
    }

    use super::*;

    mod imp {
        use std::sync::Mutex;

        use gst::subclass::prelude::*;
        use gst_base::subclass::prelude::*;
        use gst_video::subclass::prelude::*;
        use once_cell::sync::Lazy;

        use super::*;

        #[derive(Default)]
        pub struct CameraSink {
            pub info: Mutex<Option<gst_video::VideoInfo>>,
            pub sender: Mutex<Option<Sender<Action>>>,
            pub pending_frame: Mutex<Option<Frame>>,
        }

        #[glib::object_subclass]
        impl ObjectSubclass for CameraSink {
            const NAME: &'static str = "CameraSink";
            type Type = super::CameraSink;
            type ParentType = gst_video::VideoSink;
        }

        impl ObjectImpl for CameraSink {}
        impl ElementImpl for CameraSink {
            fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
                static ELEMENT_METADATA: Lazy<gst::subclass::ElementMetadata> = Lazy::new(|| {
                    gst::subclass::ElementMetadata::new(
                        "GTK Camera Sink",
                        "Sink/Camera/Video",
                        "A GTK Camera sink",
                        "Bilal Elmoussaoui <bil.elmoussaoui@gmail.com>",
                    )
                });

                Some(&*ELEMENT_METADATA)
            }

            fn pad_templates() -> &'static [gst::PadTemplate] {
                static PAD_TEMPLATES: Lazy<Vec<gst::PadTemplate>> = Lazy::new(|| {
                    let caps = gst_video::video_make_raw_caps(&[
                        gst_video::VideoFormat::Bgra,
                        gst_video::VideoFormat::Argb,
                        gst_video::VideoFormat::Rgba,
                        gst_video::VideoFormat::Abgr,
                        gst_video::VideoFormat::Rgb,
                        gst_video::VideoFormat::Bgr,
                    ])
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
        impl BaseSinkImpl for CameraSink {
            fn set_caps(
                &self,
                _element: &Self::Type,
                caps: &gst::Caps,
            ) -> Result<(), gst::LoggableError> {
                let video_info = gst_video::VideoInfo::from_caps(caps).unwrap();
                let mut info = self.info.lock().unwrap();
                info.replace(video_info);

                Ok(())
            }
        }
        impl VideoSinkImpl for CameraSink {
            fn show_frame(
                &self,
                _element: &Self::Type,
                buffer: &gst::Buffer,
            ) -> Result<gst::FlowSuccess, gst::FlowError> {
                if let Some(info) = &*self.info.lock().unwrap() {
                    let frame = Frame::new(buffer, info);
                    let mut last_frame = self.pending_frame.lock().unwrap();

                    last_frame.replace(frame);
                    let sender = self.sender.lock().unwrap();

                    sender.as_ref().unwrap().send(Action::FrameChanged).unwrap();
                }
                Ok(gst::FlowSuccess::Ok)
            }
        }
    }

    glib::wrapper! {
        pub struct CameraSink(ObjectSubclass<imp::CameraSink>) @extends gst_video::VideoSink, gst_base::BaseSink, gst::Element, gst::Object;
    }
    #[allow(clippy::non_send_fields_in_send_ty)]
    unsafe impl Send for CameraSink {}
    unsafe impl Sync for CameraSink {}

    impl CameraSink {
        pub fn new(sender: Sender<Action>) -> Self {
            let sink = glib::Object::new(&[]).expect("Failed to create a CameraSink");
            let priv_ = imp::CameraSink::from_instance(&sink);
            priv_.sender.lock().unwrap().replace(sender);
            sink
        }

        pub fn pending_frame(&self) -> Option<Frame> {
            let self_ = imp::CameraSink::from_instance(self);
            self_.pending_frame.lock().unwrap().take()
        }
    }
}

mod imp {
    use glib::subclass;
    use std::cell::RefCell;

    use super::*;

    pub struct CameraPaintable {
        pub sink: camera_sink::CameraSink,
        pub pipeline: RefCell<Option<gst::Pipeline>>,
        pub sender: Sender<Action>,
        pub image: RefCell<Option<gdk::Texture>>,
        pub receiver: RefCell<Option<Receiver<Action>>>,
    }

    impl Default for CameraPaintable {
        fn default() -> Self {
            let (sender, r) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
            let receiver = RefCell::new(Some(r));

            Self {
                pipeline: RefCell::default(),
                sink: camera_sink::CameraSink::new(sender.clone()),
                image: RefCell::new(None),
                sender,
                receiver,
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CameraPaintable {
        const NAME: &'static str = "CameraPaintable";
        type Type = super::CameraPaintable;
        type ParentType = glib::Object;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for CameraPaintable {
        fn constructed(&self, obj: &Self::Type) {
            obj.init_widgets();
            self.parent_constructed(obj);
        }
        fn dispose(&self, paintable: &Self::Type) {
            paintable.close_pipeline();
        }

        fn signals() -> &'static [subclass::Signal] {
            static SIGNALS: Lazy<Vec<subclass::Signal>> = Lazy::new(|| {
                vec![subclass::Signal::builder(
                    "code-detected",
                    &[QrVerificationDataBoxed::static_type().into()],
                    glib::Type::UNIT.into(),
                )
                .flags(glib::SignalFlags::RUN_FIRST)
                .build()]
            });
            SIGNALS.as_ref()
        }
    }

    impl PaintableImpl for CameraPaintable {
        fn intrinsic_height(&self, _paintable: &Self::Type) -> i32 {
            if let Some(frame) = &*self.image.borrow() {
                frame.height()
            } else {
                0
            }
        }

        fn intrinsic_width(&self, _paintable: &Self::Type) -> i32 {
            if let Some(frame) = &*self.image.borrow() {
                frame.width()
            } else {
                0
            }
        }

        fn snapshot(
            &self,
            _paintable: &Self::Type,
            snapshot: &gdk::Snapshot,
            width: f64,
            height: f64,
        ) {
            let snapshot = snapshot.downcast_ref::<gtk::Snapshot>().unwrap();

            if let Some(ref image) = *self.image.borrow() {
                // Transformation to avoid stretching the camera. We translate and scale the image.

                let aspect = width / height.max(std::f64::EPSILON); // Do not divide by zero.
                let image_aspect = image.intrinsic_aspect_ratio();

                if image_aspect == 0.0 {
                    image.snapshot(snapshot.upcast_ref(), width, height);
                    return;
                };

                let (new_width, new_height) = match aspect <= image_aspect {
                    true => (height * image_aspect, height), // Mobile view
                    false => (width, width / image_aspect),  // Landscape
                };

                let p = graphene::Point::new(
                    ((width - new_width) / 2.0) as f32,
                    ((height - new_height) / 2.0) as f32,
                );
                snapshot.translate(&p);

                image.snapshot(snapshot.upcast_ref(), new_width, new_height);
            } else {
                snapshot.append_color(
                    &gdk::RGBA::black(),
                    &graphene::Rect::new(0f32, 0f32, width as f32, height as f32),
                );
            }
        }
    }
}

glib::wrapper! {
    pub struct CameraPaintable(ObjectSubclass<imp::CameraPaintable>) @implements gdk::Paintable;
}

impl Default for CameraPaintable {
    fn default() -> Self {
        glib::Object::new(&[]).expect("Failed to create a CameraPaintable")
    }
}

impl CameraPaintable {
    pub fn set_pipewire_fd<F: AsRawFd>(&self, fd: F, node_id: u32) {
        self.close_pipeline();
        let pipewire_element = gst::ElementFactory::make("pipewiresrc", None).unwrap();
        pipewire_element
            .set_property("fd", &fd.as_raw_fd())
            .unwrap();
        pipewire_element
            .set_property("path", &node_id.to_string())
            .unwrap();
        self.init_pipeline(pipewire_element);
    }

    fn init_pipeline(&self, pipewire_src: gst::Element) {
        let self_ = imp::CameraPaintable::from_instance(self);
        let pipeline = gst::Pipeline::new(None);
        let detector = QrCodeDetector::new(self_.sender.clone()).upcast();

        let tee = gst::ElementFactory::make("tee", None).unwrap();
        let queue = gst::ElementFactory::make("queue", None).unwrap();
        let videoconvert1 = gst::ElementFactory::make("videoconvert", None).unwrap();
        let videoconvert2 = gst::ElementFactory::make("videoconvert", None).unwrap();
        let src_pad = queue.static_pad("src").unwrap();

        // Reduce the number of frames we use to get the qrcode from
        let start = Arc::new(Mutex::new(std::time::Instant::now()));
        src_pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
            let mut start = start.lock().unwrap();
            if start.elapsed() < std::time::Duration::from_millis(500) {
                gst::PadProbeReturn::Drop
            } else {
                *start = std::time::Instant::now();
                gst::PadProbeReturn::Ok
            }
        });

        let queue2 = gst::ElementFactory::make("queue", None).unwrap();

        pipeline
            .add_many(&[
                &pipewire_src,
                &tee,
                &queue,
                &videoconvert1,
                &detector,
                &queue2,
                &videoconvert2,
                self_.sink.upcast_ref(),
            ])
            .unwrap();

        gst::Element::link_many(&[&pipewire_src, &tee, &queue, &videoconvert1, &detector]).unwrap();

        tee.link_pads(None, &queue2, None).unwrap();
        gst::Element::link_many(&[&queue2, &videoconvert2, self_.sink.upcast_ref()]).unwrap();
        let bus = pipeline.bus().unwrap();
        bus.add_watch_local(
            clone!(@weak self as paintable => @default-return glib::Continue(false), move |_, msg| {
                if let gst::MessageView::Error(err) = msg.view() {
                    log::error!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                }
                glib::Continue(true)
            }),
        )
        .expect("Failed to add bus watch");
        pipeline.set_state(gst::State::Playing).ok();
        self_.pipeline.replace(Some(pipeline));
    }

    pub fn close_pipeline(&self) {
        let self_ = imp::CameraPaintable::from_instance(self);
        if let Some(pipeline) = self_.pipeline.borrow_mut().take() {
            pipeline.set_state(gst::State::Null).unwrap();
        }
    }

    pub fn init_widgets(&self) {
        let self_ = imp::CameraPaintable::from_instance(self);

        let receiver = self_.receiver.borrow_mut().take().unwrap();
        receiver.attach(
            None,
            glib::clone!(@weak self as paintable => @default-return glib::Continue(false), move |action| paintable.do_action(action)),
        );
    }

    fn do_action(&self, action: Action) -> glib::Continue {
        let self_ = imp::CameraPaintable::from_instance(self);
        match action {
            Action::FrameChanged => {
                if let Some(frame) = self_
                    .sink
                    .pending_frame()
                    .map::<gdk::Texture, _>(Into::into)
                {
                    let width = frame.width();
                    let height = frame.height();

                    let prev_frame = self_.image.replace(Some(frame));

                    let (prev_width, prev_height) = if let Some(frame) = prev_frame {
                        (frame.width(), frame.height())
                    } else {
                        (0, 0)
                    };

                    if prev_width != width || prev_height != height {
                        self.invalidate_size();
                    }
                    self.invalidate_contents();
                }
            }
            Action::QrCodeDetected(code) => {
                self.emit_by_name("code-detected", &[&QrVerificationDataBoxed(code)])
                    .unwrap();
            }
        }
        glib::Continue(true)
    }
}

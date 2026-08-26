use std::path::{Path, PathBuf};

use futures_channel::oneshot;
use gst::prelude::*;
use gtk::{glib, glib::clone, subclass::prelude::*};
use tracing::{error, warn};

/// An error while recording a voice message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceRecorderError {
    /// No microphone could be opened.
    ///
    /// This is the error to expect: it is what happens on a machine without a
    /// capture device, including a remote desktop session without audio
    /// redirection.
    NoMicrophone,
    /// Something else went wrong.
    Other,
}

mod imp {
    use std::{
        cell::{Cell, RefCell},
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::VoiceRecorder)]
    pub struct VoiceRecorder {
        /// The recording pipeline, while one is running.
        pipeline: RefCell<Option<gst::Pipeline>>,
        /// The guard of the bus watch of the pipeline.
        bus_guard: RefCell<Option<gst::bus::BusWatchGuard>>,
        /// The file the recording is written to.
        path: RefCell<Option<PathBuf>>,
        /// The seconds recorded so far.
        seconds: Cell<u64>,
        /// The source of the ticking clock.
        tick_source: RefCell<Option<glib::SourceId>>,
        /// The elapsed time, as a label like "0:07".
        #[property(get)]
        elapsed: RefCell<String>,
        /// The sender to resolve when the end of the stream is reached.
        eos_sender: RefCell<Option<oneshot::Sender<()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VoiceRecorder {
        const NAME: &'static str = "VoiceRecorder";
        type Type = super::VoiceRecorder;
    }

    #[glib::derived_properties]
    impl ObjectImpl for VoiceRecorder {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("failed").build()]);
            SIGNALS.as_ref()
        }

        fn dispose(&self) {
            self.cancel();
        }
    }

    impl VoiceRecorder {
        /// Start recording.
        pub(super) fn start(&self) -> Result<(), VoiceRecorderError> {
            if self.pipeline.borrow().is_some() {
                return Ok(());
            }

            let path = std::env::temp_dir()
                .join(format!("commune-voice-message-{}.ogg", glib::random_int()));

            let pipeline = Self::build_pipeline(&path).map_err(|error| {
                error!("Could not build the voice recording pipeline: {error}");
                VoiceRecorderError::Other
            })?;

            self.watch_bus(&pipeline);

            if pipeline.set_state(gst::State::Playing).is_err() {
                // The one predictable reason the pipeline refuses to start is
                // the source: there is no microphone to open.
                error!("The voice recording pipeline refused to start");
                let _ = pipeline.set_state(gst::State::Null);
                self.bus_guard.take();
                return Err(VoiceRecorderError::NoMicrophone);
            }

            self.seconds.set(0);
            self.set_elapsed(0);
            self.path.replace(Some(path));
            self.pipeline.replace(Some(pipeline));

            let tick_source = glib::timeout_add_seconds_local(
                1,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        let seconds = imp.seconds.get() + 1;
                        imp.seconds.set(seconds);
                        imp.set_elapsed(seconds);
                        glib::ControlFlow::Continue
                    }
                ),
            );
            self.tick_source.replace(Some(tick_source));

            Ok(())
        }

        /// Build the recording pipeline, writing to the given path.
        fn build_pipeline(path: &Path) -> Result<gst::Pipeline, glib::BoolError> {
            let pipeline = gst::Pipeline::new();

            let source = gst::ElementFactory::make("autoaudiosrc").build()?;
            let convert = gst::ElementFactory::make("audioconvert").build()?;
            let resample = gst::ElementFactory::make("audioresample").build()?;
            let encoder = gst::ElementFactory::make("opusenc").build()?;
            let muxer = gst::ElementFactory::make("oggmux").build()?;
            let sink = gst::ElementFactory::make("filesink")
                .property("location", path.display().to_string())
                .build()?;

            let elements = [&source, &convert, &resample, &encoder, &muxer, &sink];
            pipeline.add_many(elements)?;
            gst::Element::link_many(elements)?;

            Ok(pipeline)
        }

        /// Watch the bus of the given pipeline.
        fn watch_bus(&self, pipeline: &gst::Pipeline) {
            let bus = pipeline.bus().expect("a pipeline always has a bus");

            let guard = bus
                .add_watch_local(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move |_, message| {
                        match message.view() {
                            gst::MessageView::Eos(_) => {
                                if let Some(sender) = imp.eos_sender.take() {
                                    let _ = sender.send(());
                                }
                            }
                            gst::MessageView::Error(error) => {
                                // The debug string is where the reason is;
                                // the element that is named is rarely the
                                // element that is wrong.
                                error!(
                                    "Error from the voice recording pipeline: {} ({})",
                                    error.error(),
                                    error.debug().unwrap_or_default(),
                                );
                                imp.cancel();
                                imp.obj().emit_by_name::<()>("failed", &[]);
                            }
                            _ => {}
                        }

                        glib::ControlFlow::Continue
                    }
                ))
                .expect("adding a bus watch should work");

            self.bus_guard.replace(Some(guard));
        }

        /// Set the elapsed label for the given number of seconds.
        fn set_elapsed(&self, seconds: u64) {
            let elapsed = format!("{}:{:02}", seconds / 60, seconds % 60);

            if *self.elapsed.borrow() == elapsed {
                return;
            }

            self.elapsed.replace(elapsed);
            self.obj().notify_elapsed();
        }

        /// Stop recording and return the file holding the finished recording.
        ///
        /// The caller owns the file, and its cleanup.
        pub(super) async fn stop(&self) -> Result<PathBuf, VoiceRecorderError> {
            let Some(pipeline) = self.pipeline.borrow().clone() else {
                return Err(VoiceRecorderError::Other);
            };

            if let Some(source) = self.tick_source.take() {
                source.remove();
            }

            // Ask for the end of the stream, so that the muxer writes a
            // complete file, and wait for it to travel through the pipeline.
            let (sender, receiver) = oneshot::channel();
            self.eos_sender.replace(Some(sender));
            pipeline.send_event(gst::event::Eos::new());

            let timeout = glib::timeout_future_seconds(3);
            futures_util::pin_mut!(timeout);
            let eos = futures_util::future::select(receiver, timeout).await;
            if matches!(eos, futures_util::future::Either::Right(_)) {
                warn!("The voice recording pipeline did not finish in time");
            }

            let path = self.path.borrow().clone();
            self.tear_down();

            path.ok_or(VoiceRecorderError::Other)
        }

        /// Stop recording and throw the recording away.
        pub(super) fn cancel(&self) {
            let path = self.path.borrow().clone();

            self.tear_down();

            if let Some(path) = path
                && let Err(error) = std::fs::remove_file(&path)
            {
                warn!("Could not remove the voice recording file: {error}");
            }
        }

        /// Take the pipeline down, without touching the file.
        fn tear_down(&self) {
            if let Some(pipeline) = self.pipeline.take() {
                let _ = pipeline.set_state(gst::State::Null);
            }
            if let Some(source) = self.tick_source.take() {
                source.remove();
            }
            self.bus_guard.take();
            self.path.take();
            self.eos_sender.take();
        }
    }
}

glib::wrapper! {
    /// A recorder for voice messages.
    ///
    /// It records the default microphone to Ogg Opus in a temporary file.
    /// The duration and waveform MSC3245 asks for are read back from the
    /// file afterwards, by the same code that computes them for an audio
    /// file picked from disk.
    pub struct VoiceRecorder(ObjectSubclass<imp::VoiceRecorder>);
}

impl VoiceRecorder {
    /// Create a new `VoiceRecorder`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Start recording.
    pub(crate) fn start(&self) -> Result<(), VoiceRecorderError> {
        self.imp().start()
    }

    /// Stop recording and return the file holding the finished recording.
    ///
    /// The caller owns the file, and its cleanup.
    pub(crate) async fn stop(&self) -> Result<PathBuf, VoiceRecorderError> {
        self.imp().stop().await
    }

    /// Stop recording and throw the recording away.
    pub(crate) fn cancel(&self) {
        self.imp().cancel();
    }

    /// Connect to the signal emitted when recording fails.
    pub(crate) fn connect_failed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "failed",
            true,
            glib::closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

impl Default for VoiceRecorder {
    fn default() -> Self {
        Self::new()
    }
}

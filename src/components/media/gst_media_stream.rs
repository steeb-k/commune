//! A [`gtk::MediaStream`] that plays through `GStreamer`.
//!
//! GTK has a `GStreamer` media backend of its own, and everywhere it is
//! available `GtkVideo` uses it and this type is not built. It is compiled
//! into `libgtk` and switched on only when the `GStreamer` libraries are found
//! while GTK itself is built, and the conda-forge build we use on macOS is
//! built without them — so there `GtkMediaFile` has no backend at all, and a
//! `GtkVideo` given a file shows an empty frame and a duration of zero without
//! ever reporting an error.
//!
//! This is the same shape as GTK's own `GtkGstMediaFile`, assembled from the
//! pieces the timeline already plays video with: a [`gst_play::Play`] rendering
//! into `gtk4paintablesink`. `GtkVideo` and its `GtkMediaControls` then work
//! unchanged, because all they need is a media stream that is also a paintable.

use gtk::{gdk, gio, glib, prelude::*, subclass::prelude::*};

use super::video_player_renderer::VideoPlayerRenderer;

/// Convert a `GStreamer` time, in nanoseconds, to the microseconds that
/// [`gtk::MediaStream`] counts in.
fn to_stream_time(time: gst::ClockTime) -> i64 {
    i64::try_from(time.useconds()).unwrap_or(i64::MAX)
}

mod imp {
    use std::cell::{OnceCell, RefCell};

    use glib::clone;
    use tracing::warn;

    use super::*;

    #[derive(Debug, Default)]
    pub struct GstMediaStream {
        /// The renderer that the player draws with.
        renderer: VideoPlayerRenderer,
        /// The player driving the stream.
        player: OnceCell<gst_play::Play>,
        /// The paintable that the player renders into.
        paintable: OnceCell<gdk::Paintable>,
        /// The watch on the player's message bus.
        bus_guard: RefCell<Option<gst::bus::BusWatchGuard>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GstMediaStream {
        const NAME: &'static str = "GstMediaStream";
        type Type = super::GstMediaStream;
        type ParentType = gtk::MediaStream;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for GstMediaStream {
        fn constructed(&self) {
            self.parent_constructed();

            let bus_guard = self
                .player()
                .message_bus()
                .add_watch_local(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move |_, message| {
                        if let Ok(message) = gst_play::PlayMessage::parse(message) {
                            imp.handle_message(&message);
                        }

                        glib::ControlFlow::Continue
                    }
                ))
                .expect("adding message bus watch succeeds");
            self.bus_guard.replace(Some(bus_guard));

            // The sink hands out a single paintable for the lifetime of the
            // player, so what it paints changes under us rather than the
            // paintable itself being replaced.
            let paintable = self.paintable();
            paintable.connect_invalidate_contents(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| imp.obj().invalidate_contents()
            ));
            paintable.connect_invalidate_size(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| imp.obj().invalidate_size()
            ));
        }

        fn dispose(&self) {
            self.player().stop();
            self.player().message_bus().set_flushing(true);
            self.bus_guard.take();
        }
    }

    impl PaintableImpl for GstMediaStream {
        fn intrinsic_width(&self) -> i32 {
            self.paintable().intrinsic_width()
        }

        fn intrinsic_height(&self) -> i32 {
            self.paintable().intrinsic_height()
        }

        fn intrinsic_aspect_ratio(&self) -> f64 {
            self.paintable().intrinsic_aspect_ratio()
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            self.paintable().snapshot(snapshot, width, height);
        }
    }

    impl MediaStreamImpl for GstMediaStream {
        fn play(&self) -> bool {
            self.player().play();
            true
        }

        fn pause(&self) {
            self.player().pause();
        }

        fn seek(&self, timestamp: i64) {
            let Ok(timestamp) = u64::try_from(timestamp) else {
                warn!("Cannot seek to a negative timestamp");
                self.obj().seek_failed();
                return;
            };

            self.player().seek(gst::ClockTime::from_useconds(timestamp));
        }

        fn update_audio(&self, muted: bool, volume: f64) {
            self.player().set_mute(muted);
            // `GtkMediaStream` volume is what the user hears, `GstPlay` volume
            // is a linear factor.
            self.player().set_volume(volume * volume * volume);
        }
    }

    impl GstMediaStream {
        /// The player driving this stream.
        fn player(&self) -> &gst_play::Play {
            self.player
                .get_or_init(|| gst_play::Play::new(Some(self.renderer.clone())))
        }

        /// The paintable that the player renders into.
        fn paintable(&self) -> &gdk::Paintable {
            self.paintable.get_or_init(|| self.renderer.paintable())
        }

        /// Set the file to play.
        pub(super) fn set_file(&self, file: &gio::File) {
            self.player().set_uri(Some(file.uri().as_ref()));

            // Preroll, which is what makes the stream prepared, and what
            // `GtkMediaFile` does the moment it is given a file. Callers count
            // on it: the audio player waits to be told the stream is prepared
            // before it plays anything, while `GstPlay` reports nothing at all
            // until it is playing or paused. Left to themselves the two wait
            // for each other and the spinner never stops.
            //
            // The video path never hit this because `GtkVideo` autoplays, which
            // starts the pipeline itself.
            self.player().pause();
        }

        /// Tell the stream what it is playing, if it does not know yet.
        ///
        /// `GtkMediaStream` refuses to do anything until it has been told, and
        /// what it is told cannot be changed afterwards, so this waits for a
        /// media info with a duration in it. Failing that, claiming that both
        /// an audio and a video track exist is the friendlier guess: controls
        /// for a track that turns out not to exist beat missing controls for
        /// one that does.
        fn ensure_prepared(&self) {
            let obj = self.obj();
            if obj.is_prepared() {
                return;
            }

            let Some(media_info) = self.player().media_info() else {
                obj.stream_prepared(true, true, false, 0);
                return;
            };

            obj.stream_prepared(
                media_info.number_of_audio_streams() > 0,
                media_info.number_of_video_streams() > 0,
                media_info.is_seekable(),
                media_info
                    .duration()
                    .map(to_stream_time)
                    .unwrap_or_default(),
            );
        }

        /// Handle a message from the player.
        fn handle_message(&self, message: &gst_play::PlayMessage) {
            let obj = self.obj();

            match message {
                gst_play::PlayMessage::MediaInfoUpdated(update) => {
                    // The first media info of a file has no duration in it yet,
                    // and an unknown duration would be preserved forever.
                    if update.media_info().duration().is_some_and(|d| !d.is_zero()) {
                        self.ensure_prepared();
                    }
                }
                gst_play::PlayMessage::PositionUpdated(update) => {
                    self.ensure_prepared();

                    if let Some(position) = update.position() {
                        obj.update(to_stream_time(position));
                    }
                }
                gst_play::PlayMessage::SeekDone(seek) => {
                    // A seek we did not ask for is the one that loops the file
                    // back to its start.
                    if obj.is_seeking() {
                        obj.seek_success();
                    }

                    if let Some(position) = seek.position() {
                        obj.update(to_stream_time(position));
                    }
                }
                gst_play::PlayMessage::EndOfStream(_) => {
                    self.ensure_prepared();

                    if obj.is_ended() {
                        return;
                    }

                    if obj.is_loop() {
                        self.player().seek(gst::ClockTime::ZERO);
                    } else {
                        obj.stream_ended();
                    }
                }
                gst_play::PlayMessage::Warning(warning) => {
                    warn!("Warning playing media: {}", warning.error());
                }
                // Only the first error says anything about what went wrong.
                gst_play::PlayMessage::Error(error) if obj.error().is_none() => {
                    obj.set_error(error.error().clone());
                }
                _ => {}
            }
        }
    }
}

glib::wrapper! {
    /// A [`gtk::MediaStream`] playing a file through `GStreamer`.
    pub struct GstMediaStream(ObjectSubclass<imp::GstMediaStream>)
        @extends gtk::MediaStream,
        @implements gdk::Paintable;
}

impl GstMediaStream {
    /// Create a stream playing the given file.
    pub(crate) fn new(file: &gio::File) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp().set_file(file);
        obj
    }
}

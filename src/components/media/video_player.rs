use adw::{prelude::*, subclass::prelude::*};
use gtk::{gio, glib, glib::clone, CompositeTemplate};
use tracing::{error, warn};

use super::video_player_renderer::VideoPlayerRenderer;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/media/video_player.ui")]
    #[properties(wrapper_type = super::VideoPlayer)]
    pub struct VideoPlayer {
        #[template_child]
        video: TemplateChild<gtk::Picture>,
        #[template_child]
        timestamp: TemplateChild<gtk::Label>,
        #[template_child]
        player: TemplateChild<gst_play::Play>,
        /// The file that is currently played.
        file: RefCell<Option<gio::File>>,
        /// Whether the player is displayed in its compact form.
        #[property(get, set = Self::set_compact, explicit_notify)]
        compact: Cell<bool>,
        bus_guard: OnceCell<gst::bus::BusWatchGuard>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VideoPlayer {
        const NAME: &'static str = "VideoPlayer";
        type Type = super::VideoPlayer;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            VideoPlayerRenderer::ensure_type();

            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for VideoPlayer {
        fn constructed(&self) {
            self.parent_constructed();

            let bus_guard = self
                .player
                .message_bus()
                .add_watch_local(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move |_, message| {
                        if let Ok(message) = gst_play::PlayMessage::parse(message) {
                            imp.handle_message(message);
                        }

                        glib::ControlFlow::Continue
                    }
                ))
                .expect("adding message bus watch succeeds");
            self.bus_guard
                .set(bus_guard)
                .expect("bus guard is uninitialized");
        }

        fn dispose(&self) {
            self.player.message_bus().set_flushing(true);
        }
    }

    impl WidgetImpl for VideoPlayer {
        fn map(&self) {
            self.parent_map();
            self.player.play();
        }

        fn unmap(&self) {
            self.player.stop();
            self.parent_unmap();
        }
    }

    impl BinImpl for VideoPlayer {}

    impl VideoPlayer {
        /// Set whether this player should be displayed in a compact format.
        fn set_compact(&self, compact: bool) {
            if self.compact.get() == compact {
                return;
            }

            self.compact.set(compact);
            self.obj().notify_compact();
        }

        /// Set the video file to play.
        pub(super) fn play_video_file(&self, file: gio::File) {
            let uri = file.uri();
            self.file.replace(Some(file));
            self.duration_changed(None);

            self.player.set_uri(Some(uri.as_ref()));
            self.player.set_audio_track_enabled(false);
        }

        /// Handle a message from the player.
        fn handle_message(&self, message: gst_play::PlayMessage) {
            match message {
                gst_play::PlayMessage::DurationChanged { duration } => {
                    self.duration_changed(duration);
                }
                gst_play::PlayMessage::Warning { error, .. } => {
                    warn!("Warning playing video: {error}");
                }
                gst_play::PlayMessage::Error { error, .. } => {
                    error!("Error playing video: {error}");
                }
                _ => {}
            }
        }

        /// Handle when the duration changed.
        fn duration_changed(&self, duration: Option<gst::ClockTime>) {
            if let Some(duration) = duration {
                let mut time = duration.seconds();

                let sec = time % 60;
                time -= sec;
                let min = (time % (60 * 60)) / 60;
                time -= min * 60;
                let hour = time / (60 * 60);

                let label = if hour > 0 {
                    // FIXME: Find how to localize this.
                    // hour:minutes:seconds
                    format!("{hour}:{min:02}:{sec:02}")
                } else {
                    // FIXME: Find how to localize this.
                    // minutes:seconds
                    format!("{min:02}:{sec:02}")
                };

                self.timestamp.set_label(&label);
            }

            self.timestamp.set_visible(duration.is_some());
        }
    }
}

glib::wrapper! {
    /// A widget to preview a video file without controls or sound.
    pub struct VideoPlayer(ObjectSubclass<imp::VideoPlayer>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl VideoPlayer {
    /// Create a new video player.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the video file to play.
    pub(crate) fn play_video_file(&self, file: gio::File) {
        self.imp().play_video_file(file);
    }
}

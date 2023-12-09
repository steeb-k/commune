use adw::subclass::prelude::*;
use gst::{bus::BusWatchGuard, ClockTime};
use gst_play::{Play as GstPlay, PlayMessage};
use gtk::{gio, glib, glib::clone, prelude::*, CompositeTemplate};
use tracing::{error, warn};

use super::VideoPlayerRenderer;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/video_player.ui")]
    #[properties(wrapper_type = super::VideoPlayer)]
    pub struct VideoPlayer {
        /// Whether this player should be displayed in a compact format.
        #[property(get, set = Self::set_compact, explicit_notify)]
        pub compact: Cell<bool>,
        pub duration_handler: RefCell<Option<glib::SignalHandlerId>>,
        #[template_child]
        pub video: TemplateChild<gtk::Picture>,
        #[template_child]
        pub timestamp: TemplateChild<gtk::Label>,
        /// The [`GstPlay`] for the video.
        #[template_child]
        #[property(get = Self::player, type = GstPlay)]
        pub player: TemplateChild<GstPlay>,
        pub bus_guard: OnceCell<BusWatchGuard>,
        /// The file that is currently played.
        pub file: RefCell<Option<gio::File>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VideoPlayer {
        const NAME: &'static str = "ComponentsVideoPlayer";
        type Type = super::VideoPlayer;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            VideoPlayerRenderer::static_type();
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
            let obj = self.obj();

            let bus_guard = self.player
                .message_bus()
                .add_watch_local(
                    clone!(@weak obj =>  @default-return glib::ControlFlow::Break, move |_, message| {
                        match PlayMessage::parse(message) {
                            Ok(PlayMessage::DurationChanged { duration }) => obj.duration_changed(duration),
                            Ok(PlayMessage::Warning { error, .. }) => {
                                warn!("Warning playing video: {error}");
                            }
                            Ok(PlayMessage::Error { error, .. }) => {
                                error!("Error playing video: {error}");
                            }
                            _ => {}
                        }

                        glib::ControlFlow::Continue
                    }),
                )
                .unwrap();
            self.bus_guard.set(bus_guard).unwrap();
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
        fn player(&self) -> GstPlay {
            self.player.clone()
        }
        /// Set whether this player should be displayed in a compact format.
        fn set_compact(&self, compact: bool) {
            if self.compact.get() == compact {
                return;
            }

            self.compact.set(compact);
            self.obj().notify_compact();
        }
    }
}

glib::wrapper! {
    /// A widget to preview a video media file without controls or sound.
    pub struct VideoPlayer(ObjectSubclass<imp::VideoPlayer>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl VideoPlayer {
    /// Create a new video player.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the file to display.
    pub fn play_media_file(&self, file: gio::File) {
        self.imp().file.replace(Some(file.clone()));
        self.duration_changed(None);
        let player = self.player();
        player.set_uri(Some(file.uri().as_ref()));
        player.set_audio_track_enabled(false);
    }

    fn duration_changed(&self, duration: Option<ClockTime>) {
        let label = if let Some(duration) = duration {
            let mut time = duration.seconds();

            let sec = time % 60;
            time -= sec;
            let min = (time % (60 * 60)) / 60;
            time -= min * 60;
            let hour = time / (60 * 60);

            if hour > 0 {
                // FIXME: Find how to localize this.
                // hour:minutes:seconds
                format!("{hour}:{min:02}:{sec:02}")
            } else {
                // FIXME: Find how to localize this.
                // minutes:seconds
                format!("{min:02}:{sec:02}")
            }
        } else {
            "--:--".to_owned()
        };
        self.imp().timestamp.set_label(&label);
    }
}

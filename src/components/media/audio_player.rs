use adw::{prelude::*, subclass::prelude::*};
use gtk::{gio, glib};

use crate::utils::BoundObject;

mod imp {
    use std::cell::Cell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/media/audio_player.ui")]
    #[properties(wrapper_type = super::AudioPlayer)]
    pub struct AudioPlayer {
        /// The media file to play.
        #[property(get, set = Self::set_media_file, explicit_notify, nullable)]
        media_file: BoundObject<gtk::MediaFile>,
        /// Whether to play the media automatically.
        #[property(get, set = Self::set_autoplay, explicit_notify)]
        autoplay: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioPlayer {
        const NAME: &'static str = "AudioPlayer";
        type Type = super::AudioPlayer;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AudioPlayer {}

    impl WidgetImpl for AudioPlayer {}
    impl BinImpl for AudioPlayer {}

    impl AudioPlayer {
        /// Set the media file to play.
        fn set_media_file(&self, media_file: Option<gtk::MediaFile>) {
            if self.media_file.obj() == media_file {
                return;
            }

            self.media_file.disconnect_signals();

            if let Some(media_file) = media_file {
                let mut handlers = Vec::new();

                if self.autoplay.get() {
                    let prepared_handler = media_file.connect_prepared_notify(|media_file| {
                        if media_file.is_prepared() {
                            media_file.play();
                        }
                    });
                    handlers.push(prepared_handler);
                }

                self.media_file.set(media_file, handlers);
            }

            self.obj().notify_media_file();
        }

        /// Set whether to play the media automatically.
        fn set_autoplay(&self, autoplay: bool) {
            if self.autoplay.get() == autoplay {
                return;
            }

            self.autoplay.set(autoplay);
            self.obj().notify_autoplay();
        }
    }
}

glib::wrapper! {
    /// A widget displaying a video media file.
    pub struct AudioPlayer(ObjectSubclass<imp::AudioPlayer>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AudioPlayer {
    /// Create a new audio player.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the file to play.
    ///
    /// This is a convenience method that calls
    /// [`AudioPlayer::set_media_file()`].
    pub(crate) fn set_file(&self, file: Option<&gio::File>) {
        self.set_media_file(file.map(gtk::MediaFile::for_file));
    }
}

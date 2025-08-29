use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use glib::clone;
use gtk::glib;
use tracing::warn;

use super::HistoryViewerEvent;
use crate::{
    gettext_f, spawn,
    utils::{File, matrix::MediaMessage},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/history_viewer/audio_row.ui"
    )]
    #[properties(wrapper_type = super::AudioRow)]
    pub struct AudioRow {
        #[template_child]
        play_button: TemplateChild<gtk::Button>,
        #[template_child]
        title_label: TemplateChild<gtk::Label>,
        #[template_child]
        duration_label: TemplateChild<gtk::Label>,
        /// The audio event.
        #[property(get, set = Self::set_event, explicit_notify, nullable)]
        event: RefCell<Option<HistoryViewerEvent>>,
        /// The media file.
        file: RefCell<Option<File>>,
        /// The API for the media file.
        media_file: RefCell<Option<gtk::MediaFile>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioRow {
        const NAME: &'static str = "ContentAudioHistoryViewerRow";
        type Type = super::AudioRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AudioRow {}

    impl WidgetImpl for AudioRow {}
    impl BinImpl for AudioRow {}

    #[gtk::template_callbacks]
    impl AudioRow {
        /// Set the audio event.
        fn set_event(&self, event: Option<HistoryViewerEvent>) {
            if *self.event.borrow() == event {
                return;
            }

            if let Some(event) = &event {
                let media_message = event.media_message();
                if let MediaMessage::Audio(audio) = &media_message {
                    let filename = media_message.filename();
                    self.title_label.set_label(&filename);
                    self.play_button
                        .update_property(&[gtk::accessible::Property::Label(&gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name. In this case, the file to play is an
                            // audio file.
                            "Play {filename}",
                            &[("filename", &filename)],
                        ))]);

                    if let Some(duration) = audio.info.as_ref().and_then(|i| i.duration) {
                        let duration_secs = duration.as_secs();
                        let secs = duration_secs % 60;
                        let mins = (duration_secs % (60 * 60)) / 60;
                        let hours = duration_secs / (60 * 60);

                        let duration = if hours > 0 {
                            format!("{hours:02}:{mins:02}:{secs:02}")
                        } else {
                            format!("{mins:02}:{secs:02}")
                        };

                        self.duration_label.set_label(&duration);
                    } else {
                        self.duration_label.set_label(&gettext("Unknown duration"));
                    }
                }
            }

            self.event.replace(event);
            self.file.take();

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.download_audio().await;
                }
            ));

            self.obj().notify_event();
        }

        /// Download the given audio.
        async fn download_audio(&self) {
            let Some(event) = self.event.borrow().clone() else {
                return;
            };
            let Some(session) = event.room().and_then(|r| r.session()) else {
                return;
            };

            let media_message = event.media_message();
            let client = session.client();

            match media_message.into_tmp_file(&client).await {
                Ok(file) => {
                    self.set_media_file(file);
                }
                Err(error) => {
                    warn!("Could not retrieve audio file: {error}");
                }
            }
        }

        /// Set the media file to play.
        fn set_media_file(&self, file: File) {
            let media_file = gtk::MediaFile::for_file(&file.as_gfile());

            media_file.connect_error_notify(|media_file| {
                if let Some(error) = media_file.error() {
                    warn!("Error reading audio file: {}", error);
                }
            });
            media_file.connect_ended_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |media_file| {
                    if media_file.is_ended() {
                        imp.play_button
                            .set_icon_name("media-playback-start-symbolic");
                    }
                }
            ));

            self.file.replace(Some(file));
            self.media_file.replace(Some(media_file));
        }

        /// Toggle the audio player playing state.
        #[template_callback]
        fn toggle_play(&self) {
            if let Some(media_file) = self.media_file.borrow().as_ref() {
                if media_file.is_playing() {
                    media_file.pause();
                    self.play_button
                        .set_icon_name("media-playback-start-symbolic");
                } else {
                    media_file.play();
                    self.play_button
                        .set_icon_name("media-playback-pause-symbolic");
                }
            }
        }
    }
}

glib::wrapper! {
    /// A row presenting an audio event.
    pub struct AudioRow(ObjectSubclass<imp::AudioRow>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AudioRow {
    /// Construct an empty `AudioRow`.
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for AudioRow {
    fn default() -> Self {
        Self::new()
    }
}

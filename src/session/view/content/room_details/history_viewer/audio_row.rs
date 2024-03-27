use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use glib::clone;
use gtk::{gio, glib, CompositeTemplate};
use matrix_sdk::ruma::events::room::message::{AudioMessageEventContent, MessageType};
use tracing::warn;

use super::HistoryViewerEvent;
use crate::{gettext_f, matrix_filename, session::model::Session, spawn, spawn_tokio};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/history_viewer/audio_row.ui"
    )]
    #[properties(wrapper_type = super::AudioRow)]
    pub struct AudioRow {
        /// The audio event.
        #[property(get, set = Self::set_event, explicit_notify)]
        pub event: RefCell<Option<HistoryViewerEvent>>,
        pub media_file: RefCell<Option<gtk::MediaFile>>,
        #[template_child]
        pub play_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub title_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub duration_label: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioRow {
        const NAME: &'static str = "ContentAudioHistoryViewerRow";
        type Type = super::AudioRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action("audio-row.toggle-play", None, |obj, _, _| {
                obj.toggle_play();
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AudioRow {}

    impl WidgetImpl for AudioRow {}
    impl BinImpl for AudioRow {}

    impl AudioRow {
        /// Set the audio event.
        fn set_event(&self, event: Option<HistoryViewerEvent>) {
            if *self.event.borrow() == event {
                return;
            }
            let obj = self.obj();

            if let Some(event) = &event {
                if let MessageType::Audio(audio) = event.message_content() {
                    let filename = matrix_filename!(audio, Some(mime::AUDIO));

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

                    if let Some(session) = event.room().and_then(|r| r.session()) {
                        spawn!(clone!(@weak obj => async move {
                            obj.download_audio(audio, &session).await;
                        }));
                    }
                }
            }

            self.event.replace(event);
            obj.notify_event();
        }
    }
}

glib::wrapper! {
    /// A row presenting an audio event.
    pub struct AudioRow(ObjectSubclass<imp::AudioRow>)
        @extends gtk::Widget, adw::Bin;
}

impl AudioRow {
    async fn download_audio(&self, audio: AudioMessageEventContent, session: &Session) {
        let client = session.client();
        let handle = spawn_tokio!(async move { client.media().get_file(&audio, true).await });

        match handle.await.unwrap() {
            Ok(Some(data)) => {
                // The GStreamer backend doesn't work with input streams so
                // we need to store the file.
                // See: https://gitlab.gnome.org/GNOME/gtk/-/issues/4062
                let (file, _) = gio::File::new_tmp(None::<String>).unwrap();
                file.replace_contents(
                    &data,
                    None,
                    false,
                    gio::FileCreateFlags::REPLACE_DESTINATION,
                    gio::Cancellable::NONE,
                )
                .unwrap();
                self.prepare_audio(file);
            }
            Ok(None) => {
                warn!("Could not retrieve invalid audio file");
            }
            Err(error) => {
                warn!("Could not retrieve audio file: {error}");
            }
        }
    }

    fn prepare_audio(&self, file: gio::File) {
        let media_file = gtk::MediaFile::for_file(&file);

        media_file.connect_error_notify(clone!(@weak self as obj => move |media_file| {
            if let Some(error) = media_file.error() {
                warn!("Error reading audio file: {}", error);
            }
        }));
        media_file.connect_ended_notify(clone!(@weak self as obj => move |media_file| {
            if media_file.is_ended() {
                obj.imp().play_button.set_icon_name("media-playback-start-symbolic");
            }
        }));

        self.imp().media_file.replace(Some(media_file));
    }

    fn toggle_play(&self) {
        let imp = self.imp();

        if let Some(media_file) = self.imp().media_file.borrow().as_ref() {
            if media_file.is_playing() {
                media_file.pause();
                imp.play_button
                    .set_icon_name("media-playback-start-symbolic");
            } else {
                media_file.play();
                imp.play_button
                    .set_icon_name("media-playback-pause-symbolic");
            }
        }
    }
}

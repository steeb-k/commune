use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gio,
    glib::{self, clone},
    CompositeTemplate,
};
use matrix_sdk::ruma::events::room::message::AudioMessageEventContent;
use tracing::warn;

use super::{media::MediaState, ContentFormat};
use crate::{
    components::{AudioPlayer, Spinner},
    gettext_f,
    session::model::Session,
    spawn, spawn_tokio,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/audio.ui"
    )]
    #[properties(wrapper_type = super::MessageAudio)]
    pub struct MessageAudio {
        /// The body of the audio message.
        #[property(get)]
        pub body: RefCell<Option<String>>,
        /// The state of the audio file.
        #[property(get, builder(MediaState::default()))]
        pub state: Cell<MediaState>,
        /// Whether to display this audio message in a compact format.
        #[property(get)]
        pub compact: Cell<bool>,
        #[template_child]
        pub player: TemplateChild<AudioPlayer>,
        #[template_child]
        pub state_spinner: TemplateChild<Spinner>,
        #[template_child]
        pub state_error: TemplateChild<gtk::Image>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageAudio {
        const NAME: &'static str = "ContentMessageAudio";
        type Type = super::MessageAudio;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageAudio {}

    impl WidgetImpl for MessageAudio {}
    impl BinImpl for MessageAudio {}
}

glib::wrapper! {
    /// A widget displaying an audio message in the timeline.
    pub struct MessageAudio(ObjectSubclass<imp::MessageAudio>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageAudio {
    /// Create a new audio message.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the body of the audio message.
    fn set_body(&self, body: Option<String>) {
        if self.body() == body {
            return;
        }

        let accessible_label = if let Some(filename) = &body {
            gettext_f("Audio: {filename}", &[("filename", filename)])
        } else {
            gettext("Audio")
        };
        self.update_property(&[gtk::accessible::Property::Label(&accessible_label)]);

        self.imp().body.replace(body);

        self.notify_body();
    }

    /// Set the compact format of this audio message.
    fn set_compact(&self, compact: bool) {
        self.imp().compact.set(compact);

        if compact {
            self.remove_css_class("osd");
            self.remove_css_class("toolbar");
        } else {
            self.add_css_class("osd");
            self.add_css_class("toolbar");
        }

        self.notify_compact();
    }

    /// Set the state of the audio file.
    fn set_state(&self, state: MediaState) {
        let imp = self.imp();

        if self.state() == state {
            return;
        }

        match state {
            MediaState::Loading | MediaState::Initial => {
                imp.state_spinner.set_visible(true);
                imp.state_error.set_visible(false);
            }
            MediaState::Ready => {
                imp.state_spinner.set_visible(false);
                imp.state_error.set_visible(false);
            }
            MediaState::Error => {
                imp.state_spinner.set_visible(false);
                imp.state_error.set_visible(true);
            }
        }

        imp.state.set(state);
        self.notify_state();
    }

    /// Convenience method to set the state to `Error` with the given error
    /// message.
    fn set_error(&self, error: String) {
        self.set_state(MediaState::Error);
        self.imp().state_error.set_tooltip_text(Some(&error));
    }

    /// Display the given `audio` message.
    pub fn audio(&self, audio: AudioMessageEventContent, session: &Session, format: ContentFormat) {
        self.set_body(Some(audio.body.clone()));

        let compact = matches!(format, ContentFormat::Compact | ContentFormat::Ellipsized);
        self.set_compact(compact);
        if compact {
            self.set_state(MediaState::Ready);
            return;
        }

        self.set_state(MediaState::Loading);

        let client = session.client();
        let handle = spawn_tokio!(async move { client.media().get_file(&audio, true).await });

        spawn!(
            glib::Priority::LOW,
            clone!(@weak self as obj => async move {
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
                        obj.display_file(file);
                    }
                    Ok(None) => {
                        warn!("Could not retrieve invalid audio file");
                        obj.set_error(gettext("Could not retrieve audio file"));
                    }
                    Err(error) => {
                        warn!("Could not retrieve audio file: {error}");
                        obj.set_error(gettext("Could not retrieve audio file"));
                    }
                }
            })
        );
    }

    fn display_file(&self, file: gio::File) {
        let media_file = gtk::MediaFile::for_file(&file);

        media_file.connect_error_notify(clone!(@weak self as obj => move |media_file| {
            if let Some(error) = media_file.error() {
                warn!("Error reading audio file: {error}");
                obj.set_error(gettext("Error reading audio file"));
            }
        }));

        self.imp().player.set_media_file(Some(media_file));
        self.set_state(MediaState::Ready);
    }
}

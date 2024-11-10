use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    glib::{self, clone},
    CompositeTemplate,
};
use tracing::warn;

use super::ContentFormat;
use crate::{
    components::AudioPlayer,
    gettext_f,
    session::model::Session,
    spawn,
    utils::{matrix::MediaMessage, File, LoadingState},
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
        /// The filename of the audio file.
        #[property(get)]
        pub filename: RefCell<Option<String>>,
        /// The media file.
        pub(super) file: RefCell<Option<File>>,
        /// The state of the audio file.
        #[property(get, builder(LoadingState::default()))]
        pub state: Cell<LoadingState>,
        /// Whether to display this audio message in a compact format.
        #[property(get)]
        pub compact: Cell<bool>,
        #[template_child]
        pub player: TemplateChild<AudioPlayer>,
        #[template_child]
        pub state_spinner: TemplateChild<adw::Spinner>,
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

    /// Set the filename of the audio file.
    fn set_filename(&self, filename: Option<String>) {
        if self.filename() == filename {
            return;
        }

        let accessible_label = if let Some(filename) = &filename {
            gettext_f("Audio: {filename}", &[("filename", filename)])
        } else {
            gettext("Audio")
        };
        self.update_property(&[gtk::accessible::Property::Label(&accessible_label)]);

        self.imp().filename.replace(filename);

        self.notify_filename();
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
    fn set_state(&self, state: LoadingState) {
        let imp = self.imp();

        if self.state() == state {
            return;
        }

        match state {
            LoadingState::Loading | LoadingState::Initial => {
                imp.state_spinner.set_visible(true);
                imp.state_error.set_visible(false);
            }
            LoadingState::Ready => {
                imp.state_spinner.set_visible(false);
                imp.state_error.set_visible(false);
            }
            LoadingState::Error => {
                imp.state_spinner.set_visible(false);
                imp.state_error.set_visible(true);
            }
        }

        imp.state.set(state);
        self.notify_state();
    }

    /// Convenience method to set the state to `Error` with the given error
    /// message.
    fn set_error(&self, error: &str) {
        self.set_state(LoadingState::Error);
        self.imp().state_error.set_tooltip_text(Some(error));
    }

    /// Display the given `audio` message.
    pub fn audio(&self, message: MediaMessage, session: &Session, format: ContentFormat) {
        self.imp().file.take();
        self.set_filename(Some(message.filename()));

        let compact = matches!(format, ContentFormat::Compact | ContentFormat::Ellipsized);
        self.set_compact(compact);
        if compact {
            self.set_state(LoadingState::Ready);
            return;
        }

        self.set_state(LoadingState::Loading);

        let client = session.client();

        spawn!(
            glib::Priority::LOW,
            clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    match message.into_tmp_file(&client).await {
                        Ok(file) => {
                            obj.display_file(file);
                        }
                        Err(error) => {
                            warn!("Could not retrieve audio file: {error}");
                            obj.set_error(&gettext("Could not retrieve audio file"));
                        }
                    }
                }
            )
        );
    }

    fn display_file(&self, file: File) {
        let media_file = gtk::MediaFile::for_file(&file.as_gfile());

        media_file.connect_error_notify(clone!(
            #[weak(rename_to = obj)]
            self,
            move |media_file| {
                if let Some(error) = media_file.error() {
                    warn!("Error reading audio file: {error}");
                    obj.set_error(&gettext("Error reading audio file"));
                }
            }
        ));

        let imp = self.imp();
        imp.file.replace(Some(file));
        imp.player.set_media_file(Some(media_file));
        self.set_state(LoadingState::Ready);
    }
}

impl Default for MessageAudio {
    fn default() -> Self {
        Self::new()
    }
}

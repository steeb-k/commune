use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    glib::{self, clone},
    CompositeTemplate,
};
use tracing::warn;

use super::{content::MessageCacheKey, ContentFormat};
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
        #[template_child]
        player: TemplateChild<AudioPlayer>,
        #[template_child]
        state_spinner: TemplateChild<adw::Spinner>,
        #[template_child]
        state_error: TemplateChild<gtk::Image>,
        /// The filename of the audio file.
        #[property(get)]
        filename: RefCell<Option<String>>,
        /// The cache key for the current audio message.
        ///
        /// The audio is only reloaded if the cache key changes. This is to
        /// avoid reloading the audio when the local echo is updated to a remote
        /// echo.
        cache_key: RefCell<MessageCacheKey>,
        /// The media file.
        file: RefCell<Option<File>>,
        /// The state of the audio file.
        #[property(get, builder(LoadingState::default()))]
        state: Cell<LoadingState>,
        /// Whether to display this audio message in a compact format.
        #[property(get)]
        compact: Cell<bool>,
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

    impl MessageAudio {
        /// Set the filename of the audio file.
        fn set_filename(&self, filename: Option<String>) {
            if *self.filename.borrow() == filename {
                return;
            }
            let obj = self.obj();

            let accessible_label = if let Some(filename) = &filename {
                gettext_f("Audio: {filename}", &[("filename", filename)])
            } else {
                gettext("Audio")
            };
            obj.update_property(&[gtk::accessible::Property::Label(&accessible_label)]);

            self.filename.replace(filename);
            obj.notify_filename();
        }

        /// Set the compact format of this audio message.
        fn set_compact(&self, compact: bool) {
            let obj = self.obj();
            self.compact.set(compact);

            if compact {
                obj.remove_css_class("osd");
                obj.remove_css_class("toolbar");
            } else {
                obj.add_css_class("osd");
                obj.add_css_class("toolbar");
            }

            obj.notify_compact();
        }

        /// Set the state of the audio file.
        fn set_state(&self, state: LoadingState) {
            if self.state.get() == state {
                return;
            }

            match state {
                LoadingState::Loading | LoadingState::Initial => {
                    self.state_spinner.set_visible(true);
                    self.state_error.set_visible(false);
                }
                LoadingState::Ready => {
                    self.state_spinner.set_visible(false);
                    self.state_error.set_visible(false);
                }
                LoadingState::Error => {
                    self.state_spinner.set_visible(false);
                    self.state_error.set_visible(true);
                }
            }

            self.state.set(state);
            self.obj().notify_state();
        }

        /// Convenience method to set the state to `Error` with the given error
        /// message.
        fn set_error(&self, error: &str) {
            self.set_state(LoadingState::Error);
            self.state_error.set_tooltip_text(Some(error));
        }

        /// Set the cache key with the given value.
        ///
        /// Returns `true` if the audio should be reloaded.
        fn set_cache_key(&self, key: MessageCacheKey) -> bool {
            let should_reload = self.cache_key.borrow().should_reload(&key);
            self.cache_key.replace(key);

            should_reload
        }

        /// Display the given `audio` message.
        pub(super) fn audio(
            &self,
            message: MediaMessage,
            session: &Session,
            format: ContentFormat,
            cache_key: MessageCacheKey,
        ) {
            if !self.set_cache_key(cache_key) {
                // We do not need to reload the audio.
                return;
            }

            self.file.take();
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
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        match message.into_tmp_file(&client).await {
                            Ok(file) => {
                                imp.display_file(file);
                            }
                            Err(error) => {
                                warn!("Could not retrieve audio file: {error}");
                                imp.set_error(&gettext("Could not retrieve audio file"));
                            }
                        }
                    }
                )
            );
        }

        fn display_file(&self, file: File) {
            let media_file = gtk::MediaFile::for_file(&file.as_gfile());

            media_file.connect_error_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |media_file| {
                    if let Some(error) = media_file.error() {
                        warn!("Error reading audio file: {error}");
                        imp.set_error(&gettext("Error reading audio file"));
                    }
                }
            ));

            self.file.replace(Some(file));
            self.player.set_media_file(Some(media_file));
            self.set_state(LoadingState::Ready);
        }
    }
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

    /// Display the given `audio` message.
    pub(crate) fn audio(
        &self,
        message: MediaMessage,
        session: &Session,
        format: ContentFormat,
        cache_key: MessageCacheKey,
    ) {
        self.imp().audio(message, session, format, cache_key);
    }
}

impl Default for MessageAudio {
    fn default() -> Self {
        Self::new()
    }
}

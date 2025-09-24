use std::time::Duration;

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone};
use tracing::warn;

mod waveform;
mod waveform_paintable;

use self::waveform::Waveform;
use crate::{
    session::model::Session,
    spawn,
    utils::{
        File, LoadingState,
        matrix::{AudioMessageExt, MediaMessage, MessageCacheKey},
        media::{
            self, MediaFileError,
            audio::{generate_waveform, load_audio_info},
        },
    },
};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/media/audio_player/mod.ui")]
    #[properties(wrapper_type = super::AudioPlayer)]
    pub struct AudioPlayer {
        #[template_child]
        position_label: TemplateChild<gtk::Label>,
        #[template_child]
        waveform: TemplateChild<Waveform>,
        #[template_child]
        spinner: TemplateChild<adw::Spinner>,
        #[template_child]
        error_img: TemplateChild<gtk::Image>,
        #[template_child]
        remaining_label: TemplateChild<gtk::Label>,
        #[template_child]
        bottom_box: TemplateChild<gtk::Box>,
        #[template_child]
        play_button: TemplateChild<gtk::Button>,
        #[template_child]
        filename_label: TemplateChild<gtk::Label>,
        #[template_child]
        position_label_narrow: TemplateChild<gtk::Label>,
        /// The source to play.
        source: RefCell<Option<AudioPlayerSource>>,
        /// The API used to play the audio file.
        #[property(get)]
        media_file: gtk::MediaFile,
        /// The audio file that is currently loaded.
        ///
        /// This is used to keep a strong reference to the temporary file.
        file: RefCell<Option<File>>,
        /// Whether the audio player is the main widget of the current view.
        ///
        /// This hides the filename and centers the play button.
        #[property(get, set = Self::set_standalone, explicit_notify)]
        standalone: Cell<bool>,
        /// Whether we are in narrow mode.
        narrow: Cell<bool>,
        /// The state of the audio file.
        #[property(get, builder(LoadingState::default()))]
        state: Cell<LoadingState>,
        /// The duration of the audio stream, in microseconds.
        duration: Cell<Duration>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioPlayer {
        const NAME: &'static str = "AudioPlayer";
        type Type = super::AudioPlayer;
        type ParentType = adw::BreakpointBin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_css_name("audio-player");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AudioPlayer {
        fn constructed(&self) {
            self.parent_constructed();

            let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                360.0,
                adw::LengthUnit::Px,
            ));
            breakpoint.connect_apply(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.set_narrow(true);
                }
            ));
            breakpoint.connect_unapply(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.set_narrow(false);
                }
            ));
            self.obj().add_breakpoint(breakpoint);

            self.media_file.connect_duration_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |media_file| {
                    if !imp.use_media_file_data() {
                        return;
                    }

                    let duration = Duration::from_micros(media_file.duration().cast_unsigned());
                    imp.set_duration(duration);
                }
            ));

            self.media_file.connect_timestamp_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |media_file| {
                    if !imp.use_media_file_data() {
                        return;
                    }

                    let mut duration = media_file.duration();
                    let timestamp = media_file.timestamp();

                    // The duration should always be bigger than the timestamp, but let's be safe.
                    if duration != 0 && timestamp > duration {
                        duration = timestamp;
                    }

                    let position = if duration == 0 {
                        0.0
                    } else {
                        (timestamp as f64 / duration as f64) as f32
                    };

                    imp.waveform.set_position(position);
                }
            ));

            self.media_file.connect_playing_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_play_button();
                }
            ));

            self.media_file.connect_prepared_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |media_file| {
                    if media_file.is_prepared() {
                        // The media file should only become prepared after the user clicked play,
                        // so start playing it.
                        media_file.set_playing(true);

                        // If the user selected a position while we didn't have a media file, seek
                        // to it.
                        let position = imp.waveform.position();
                        if position > 0.0 {
                            media_file
                                .seek((media_file.duration() as f64 * f64::from(position)) as i64);
                        }
                    }
                }
            ));

            self.media_file.connect_error_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |media_file| {
                    if let Some(error) = media_file.error() {
                        warn!("Could not read audio file: {error}");
                        imp.set_error(&gettext("Error reading audio file"));
                    }
                }
            ));

            self.waveform.connect_position_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_position_labels();
                }
            ));

            self.update_play_button();
        }

        fn dispose(&self) {
            self.media_file.clear();
        }
    }

    impl WidgetImpl for AudioPlayer {}
    impl BreakpointBinImpl for AudioPlayer {}

    #[gtk::template_callbacks]
    impl AudioPlayer {
        /// Set the source to play.
        pub(super) fn set_source(&self, source: Option<AudioPlayerSource>) {
            let should_reload = source.as_ref().is_none_or(|source| {
                self.source
                    .borrow()
                    .as_ref()
                    .is_none_or(|old_source| old_source.should_reload(source))
            });

            if should_reload {
                self.set_state(LoadingState::Initial);
                self.media_file.clear();
                self.file.take();
            }

            self.source.replace(source);

            if should_reload {
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.load_source_duration().await;
                    }
                ));
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.load_source_waveform().await;
                    }
                ));

                self.update_source_filename();
            }

            self.update_play_button();
        }

        /// Set whether the audio player is the main widget of the current view.
        fn set_standalone(&self, standalone: bool) {
            if self.standalone.get() == standalone {
                return;
            }

            self.standalone.set(standalone);
            self.update_layout();
            self.obj().notify_standalone();
        }

        /// Set whether we are in narrow mode.
        fn set_narrow(&self, narrow: bool) {
            if self.narrow.get() == narrow {
                return;
            }

            self.narrow.set(narrow);
            self.update_layout();
        }

        /// Update the layout for the current state.
        fn update_layout(&self) {
            let standalone = self.standalone.get();
            let narrow = self.narrow.get();

            self.position_label.set_visible(!narrow);
            self.remaining_label.set_visible(!narrow);
            self.filename_label.set_visible(!standalone);
            self.position_label_narrow
                .set_visible(narrow && !standalone);

            self.bottom_box.set_halign(if standalone {
                gtk::Align::Center
            } else {
                gtk::Align::Fill
            });
        }

        /// Set the state of the audio stream.
        fn set_state(&self, state: LoadingState) {
            if self.state.get() == state {
                return;
            }

            self.waveform
                .set_sensitive(matches!(state, LoadingState::Initial | LoadingState::Ready));
            self.spinner
                .set_visible(matches!(state, LoadingState::Loading));
            self.error_img
                .set_visible(matches!(state, LoadingState::Error));

            self.state.set(state);
            self.obj().notify_state();
        }

        /// Convenience method to set the state to `Error` with the given error
        /// message.
        fn set_error(&self, error: &str) {
            self.set_state(LoadingState::Error);
            self.error_img.set_tooltip_text(Some(error));
        }

        /// Whether we should use the source data rather than the `GtkMediaFile`
        /// data.
        ///
        /// We cannot use the `GtkMediaFile` data if it doesn't have a `GFile`
        /// set.
        fn use_media_file_data(&self) -> bool {
            self.state.get() != LoadingState::Initial
        }

        /// Set the duration of the audio stream.
        fn set_duration(&self, duration: Duration) {
            if self.duration.get() == duration {
                return;
            }

            self.duration.set(duration);
            self.update_duration_labels_width();
            self.update_position_labels();
        }

        /// Update the width of labels presenting a duration.
        fn update_duration_labels_width(&self) {
            let has_hours = self.duration.get().as_secs() > 60 * 60;
            let time_width = if has_hours { 8 } else { 5 };

            self.position_label.set_width_chars(time_width);
            self.remaining_label.set_width_chars(time_width + 1);
        }

        /// Load the duration of the current source.
        async fn load_source_duration(&self) {
            let Some(source) = self.source.borrow().clone() else {
                self.set_duration(Duration::default());
                return;
            };

            let duration = source.duration().await;
            self.set_duration(duration.unwrap_or_default());
        }

        /// Load the waveform of the current source.
        async fn load_source_waveform(&self) {
            let Some(source) = self.source.borrow().clone() else {
                self.waveform.set_waveform(vec![]);
                return;
            };

            let waveform = source.waveform().await;
            self.waveform.set_waveform(waveform.unwrap_or_default());
        }

        /// Update the name of the source.
        fn update_source_filename(&self) {
            let filename = self
                .source
                .borrow()
                .as_ref()
                .map(AudioPlayerSource::filename)
                .unwrap_or_default();

            self.filename_label.set_label(&filename);
        }

        /// Update the labels displaying the position in the audio stream.
        fn update_position_labels(&self) {
            let duration = self.duration.get();
            let position = self.waveform.position();

            let position = duration.mul_f32(position);
            let remaining = duration.saturating_sub(position);

            self.position_label
                .set_label(&media::time_to_label(&position));
            self.remaining_label
                .set_label(&format!("-{}", media::time_to_label(&remaining)));
        }

        /// Update the play button.
        fn update_play_button(&self) {
            let is_playing = self.media_file.is_playing();

            let (icon_name, tooltip) = if is_playing {
                ("pause-symbolic", gettext("Pause"))
            } else {
                ("play-symbolic", gettext("Play"))
            };

            self.play_button.set_icon_name(icon_name);
            self.play_button.set_tooltip_text(Some(&tooltip));

            if is_playing {
                self.set_state(LoadingState::Ready);
            }
        }

        /// Set the media file to play.
        async fn set_file(&self, file: File) {
            let gfile = file.as_gfile();
            self.media_file.set_file(Some(&gfile));
            self.file.replace(Some(file));

            // Reload the waveform if we got it from a message, because we cannot trust the
            // sender.
            if self
                .source
                .borrow()
                .as_ref()
                .is_some_and(|source| matches!(source, AudioPlayerSource::Message(_)))
                && let Some(waveform) = generate_waveform(&gfile, None).await
            {
                self.waveform.set_waveform(waveform);
            }
        }

        /// Play or pause the media.
        #[template_callback]
        async fn toggle_playing(&self) {
            if self.use_media_file_data() {
                self.media_file.set_playing(!self.media_file.is_playing());
                return;
            }

            let Some(source) = self.source.borrow().clone() else {
                return;
            };

            self.set_state(LoadingState::Loading);

            match source.to_file().await {
                Ok(file) => {
                    self.set_file(file).await;
                }
                Err(error) => {
                    warn!("Could not retrieve audio file: {error}");
                    self.set_error(&gettext("Could not retrieve audio file"));
                }
            }
        }

        /// Seek to the given relative position.
        ///
        /// The position must be a value between 0 and 1.
        #[template_callback]
        fn seek(&self, new_position: f32) {
            if self.use_media_file_data() {
                let duration = self.duration.get();

                if !duration.is_zero() {
                    let timestamp = duration.as_micros() as f64 * f64::from(new_position);
                    self.media_file.seek(timestamp as i64);
                }
            } else {
                self.waveform.set_position(new_position);
            }
        }
    }
}

glib::wrapper! {
    /// A widget displaying a video media file.
    pub struct AudioPlayer(ObjectSubclass<imp::AudioPlayer>)
        @extends gtk::Widget, adw::BreakpointBin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AudioPlayer {
    /// Create a new audio player.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the source to play.
    pub(crate) fn set_source(&self, source: Option<AudioPlayerSource>) {
        self.imp().set_source(source);
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// The possible sources accepted by the audio player.
#[derive(Debug, Clone)]
pub(crate) enum AudioPlayerSource {
    /// An audio file.
    File(gio::File),
    /// An audio message.
    Message(AudioPlayerMessage),
}

impl AudioPlayerSource {
    /// Get the filename of the source.
    fn filename(&self) -> String {
        match self {
            Self::File(file) => file
                .path()
                .and_then(|path| path.file_name().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_default(),
            Self::Message(message) => message.message.filename(),
        }
    }

    /// Whether the source should be reloaded because it has changed.
    fn should_reload(&self, new_source: &Self) -> bool {
        match (self, new_source) {
            (Self::File(file), Self::File(new_file)) => file != new_file,
            (Self::Message(message), Self::Message(new_message)) => {
                message.cache_key.should_reload(&new_message.cache_key)
            }
            _ => true,
        }
    }

    /// Get the duration of this source, if any.
    async fn duration(&self) -> Option<Duration> {
        match self {
            Self::File(file) => load_audio_info(file).await.duration,
            Self::Message(message) => {
                if let MediaMessage::Audio(content) = &message.message {
                    content.info.as_deref().and_then(|info| info.duration)
                } else {
                    None
                }
            }
        }
    }

    /// Get the waveform representation of this source, if any.
    async fn waveform(&self) -> Option<Vec<f32>> {
        match self {
            Self::File(file) => generate_waveform(file, None).await,
            Self::Message(message) => {
                if let MediaMessage::Audio(content) = &message.message {
                    content.normalized_waveform()
                } else {
                    None
                }
            }
        }
    }

    /// Get a file to play this source.
    async fn to_file(&self) -> Result<File, MediaFileError> {
        match self {
            Self::File(file) => Ok(file.clone().into()),
            Self::Message(message) => {
                let Some(session) = message.session.upgrade() else {
                    return Err(MediaFileError::NoSession);
                };

                message
                    .message
                    .clone()
                    .into_tmp_file(&session.client())
                    .await
            }
        }
    }
}

/// The data required to play an audio message.
#[derive(Debug, Clone)]
pub(crate) struct AudioPlayerMessage {
    /// The audio message.
    pub(crate) message: MediaMessage,
    /// The session that will be used to load the file.
    pub(crate) session: glib::WeakRef<Session>,
    /// The cache key for the audio message.
    ///
    /// The audio is only reloaded if the cache key changes. This is to
    /// avoid reloading the audio when the local echo is updated to a remote
    /// echo.
    pub(crate) cache_key: MessageCacheKey,
}

impl AudioPlayerMessage {
    /// Construct a new `AudioPlayerMessage`.
    pub(crate) fn new(
        message: MediaMessage,
        session: &Session,
        cache_key: MessageCacheKey,
    ) -> Self {
        let session_weak = glib::WeakRef::new();
        session_weak.set(Some(session));

        Self {
            message,
            session: session_weak,
            cache_key,
        }
    }
}

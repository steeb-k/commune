use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/components-video-player.ui")]
    pub struct VideoPlayer {
        /// Whether this player should be displayed in a compact format.
        pub compact: Cell<bool>,
        pub duration_handler: RefCell<Option<glib::SignalHandlerId>>,
        #[template_child]
        pub video: TemplateChild<gtk::Picture>,
        #[template_child]
        pub timestamp: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VideoPlayer {
        const NAME: &'static str = "ComponentsVideoPlayer";
        type Type = super::VideoPlayer;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for VideoPlayer {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecBoolean::new(
                    "compact",
                    "Compact",
                    "Whether this player should be displayed in a compact format",
                    false,
                    glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "compact" => obj.set_compact(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "compact" => obj.compact().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for VideoPlayer {}

    impl BinImpl for VideoPlayer {}
}

glib::wrapper! {
    /// A widget displaying a video media file.
    pub struct VideoPlayer(ObjectSubclass<imp::VideoPlayer>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl VideoPlayer {
    /// Create a new video player.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create VideoPlayer")
    }

    pub fn compact(&self) -> bool {
        let priv_ = imp::VideoPlayer::from_instance(self);
        priv_.compact.get()
    }

    pub fn set_compact(&self, compact: bool) {
        let priv_ = imp::VideoPlayer::from_instance(self);

        if self.compact() == compact {
            return;
        }

        priv_.compact.set(compact);
        self.notify("compact");
    }

    /// Set the media_file to display.
    pub fn set_media_file(&self, media_file: &gtk::MediaFile) {
        let priv_ = imp::VideoPlayer::from_instance(self);

        if let Some(handler_id) = priv_.duration_handler.take() {
            if let Some(paintable) = priv_.video.paintable() {
                paintable.disconnect(handler_id);
            }
        }

        priv_.video.set_paintable(Some(media_file));
        let timestamp = &*priv_.timestamp;
        let handler_id =
            media_file.connect_duration_notify(clone!(@weak timestamp => move |media_file| {
                timestamp.set_label(&duration(media_file));
            }));
        priv_.duration_handler.replace(Some(handler_id));
    }
}

/// Get the duration of `media_file` as a `String`.
fn duration(media_file: &gtk::MediaFile) -> String {
    let mut time = media_file.duration() / 1000000;

    let sec = time % 60;
    time -= sec;
    let min = (time % (60 * 60)) / 60;
    time -= min * 60;
    let hour = time / (60 * 60);

    if hour > 0 {
        // FIXME: Find how to localize this.
        // hour:minutes:seconds
        format!("{}:{:02}:{:02}", hour, min, sec)
    } else {
        // FIXME: Find how to localize this.
        // minutes:seconds
        format!("{:02}:{:02}", min, sec)
    }
}

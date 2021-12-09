use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/components-video-player.ui")]
    pub struct VideoPlayer {
        pub media_file: RefCell<Option<gtk::MediaFile>>,
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

    impl ObjectImpl for VideoPlayer {}

    impl WidgetImpl for VideoPlayer {}

    impl BinImpl for VideoPlayer {}
}

glib::wrapper! {
    /// A widget displaying a video media file.
    pub struct VideoPlayer(ObjectSubclass<imp::VideoPlayer>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl VideoPlayer {
    pub fn new(media_file: &gtk::MediaFile) -> Self {
        let self_: Self = glib::Object::new(&[]).expect("Failed to create VideoPlayer");
        self_.build(media_file);
        self_
    }

    pub fn build(&self, media_file: &gtk::MediaFile) {
        let priv_ = imp::VideoPlayer::from_instance(self);

        priv_.video.set_paintable(Some(media_file));
        let timestamp = &*priv_.timestamp;
        media_file.connect_duration_notify(clone!(@weak timestamp => move |media_file| {
            timestamp.set_label(&duration(media_file));
        }));
    }
}

/// Get the duration of `media_file` as a `String`.
fn duration(media_file: &gtk::MediaFile) -> String {
    let mut time = media_file.duration() / 1000000;

    let sec = time % 60;
    time = time - sec;
    let min = (time % (60 * 60)) / 60;
    time = time - (min * 60);
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

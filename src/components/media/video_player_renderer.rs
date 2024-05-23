use adw::subclass::prelude::*;
use gst_gtk::PaintableSink;
use gst_play::{subclass::prelude::*, Play, PlayVideoRenderer};
use gtk::{gdk, glib, prelude::*};

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::VideoPlayerRenderer)]
    pub struct VideoPlayerRenderer {
        /// The sink to use to display the video.
        pub sink: OnceCell<PaintableSink>,
        /// The [`gdk::Paintable`] to render the video into.
        #[property(get = Self::paintable)]
        pub paintable: PhantomData<gdk::Paintable>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VideoPlayerRenderer {
        const NAME: &'static str = "ComponentsVideoPlayerRenderer";
        type Type = super::VideoPlayerRenderer;
        type Interfaces = (PlayVideoRenderer,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for VideoPlayerRenderer {
        fn constructed(&self) {
            self.sink.set(PaintableSink::new(None)).unwrap();
        }
    }

    impl PlayVideoRendererImpl for VideoPlayerRenderer {
        fn create_video_sink(&self, _player: &Play) -> gst::Element {
            self.sink.get().unwrap().clone().upcast()
        }
    }

    impl VideoPlayerRenderer {
        /// The [`gdk::Paintable`] to render the video into.
        fn paintable(&self) -> gdk::Paintable {
            self.sink.get().unwrap().property("paintable")
        }
    }
}

glib::wrapper! {
    /// A widget displaying a video media file.
    pub struct VideoPlayerRenderer(ObjectSubclass<imp::VideoPlayerRenderer>)
        @implements PlayVideoRenderer;
}

impl VideoPlayerRenderer {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for VideoPlayerRenderer {
    fn default() -> Self {
        Self::new()
    }
}

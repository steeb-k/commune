use adw::{prelude::BinExt, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gio,
    glib::{self, clone},
    prelude::*,
    subclass::prelude::*,
};
use log::warn;
use matrix_sdk::ruma::events::room::message::VideoMessageEventContent;

use crate::{
    components::VideoPlayer,
    session::Session,
    spawn, spawn_tokio,
    utils::{cache_dir, uint_to_i32},
};

mod imp {
    use std::cell::Cell;

    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default)]
    pub struct MessageVideo {
        /// The intended display width of the video.
        pub width: Cell<i32>,
        /// The intended display height of the video.
        pub height: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageVideo {
        const NAME: &'static str = "ContentMessageVideo";
        type Type = super::MessageVideo;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for MessageVideo {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_int(
                        "width",
                        "Width",
                        "The intended display width of the video",
                        -1,
                        i32::MAX,
                        -1,
                        glib::ParamFlags::WRITABLE,
                    ),
                    glib::ParamSpec::new_int(
                        "height",
                        "Height",
                        "The intended display height of the video",
                        -1,
                        i32::MAX,
                        -1,
                        glib::ParamFlags::WRITABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "width" => {
                    self.width.set(value.get().unwrap());
                }
                "height" => {
                    self.height.set(value.get().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            // We need to control the value returned by `measure`.
            obj.set_layout_manager(gtk::NONE_LAYOUT_MANAGER);
        }
    }

    impl WidgetImpl for MessageVideo {
        fn measure(
            &self,
            obj: &Self::Type,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            match obj.child() {
                Some(child) => {
                    let original_width = self.width.get();
                    let original_height = self.height.get();

                    if orientation == gtk::Orientation::Vertical {
                        // We limit the width to 320 pixels.
                        let width = for_size.min(320);

                        let nat_height = if original_height > 0 && original_width > 0 {
                            // We don't want the paintable to be upscaled.
                            let width = width.min(original_width);
                            width * original_height / original_width
                        } else {
                            // Get the natural height of the data.
                            child.measure(orientation, width).1
                        };

                        // We limit the height to 240 pixels.
                        let height = nat_height.min(240);
                        (0, height, -1, -1)
                    } else {
                        // We limit the height to 240 pixels.
                        let height = for_size.min(240);

                        let nat_width = if original_height > 0 && original_width > 0 {
                            // We don't want the paintable to be upscaled.
                            let height = height.min(original_height);
                            height * original_width / original_height
                        } else {
                            // Get the natural height of the data.
                            child.measure(orientation, height).1
                        };

                        // We limit the width to 320 pixels.
                        let width = nat_width.min(320);
                        (0, width, -1, -1)
                    }
                }
                None => (0, 0, -1, -1),
            }
        }

        fn request_mode(&self, _obj: &Self::Type) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn size_allocate(&self, obj: &Self::Type, _width: i32, height: i32, baseline: i32) {
            if let Some(child) = obj.child() {
                // We need to allocate just enough width to the child so it doesn't expand.
                let original_width = self.width.get();
                let original_height = self.height.get();
                let width = if original_height > 0 && original_width > 0 {
                    height * original_width / original_height
                } else {
                    // Get the natural width of the video data.
                    child.measure(gtk::Orientation::Horizontal, height).1
                };

                child.allocate(width, height, baseline, None);
            }
        }
    }

    impl BinImpl for MessageVideo {}
}

glib::wrapper! {
    /// A widget displaying an message's thumbnail.
    pub struct MessageVideo(ObjectSubclass<imp::MessageVideo>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageVideo {
    pub fn new(video: VideoMessageEventContent, session: &Session) -> Self {
        let info = video.info.as_deref();
        let width = uint_to_i32(info.and_then(|info| info.width));
        let height = uint_to_i32(info.and_then(|info| info.height));

        let self_: Self = glib::Object::new(&[("width", &width), ("height", &height)])
            .expect("Failed to create MessageVideo");
        self_.build(video, session);
        self_
    }

    fn build(&self, video: VideoMessageEventContent, session: &Session) {
        let body = video.body.clone();
        let client = session.client();
        let handle = spawn_tokio!(async move { client.get_file(video, true,).await });

        spawn!(
            glib::PRIORITY_LOW,
            clone!(@weak self as obj => async move {
                match handle.await.unwrap() {
                    Ok(Some(data)) => {
                        // The GStreamer backend of GtkVideo doesn't work with input streams so
                        // we need to store the file.
                        // See: https://gitlab.gnome.org/GNOME/gtk/-/issues/4062
                        let mut path = cache_dir();
                        path.push(body);
                        let file = gio::File::for_path(path);
                        file.replace_contents(
                            &data,
                            None,
                            false,
                            gio::FileCreateFlags::REPLACE_DESTINATION,
                            gio::NONE_CANCELLABLE,
                        )
                        .unwrap();
                        let media_file = gtk::MediaFile::for_file(&file);
                        media_file.set_muted(true);
                        media_file.connect_prepared_notify(|media_file| media_file.play());

                        let video_player = VideoPlayer::new(&media_file);

                        obj.set_child(Some(&video_player));
                    }
                    Ok(None) => {
                        warn!("Could not retrieve invalid image file");
                        let child = gtk::Label::new(Some(&gettext("Could not retrieve image")));
                        obj.set_child(Some(&child));
                    }
                    Err(error) => {
                        warn!("Could not retrieve image file: {}", error);
                        let child = gtk::Label::new(Some(&gettext("Could not retrieve image")));
                        obj.set_child(Some(&child));
                    }
                }
            })
        );
    }
}

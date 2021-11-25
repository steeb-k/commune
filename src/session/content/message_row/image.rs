use std::convert::TryInto;

use adw::{prelude::BinExt, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gdk,
    gdk_pixbuf::Pixbuf,
    gio,
    glib::{self, clone},
    prelude::*,
    subclass::prelude::*,
};
use log::warn;
use matrix_sdk::{
    media::{MediaEventContent, MediaThumbnailSize},
    ruma::{
        api::client::r0::media::get_content_thumbnail::Method,
        events::room::{message::ImageMessageEventContent, ImageInfo},
        uint,
    },
};

use crate::{session::Session, spawn, spawn_tokio};

mod imp {
    use std::cell::Cell;

    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default)]
    pub struct MessageImage {
        /// The intended display width of the full image.
        pub width: Cell<i32>,
        /// The intended display height of the full image.
        pub height: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageImage {
        const NAME: &'static str = "ContentMessageImage";
        type Type = super::MessageImage;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for MessageImage {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_int(
                        "width",
                        "Width",
                        "The intended display width of the full image",
                        -1,
                        i32::MAX,
                        -1,
                        glib::ParamFlags::WRITABLE,
                    ),
                    glib::ParamSpec::new_int(
                        "height",
                        "Height",
                        "The intended display height of the full image",
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

    impl WidgetImpl for MessageImage {
        fn measure(
            &self,
            obj: &Self::Type,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            match obj.child() {
                Some(child) => {
                    // The GdkPaintable will keep its ratio, so we only need to control the height.
                    if orientation == gtk::Orientation::Vertical {
                        let original_width = self.width.get();
                        let original_height = self.height.get();

                        // We limit the thumbnail's width to 320 pixels.
                        let width = for_size.min(320);

                        let nat_height = if original_height > 0 && original_width > 0 {
                            // We don't want the image to be upscaled.
                            let width = width.min(original_width);
                            width * original_height / original_width
                        } else {
                            // Get the natural height of the image data.
                            child.measure(orientation, width).1
                        };

                        // We limit the thumbnail's height to 240 pixels.
                        let height = nat_height.min(240);
                        (0, height, -1, -1)
                    } else {
                        child.measure(orientation, for_size)
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
                    // Get the natural width of the image data.
                    child.measure(gtk::Orientation::Horizontal, height).1
                };

                child.allocate(width, height, baseline, None);
            }
        }
    }

    impl BinImpl for MessageImage {}
}

glib::wrapper! {
    /// A widget displaying an image message.
    pub struct MessageImage(ObjectSubclass<imp::MessageImage>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageImage {
    pub fn image(image: ImageMessageEventContent, session: &Session) -> Self {
        let (width, height) = get_width_height(image.info.as_deref());

        let self_: Self = glib::Object::new(&[("width", &width), ("height", &height)])
            .expect("Failed to create MessageImage");
        self_.build(image, session);
        self_
    }

    fn build<C>(&self, content: C, session: &Session)
    where
        C: MediaEventContent + Send + Sync + 'static,
    {
        let client = session.client();
        let handle = match content.thumbnail() {
            Some(_) => {
                spawn_tokio!(async move {
                    client
                        .get_thumbnail(
                            content,
                            MediaThumbnailSize {
                                method: Method::Scale,
                                width: uint!(320),
                                height: uint!(240),
                            },
                            true,
                        )
                        .await
                })
            }
            None => {
                spawn_tokio!(async move { client.get_file(content, true,).await })
            }
        };

        spawn!(
            glib::PRIORITY_LOW,
            clone!(@weak self as obj => async move {
                match handle.await.unwrap() {
                    Ok(Some(data)) => {
                        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(&data));
                        let texture = Pixbuf::from_stream(&stream, gio::NONE_CANCELLABLE)
                            .ok()
                            .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf));
                        let child = gtk::Picture::for_paintable(texture.as_ref());

                        // To get rounded corners
                        child.set_overflow(gtk::Overflow::Hidden);
                        child.add_css_class("thumbnail");

                        obj.set_child(Some(&child));
                        obj.queue_resize();
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

/// Gets the width and height of the full image in info.
///
/// Returns a (width, height) tuple with either value set to -1 if it wasn't found.
fn get_width_height(info: Option<&ImageInfo>) -> (i32, i32) {
    let width = info
        .and_then(|info| info.width)
        .and_then(|ui| {
            let u: Option<u16> = ui.try_into().ok();
            u
        })
        .and_then(|u| {
            let i: i32 = u.into();
            Some(i)
        })
        .unwrap_or(-1);

    let height = info
        .and_then(|info| info.height)
        .and_then(|ui| {
            let u: Option<u16> = ui.try_into().ok();
            u
        })
        .and_then(|u| {
            let i: i32 = u.into();
            Some(i)
        })
        .unwrap_or(-1);

    (width, height)
}

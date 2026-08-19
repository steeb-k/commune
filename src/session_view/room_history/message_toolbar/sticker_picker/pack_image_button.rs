use gtk::{gdk, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::api::client::media::get_content_thumbnail::v3::Method;
use tracing::error;

use crate::{
    gettext_f,
    session::{PackImage, Session},
    spawn,
    utils::media::{
        FrameDimensions,
        image::{ImageRequestPriority, ImageSource, ThumbnailDownloader, ThumbnailSettings},
    },
};

/// The size at which an image of a pack is presented, in pixels.
const IMAGE_SIZE: u32 = 64;

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PackImageButton)]
    pub struct PackImageButton {
        /// The session that the image belongs to.
        #[property(get, construct_only)]
        pub(super) session: OnceCell<Session>,
        /// The image presented by this button.
        #[property(get, construct_only)]
        pub(super) image: OnceCell<PackImage>,
        pub(super) picture: gtk::Picture,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PackImageButton {
        const NAME: &'static str = "PackImageButton";
        type Type = super::PackImageButton;
        type ParentType = gtk::Button;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PackImageButton {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            let image = self.image.get().expect("image should be initialized");

            self.picture.set_can_shrink(true);
            self.picture.set_width_request(IMAGE_SIZE.cast_signed());
            self.picture.set_height_request(IMAGE_SIZE.cast_signed());

            obj.set_child(Some(&self.picture));
            obj.set_has_frame(false);
            obj.set_tooltip_text(Some(&image.body()));
            obj.update_property(&[gtk::accessible::Property::Label(&image.body())]);

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }
    }

    impl WidgetImpl for PackImageButton {}
    impl ButtonImpl for PackImageButton {}

    impl PackImageButton {
        /// Present this image as unavailable.
        ///
        /// An image that we could not load is still presented, so that the
        /// pack does not silently lose it, but it cannot be sent.
        fn set_unavailable(&self) {
            let obj = self.obj();
            let image = self.image.get().expect("image should be initialized");

            let icon = gtk::Image::from_icon_name("image-missing-symbolic");
            icon.set_pixel_size(IMAGE_SIZE.cast_signed() / 2);
            icon.set_width_request(IMAGE_SIZE.cast_signed());
            icon.set_height_request(IMAGE_SIZE.cast_signed());

            let label = gettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name.
                "“{name}” could not be loaded",
                &[("name", &image.body())],
            );

            obj.set_child(Some(&icon));
            obj.set_sensitive(false);
            obj.set_tooltip_text(Some(&label));
            obj.update_property(&[gtk::accessible::Property::Label(&label)]);
        }

        /// Load the image presented by this button.
        async fn load(&self) {
            let obj = self.obj();
            let image = self.image.get().expect("image should be initialized");
            let client = self
                .session
                .get()
                .expect("session should be initialized")
                .client();

            let dimensions = FrameDimensions {
                width: IMAGE_SIZE,
                height: IMAGE_SIZE,
            }
            .scale(u32::try_from(obj.scale_factor()).unwrap_or(1));

            let downloader = ThumbnailDownloader {
                main: ImageSource {
                    source: image.uri().into(),
                    info: image.info().map(Into::into),
                },
                // Images of a pack are never encrypted, so the original is
                // always the best source.
                alt: None,
            };
            let settings = ThumbnailSettings {
                dimensions,
                method: Method::Scale,
                animated: true,
                prefer_thumbnail: true,
            };

            match downloader
                .download(client, settings, ImageRequestPriority::Low)
                .await
            {
                Ok(loaded) => {
                    let paintable: gdk::Paintable = loaded.into();
                    self.picture.set_paintable(Some(&paintable));
                }
                Err(error) => {
                    error!("Could not load the image of a pack: {error}");
                    self.set_unavailable();
                }
            }
        }
    }
}

glib::wrapper! {
    /// A button to send an image of a pack.
    pub struct PackImageButton(ObjectSubclass<imp::PackImageButton>)
        @extends gtk::Widget, gtk::Button,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl PackImageButton {
    /// Create a new `PackImageButton` for the given image.
    pub(crate) fn new(session: &Session, image: &PackImage) -> Self {
        glib::Object::builder()
            .property("session", session)
            .property("image", image)
            .build()
    }
}

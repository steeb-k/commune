use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use tracing::error;

use crate::{
    components::AnimatedImagePaintable,
    gettext_f,
    session::{PackImage, Session},
    spawn,
    utils::CountedRef,
};

/// The size at which an image of a pack is presented, in pixels.
const IMAGE_SIZE: u32 = 64;

mod imp {
    use std::cell::{OnceCell, RefCell};

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
        /// The reference that keeps an animated image playing.
        ///
        /// An [`AnimatedImagePaintable`] only advances while something holds
        /// one of these, so without it every animated image of a pack sits on
        /// its first frame.
        animation_ref: RefCell<Option<CountedRef>>,
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

    impl WidgetImpl for PackImageButton {
        fn map(&self) {
            self.parent_map();
            self.update_animation_ref();
        }

        fn unmap(&self) {
            self.parent_unmap();
            self.update_animation_ref();
        }
    }

    impl ButtonImpl for PackImageButton {}

    impl PackImageButton {
        /// Hold or drop the reference that keeps an animated image playing,
        /// according to whether this button is on screen.
        fn update_animation_ref(&self) {
            self.animation_ref.take();

            let Some(paintable) = self
                .picture
                .paintable()
                .and_downcast::<AnimatedImagePaintable>()
            else {
                return;
            };

            if self.obj().is_mapped() {
                self.animation_ref.replace(Some(paintable.animation_ref()));
            }
        }

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
            let session = self.session.get().expect("session should be initialized");

            match image
                .download_thumbnail(session, IMAGE_SIZE, obj.scale_factor())
                .await
            {
                Ok(paintable) => {
                    self.picture.set_paintable(Some(&paintable));
                    self.update_animation_ref();
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

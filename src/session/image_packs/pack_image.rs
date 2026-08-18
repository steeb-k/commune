use gtk::{glib, prelude::*, subclass::prelude::*};
use ruma::{
    OwnedMxcUri,
    events::{room::ImageInfo, sticker::StickerEventContent},
};

use super::events::PackImage as PackImageData;

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PackImage)]
    pub struct PackImage {
        /// The shortcode that identifies this image in its pack.
        #[property(get = Self::shortcode)]
        pub(super) shortcode: OnceCell<String>,
        /// The description of this image.
        ///
        /// Falls back to the shortcode.
        #[property(get = Self::body)]
        body: PhantomData<String>,
        /// The data of this image.
        pub(super) data: OnceCell<PackImageData>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PackImage {
        const NAME: &'static str = "ImagePackImage";
        type Type = super::PackImage;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PackImage {}

    impl PackImage {
        /// The shortcode that identifies this image in its pack.
        fn shortcode(&self) -> String {
            self.shortcode
                .get()
                .expect("shortcode should be initialized")
                .clone()
        }

        /// The description of this image.
        fn body(&self) -> String {
            self.data().body.clone().unwrap_or_else(|| self.shortcode())
        }

        /// The data of this image.
        pub(super) fn data(&self) -> &PackImageData {
            self.data.get().expect("data should be initialized")
        }
    }
}

glib::wrapper! {
    /// An image in an [`ImagePack`](super::ImagePack).
    pub struct PackImage(ObjectSubclass<imp::PackImage>);
}

impl PackImage {
    /// Create a new `PackImage` with the given shortcode and data.
    pub(crate) fn new(shortcode: String, data: PackImageData) -> Self {
        let obj = glib::Object::new::<Self>();

        let imp = obj.imp();
        imp.shortcode
            .set(shortcode)
            .expect("shortcode is not initialized");
        imp.data.set(data).expect("data is not initialized");

        obj
    }

    /// The `mxc://` URI of this image.
    pub(crate) fn uri(&self) -> &OwnedMxcUri {
        &self.imp().data().url
    }

    /// The metadata of this image.
    pub(crate) fn info(&self) -> Option<&ImageInfo> {
        self.imp().data().info.as_deref()
    }

    /// The content to send this image as a sticker.
    pub(crate) fn sticker_content(&self) -> StickerEventContent {
        StickerEventContent::new(
            self.body(),
            self.info().cloned().unwrap_or_default(),
            self.uri().clone(),
        )
    }
}

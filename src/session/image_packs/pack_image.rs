use gtk::{gdk, glib, prelude::*, subclass::prelude::*};
use ruma::{
    OwnedMxcUri,
    api::client::media::get_content_thumbnail::v3::Method,
    events::{room::ImageInfo, sticker::StickerEventContent},
};

use super::events::PackImage as PackImageData;
use crate::{
    session::Session,
    utils::media::{
        FrameDimensions,
        image::{
            ImageError, ImageRequestPriority, ImageSource, ThumbnailDownloader, ThumbnailSettings,
        },
    },
};

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

    /// A copy of the data of this image.
    pub(crate) fn data(&self) -> PackImageData {
        self.imp().data().clone()
    }

    /// The metadata of this image.
    pub(crate) fn info(&self) -> Option<&ImageInfo> {
        self.imp().data().info.as_deref()
    }

    /// Load this image at the given size, in pixels.
    pub(crate) async fn download_thumbnail(
        &self,
        session: &Session,
        size: u32,
        scale_factor: i32,
    ) -> Result<gdk::Paintable, ImageError> {
        let dimensions = FrameDimensions {
            width: size,
            height: size,
        }
        .scale(u32::try_from(scale_factor).unwrap_or(1));

        let downloader = ThumbnailDownloader {
            main: ImageSource {
                source: self.uri().into(),
                info: self.info().map(Into::into),
            },
            // Images of a pack are never encrypted, so the original is always
            // the best source.
            alt: None,
        };
        let settings = ThumbnailSettings {
            dimensions,
            method: Method::Scale,
            animated: true,
            prefer_thumbnail: true,
        };

        downloader
            .download(session.client(), settings, ImageRequestPriority::Low)
            .await
            .map(Into::into)
    }

    /// The content to send this image as a sticker.
    pub(crate) fn sticker_content(&self) -> StickerEventContent {
        commune_core::session::sticker_content(&self.shortcode(), self.imp().data())
    }
}

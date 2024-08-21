use gtk::{gdk, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{
    api::client::media::get_content_thumbnail::v3::Method, events::room::avatar::ImageInfo,
    OwnedMxcUri,
};
use tracing::error;

use crate::{
    session::model::Session,
    spawn,
    utils::media::image::{
        load_image, ImageDimensions, ImageSource, ThumbnailDownloader, ThumbnailSettings,
    },
};

/// The source of an avatar's URI.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "AvatarUriSource")]
pub enum AvatarUriSource {
    /// The URI comes from a Matrix user.
    #[default]
    User = 0,
    /// The URI comes from a Matrix room.
    Room = 1,
}

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::AvatarImage)]
    pub struct AvatarImage {
        /// The image content as a paintable, if any.
        #[property(get)]
        pub paintable: RefCell<Option<gdk::Paintable>>,
        /// The biggest needed size of the user-defined image.
        ///
        /// If this is `0`, no image will be loaded.
        #[property(get, set = Self::set_needed_size, explicit_notify, minimum = 0)]
        pub needed_size: Cell<u32>,
        /// The Matrix URI of the avatar.
        pub(super) uri: RefCell<Option<OwnedMxcUri>>,
        /// The Matrix URI of the `AvatarImage`, as a string.
        #[property(get = Self::uri_string)]
        uri_string: PhantomData<Option<String>>,
        /// Information about the avatar.
        pub(super) info: RefCell<Option<Box<ImageInfo>>>,
        /// The source of the avatar's URI.
        #[property(get, construct_only, builder(AvatarUriSource::default()))]
        pub uri_source: Cell<AvatarUriSource>,
        /// The current session.
        #[property(get, construct_only)]
        pub session: OnceCell<Session>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AvatarImage {
        const NAME: &'static str = "AvatarImage";
        type Type = super::AvatarImage;
    }

    #[glib::derived_properties]
    impl ObjectImpl for AvatarImage {}

    impl AvatarImage {
        /// Set the needed size of the user-defined image.
        ///
        /// Only the biggest size will be stored.
        fn set_needed_size(&self, size: u32) {
            if self.needed_size.get() >= size {
                return;
            }
            let obj = self.obj();

            self.needed_size.set(size);
            obj.load();
            obj.notify_needed_size();
        }

        /// The Matrix URI of the `AvatarImage`.
        pub(super) fn uri(&self) -> Option<OwnedMxcUri> {
            self.uri.borrow().clone()
        }

        /// Set the Matrix URI of the `AvatarImage`.
        ///
        /// Returns whether the URI changed.
        pub(super) fn set_uri(&self, uri: Option<OwnedMxcUri>) -> bool {
            if *self.uri.borrow() == uri {
                return false;
            }

            self.uri.replace(uri);
            self.obj().notify_uri_string();

            true
        }

        /// The Matrix URI of the `AvatarImage`, as a string.
        fn uri_string(&self) -> Option<String> {
            self.uri.borrow().as_ref().map(ToString::to_string)
        }

        /// Information about the avatar.
        pub(super) fn info(&self) -> Option<Box<ImageInfo>> {
            self.info.borrow().clone()
        }

        /// Set information about the avatar.
        pub(super) fn set_info(&self, info: Option<Box<ImageInfo>>) {
            self.info.replace(info);
        }

        /// Set the image content as a paintable
        pub(super) fn set_paintable(&self, paintable: Option<gdk::Paintable>) {
            self.paintable.replace(paintable);
            self.obj().notify_paintable();
        }
    }
}

glib::wrapper! {
    /// The image data for an avatar.
    pub struct AvatarImage(ObjectSubclass<imp::AvatarImage>);
}

impl AvatarImage {
    /// Construct a new `AvatarImage` with the given session, Matrix URI and
    /// avatar info.
    pub fn new(
        session: &Session,
        uri_source: AvatarUriSource,
        uri: Option<OwnedMxcUri>,
        info: Option<Box<ImageInfo>>,
    ) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .property("uri-source", uri_source)
            .build();

        obj.set_uri_and_info(uri, info);
        obj
    }

    /// Set the Matrix URI and information of the avatar.
    pub fn set_uri_and_info(&self, uri: Option<OwnedMxcUri>, info: Option<Box<ImageInfo>>) {
        let imp = self.imp();

        let changed = imp.set_uri(uri);
        imp.set_info(info);

        if changed {
            self.load();
        }
    }

    /// The Matrix URI of the avatar.
    pub fn uri(&self) -> Option<OwnedMxcUri> {
        self.imp().uri()
    }

    /// Information about the avatar.
    pub fn info(&self) -> Option<Box<ImageInfo>> {
        self.imp().info()
    }

    /// Load the image with the current settings.
    fn load(&self) {
        if self.needed_size() == 0 {
            // We do not need the avatar.
            self.imp().set_paintable(None);
            return;
        }

        let Some(uri) = self.uri() else {
            // We do not have an avatar.
            self.imp().set_paintable(None);
            return;
        };

        spawn!(
            glib::Priority::LOW,
            clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    obj.load_inner(uri).await;
                }
            )
        );
    }

    async fn load_inner(&self, uri: OwnedMxcUri) {
        let client = self.session().client();
        let info = self.info();
        let needed_size = self.needed_size();

        let downloader = ThumbnailDownloader {
            main: ImageSource {
                source: (&uri).into(),
                info: info.as_deref().map(Into::into),
            },
            // Avatars are not encrypted so we should always get the thumbnail from the original.
            alt: None,
        };
        let settings = ThumbnailSettings {
            dimensions: ImageDimensions {
                width: needed_size,
                height: needed_size,
            },
            method: Method::Crop,
            animated: true,
            prefer_thumbnail: true,
        };

        match downloader.download_to_file(&client, settings).await {
            Ok(file) => {
                let paintable = load_image(file).await.ok();
                self.imp().set_paintable(paintable);
            }
            Err(error) => error!("Could not fetch avatar: {error}"),
        };
    }
}

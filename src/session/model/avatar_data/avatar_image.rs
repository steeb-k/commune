use gtk::{gdk, glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    media::{MediaFormat, MediaRequest, MediaThumbnailSize},
    ruma::{
        api::client::media::get_content_thumbnail::v3::Method, events::room::MediaSource, MxcUri,
        OwnedMxcUri,
    },
};
use tracing::error;

use crate::{components::ImagePaintable, session::model::Session, spawn, spawn_tokio};

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
    use std::cell::{Cell, OnceCell, RefCell};

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
        /// The Matrix URI of the `AvatarImage`.
        #[property(get = Self::uri, set = Self::set_uri, explicit_notify, nullable, type = Option<String>)]
        pub uri: RefCell<Option<OwnedMxcUri>>,
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
        fn uri(&self) -> Option<String> {
            self.uri.borrow().as_ref().map(ToString::to_string)
        }

        /// Set the Matrix URI of the `AvatarImage`.
        fn set_uri(&self, uri: Option<String>) {
            let uri = uri.map(OwnedMxcUri::from);

            if self.uri.borrow().as_ref() == uri.as_ref() {
                return;
            }
            let obj = self.obj();

            let has_uri = uri.is_some();
            self.uri.replace(uri);

            if has_uri {
                obj.load();
            } else {
                obj.set_image_data(None);
            }

            obj.notify_uri();
        }
    }
}

glib::wrapper! {
    /// The image data for an avatar.
    pub struct AvatarImage(ObjectSubclass<imp::AvatarImage>);
}

impl AvatarImage {
    /// Construct a new `AvatarImage` with the given session and Matrix URI.
    pub fn new(session: &Session, uri: Option<&MxcUri>, uri_source: AvatarUriSource) -> Self {
        glib::Object::builder()
            .property("session", session)
            .property("uri", uri.map(|uri| uri.to_string()))
            .property("uri-source", uri_source)
            .build()
    }

    /// Set the content of the image.
    fn set_image_data(&self, data: Option<Vec<u8>>) {
        let paintable = data
            .and_then(|data| ImagePaintable::from_bytes(&glib::Bytes::from(&data), None).ok())
            .map(|texture| texture.upcast());
        self.imp().paintable.replace(paintable);
        self.notify("paintable");
    }

    fn load(&self) {
        // Don't do anything here if we don't need the avatar.
        if self.needed_size() == 0 {
            return;
        }

        let Some(uri) = self.imp().uri.borrow().clone() else {
            return;
        };

        let client = self.session().client();
        let needed_size = self.needed_size();
        let request = MediaRequest {
            source: MediaSource::Plain(uri),
            format: MediaFormat::Thumbnail(MediaThumbnailSize {
                width: needed_size.into(),
                height: needed_size.into(),
                method: Method::Scale,
            }),
        };
        let handle =
            spawn_tokio!(async move { client.media().get_media_content(&request, true).await });

        spawn!(
            glib::Priority::LOW,
            clone!(@weak self as obj => async move {
                match handle.await.unwrap() {
                    Ok(data) => obj.set_image_data(Some(data)),
                    Err(error) => error!("Could not fetch avatar: {error}"),
                };
            })
        );
    }
}

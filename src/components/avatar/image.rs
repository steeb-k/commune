use gtk::{
    gdk, glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use ruma::{
    api::client::media::get_content_thumbnail::v3::Method, events::room::avatar::ImageInfo,
    OwnedMxcUri,
};

use crate::{
    session::model::Session,
    spawn,
    utils::media::{
        image::{
            ImageError, ImageRequestPriority, ImageSource, ThumbnailDownloader, ThumbnailSettings,
        },
        FrameDimensions,
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
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::AvatarImage)]
    pub struct AvatarImage {
        /// The image content as a paintable, if any.
        #[property(get)]
        paintable: RefCell<Option<gdk::Paintable>>,
        /// The biggest needed size of the user-defined image.
        ///
        /// If this is `0`, no image will be loaded.
        #[property(get, set = Self::set_needed_size, explicit_notify, minimum = 0)]
        needed_size: Cell<u32>,
        /// The Matrix URI of the avatar.
        uri: RefCell<Option<OwnedMxcUri>>,
        /// The Matrix URI of the `AvatarImage`, as a string.
        #[property(get = Self::uri_string)]
        uri_string: PhantomData<Option<String>>,
        /// Information about the avatar.
        info: RefCell<Option<ImageInfo>>,
        /// The source of the avatar's URI.
        #[property(get, construct_only, builder(AvatarUriSource::default()))]
        uri_source: Cell<AvatarUriSource>,
        /// The current session.
        #[property(get, construct_only)]
        session: OnceCell<Session>,
        /// The error encountered when loading the avatar, if any.
        pub(super) error: Cell<Option<ImageError>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AvatarImage {
        const NAME: &'static str = "AvatarImage";
        type Type = super::AvatarImage;
    }

    #[glib::derived_properties]
    impl ObjectImpl for AvatarImage {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("error-changed").build()]);
            SIGNALS.as_ref()
        }
    }

    impl AvatarImage {
        /// Set the needed size of the user-defined image.
        ///
        /// Only the biggest size will be stored.
        fn set_needed_size(&self, size: u32) {
            if self.needed_size.get() >= size {
                return;
            }

            self.needed_size.set(size);
            self.load();
            self.obj().notify_needed_size();
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
        pub(super) fn info(&self) -> Option<ImageInfo> {
            self.info.borrow().clone()
        }

        /// Set information about the avatar.
        pub(super) fn set_info(&self, info: Option<ImageInfo>) {
            self.info.replace(info);
        }

        /// Set the image content as a paintable or the error encountered when
        /// loading the avatar.
        pub(super) fn set_paintable(&self, paintable: Result<Option<gdk::Paintable>, ImageError>) {
            let (paintable, error) = match paintable {
                Ok(paintable) => (paintable, None),
                Err(error) => (None, Some(error)),
            };

            if *self.paintable.borrow() != paintable {
                self.paintable.replace(paintable);
                self.obj().notify_paintable();
            }

            self.set_error(error);
        }

        /// Set the error encountered when loading the avatar, if any.
        fn set_error(&self, error: Option<ImageError>) {
            if self.error.get() == error {
                return;
            }

            self.error.set(error);
            self.obj().emit_by_name::<()>("error-changed", &[]);
        }

        /// Load the image with the current settings.
        pub(super) fn load(&self) {
            if self.needed_size.get() == 0 {
                // We do not need the avatar.
                self.set_paintable(Ok(None));
                return;
            }

            let Some(uri) = self.uri() else {
                // We do not have an avatar.
                self.set_paintable(Ok(None));
                return;
            };

            spawn!(
                glib::Priority::LOW,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.load_inner(uri).await;
                    }
                )
            );
        }

        async fn load_inner(&self, uri: OwnedMxcUri) {
            let client = self.session.get().expect("session is initialized").client();
            let info = self.info();

            let needed_size = self.needed_size.get();
            let dimensions = FrameDimensions {
                width: needed_size,
                height: needed_size,
            };

            let downloader = ThumbnailDownloader {
                main: ImageSource {
                    source: (&uri).into(),
                    info: info.as_ref().map(Into::into),
                },
                // Avatars are not encrypted so we should always get the thumbnail from the
                // original.
                alt: None,
            };
            let settings = ThumbnailSettings {
                dimensions,
                method: Method::Crop,
                animated: true,
                prefer_thumbnail: true,
            };

            // TODO: Change priority depending on size?
            let result = downloader
                .download(client, settings, ImageRequestPriority::Low)
                .await;
            self.set_paintable(result.map(|image| Some(image.into())));
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
    pub(crate) fn new(
        session: &Session,
        uri_source: AvatarUriSource,
        uri: Option<OwnedMxcUri>,
        info: Option<ImageInfo>,
    ) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .property("uri-source", uri_source)
            .build();

        obj.set_uri_and_info(uri, info);
        obj
    }

    /// Set the Matrix URI and information of the avatar.
    pub(crate) fn set_uri_and_info(&self, uri: Option<OwnedMxcUri>, info: Option<ImageInfo>) {
        let imp = self.imp();

        let changed = imp.set_uri(uri);
        imp.set_info(info);

        if changed {
            imp.load();
        }
    }

    /// The Matrix URI of the avatar.
    pub(crate) fn uri(&self) -> Option<OwnedMxcUri> {
        self.imp().uri()
    }

    /// The error encountered when loading the avatar, if any.
    pub(crate) fn error(&self) -> Option<ImageError> {
        self.imp().error.get()
    }

    /// Connect to the signal emitted when the error changed.
    pub(crate) fn connect_error_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "error-changed",
            true,
            closure_local!(|obj: Self| {
                f(&obj);
            }),
        )
    }
}

use adw::{prelude::*, subclass::prelude::*};
use gtk::{gdk, glib, glib::clone, pango};
use ruma::{OwnedMxcUri, api::client::media::get_content_thumbnail::v3::Method};
use tracing::error;

use crate::{
    session::Session,
    spawn,
    utils::media::{
        FrameDimensions,
        image::{ImageRequestPriority, ImageSource, ThumbnailDownloader, ThumbnailSettings},
    },
};

/// The height of a custom emoticon, as a multiple of the height of a line of
/// text.
///
/// The specification asks for the `height` attribute of the `img` element to
/// always be `32`, for the clients that do not support custom emoticons, and
/// for the clients that do to override it with a height that suits the font of
/// the user.
const HEIGHT_FACTOR: f64 = 1.6;

/// The height of a custom emoticon when the font metrics are unknown, in
/// pixels.
const FALLBACK_HEIGHT: i32 = 24;

mod imp {
    use std::cell::{Cell, OnceCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::CustomEmoticon)]
    pub struct CustomEmoticon {
        /// The session to load the image with.
        #[property(get, construct_only)]
        pub(super) session: OnceCell<Session>,
        /// The `mxc://` URI of the image.
        #[property(get, construct_only)]
        pub(super) uri: OnceCell<String>,
        /// The description of the image.
        #[property(get, construct_only)]
        pub(super) body: OnceCell<String>,
        pub(super) picture: gtk::Picture,
        /// The height that the image was loaded at, in pixels.
        loaded_height: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CustomEmoticon {
        const NAME: &'static str = "CustomEmoticon";
        type Type = super::CustomEmoticon;
        type ParentType = adw::Bin;
    }

    #[glib::derived_properties]
    impl ObjectImpl for CustomEmoticon {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            let body = self.body.get().expect("body should be initialized");

            self.picture.set_can_shrink(true);
            self.picture.set_content_fit(gtk::ContentFit::Contain);

            obj.set_child(Some(&self.picture));
            obj.set_valign(gtk::Align::Center);
            obj.set_tooltip_text(Some(body));
            obj.set_accessible_role(gtk::AccessibleRole::Img);
            obj.update_property(&[gtk::accessible::Property::Label(body)]);
        }
    }

    impl WidgetImpl for CustomEmoticon {
        fn map(&self) {
            self.parent_map();
            // The font is only known once the widget is in a window, and it can
            // change while it is presented.
            self.update_size();
        }
    }

    impl BinImpl for CustomEmoticon {}

    impl CustomEmoticon {
        /// The height that the image should be presented at, in pixels.
        fn height(&self) -> i32 {
            let metrics = self.obj().pango_context().metrics(None, None);
            let line_height = (metrics.ascent() + metrics.descent()) / pango::SCALE;

            if line_height <= 0 {
                return FALLBACK_HEIGHT;
            }

            (f64::from(line_height) * HEIGHT_FACTOR).round() as i32
        }

        /// Resize the image for the current font, and load it if its size
        /// changed.
        fn update_size(&self) {
            let height = self.height();
            if self.loaded_height.get() == height {
                return;
            }
            self.loaded_height.set(height);

            self.picture.set_height_request(height);
            // The image keeps its aspect ratio, but it should not be able to
            // push the rest of the message out of the way.
            self.picture.set_width_request(height);
            self.picture.set_size_request(-1, height);

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load(height).await;
                }
            ));
        }

        /// Load the image at the given height.
        async fn load(&self, height: i32) {
            let obj = self.obj();
            let uri =
                OwnedMxcUri::from(self.uri.get().expect("URI should be initialized").as_str());
            let client = self
                .session
                .get()
                .expect("session should be initialized")
                .client();

            let height = u32::try_from(height).unwrap_or(FALLBACK_HEIGHT.cast_unsigned());
            let dimensions = FrameDimensions {
                // The image keeps its aspect ratio, so allow it to be wide.
                width: height * 4,
                height,
            }
            .scale(u32::try_from(obj.scale_factor()).unwrap_or(1));

            let downloader = ThumbnailDownloader {
                main: ImageSource {
                    source: (&uri).into(),
                    info: None,
                },
                // Images of a pack are never encrypted.
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
                Ok(image) => {
                    let paintable: gdk::Paintable = image.into();
                    self.picture.set_paintable(Some(&paintable));
                }
                Err(error) => {
                    error!("Could not load a custom emoticon: {error}");
                    self.picture.set_paintable(gdk::Paintable::NONE);
                }
            }
        }
    }
}

glib::wrapper! {
    /// An image sent inline in a message, as defined by the image packs.
    pub struct CustomEmoticon(ObjectSubclass<imp::CustomEmoticon>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CustomEmoticon {
    /// Create a new `CustomEmoticon` for the image at the given `mxc://` URI.
    ///
    /// `body` is the description of the image, used as its accessible label.
    pub(crate) fn new(session: &Session, uri: &OwnedMxcUri, body: &str) -> Self {
        glib::Object::builder()
            .property("session", session)
            .property("uri", uri.as_str())
            .property("body", body)
            .build()
    }
}

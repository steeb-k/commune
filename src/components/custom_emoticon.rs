use gtk::{gdk, glib, glib::clone, graphene, pango, prelude::*, subclass::prelude::*};
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

/// The widest a custom emoticon can be, relative to its height.
const MAX_ASPECT_RATIO: f64 = 3.0;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

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
        /// The image, once it is loaded.
        pub(super) paintable: RefCell<Option<gdk::Paintable>>,
        /// The height that the image was loaded at, in pixels.
        loaded_height: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CustomEmoticon {
        const NAME: &'static str = "CustomEmoticon";
        type Type = super::CustomEmoticon;
        // Not an `AdwBin`: a widget whose class sets a layout manager, which
        // `AdwBin` does, is measured and allocated through that manager, and
        // the methods below are never called.
        type ParentType = gtk::Widget;
    }

    #[glib::derived_properties]
    impl ObjectImpl for CustomEmoticon {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            let body = self.body.get().expect("body should be initialized");

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

        /// The size of an emoticon follows the font, not the image.
        ///
        /// The images of a pack are the size of a sticker, which the
        /// specification asks to be at least 512 pixels, so presenting one at
        /// its own size would take over the message.
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = if orientation == gtk::Orientation::Vertical {
                self.height()
            } else {
                self.width()
            };

            // The minimum and the natural size are the same, so that the
            // emoticon takes exactly the room that was computed for it.
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(paintable) = self.paintable.borrow().clone() else {
                return;
            };

            let obj = self.obj();
            let width = f64::from(obj.width());
            let height = f64::from(obj.height());

            // Keep the aspect ratio of the image inside the room that we have,
            // which matters when it is wider than we allow.
            let (concrete_width, concrete_height) =
                paintable.compute_concrete_size(0.0, 0.0, width, height);

            let x = ((width - concrete_width) / 2.0) as f32;
            let y = ((height - concrete_height) / 2.0) as f32;

            snapshot.translate(&graphene::Point::new(x, y));
            paintable.snapshot(snapshot, concrete_width, concrete_height);
        }
    }

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

        /// The width that the image should be presented at, in pixels.
        ///
        /// The height is fixed by the font, and the aspect ratio of the image
        /// is kept.
        fn width(&self) -> i32 {
            let ratio = self
                .paintable
                .borrow()
                .as_ref()
                .map(PaintableExt::intrinsic_aspect_ratio)
                .filter(|ratio| *ratio > 0.0)
                .unwrap_or(1.0);

            // An image that is much wider than it is tall would push the rest
            // of the message out of the way.
            let width = f64::from(self.height()) * ratio.min(MAX_ASPECT_RATIO);
            width.round() as i32
        }

        /// Reload the image for the current font, if its size changed.
        fn update_size(&self) {
            let height = self.height();
            if self.loaded_height.get() == height {
                return;
            }
            self.loaded_height.set(height);

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load(height).await;
                }
            ));
        }

        /// Set the image to present.
        fn set_paintable(&self, paintable: Option<gdk::Paintable>) {
            let obj = self.obj();

            if let Some(paintable) = &paintable {
                // An animated image tells us when it must be drawn again.
                paintable.connect_invalidate_contents(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.queue_draw();
                    }
                ));
            }

            self.paintable.replace(paintable);

            // The width follows the aspect ratio, which is only known now.
            obj.queue_resize();
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
                Ok(image) => self.set_paintable(Some(image.into())),
                Err(error) => {
                    error!("Could not load a custom emoticon: {error}");
                    self.set_paintable(None);
                }
            }
        }
    }
}

glib::wrapper! {
    /// An image sent inline in a message, as defined by the image packs.
    pub struct CustomEmoticon(ObjectSubclass<imp::CustomEmoticon>)
        @extends gtk::Widget,
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

use gtk::{gdk, glib, glib::clone, pango, prelude::*, subclass::prelude::*};
use ruma::{OwnedMxcUri, api::client::media::get_content_thumbnail::v3::Method};
use tracing::error;

use crate::{
    session::Session,
    spawn,
    utils::media::{
        FrameDimensions,
        image::{
            ImageRequestPriority, ImageSource, THUMBNAIL_MAX_DIMENSIONS, ThumbnailDownloader,
            ThumbnailSettings,
        },
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
        /// Whether this emoticon is alone in its message.
        ///
        /// Such an emoticon is presented larger.
        #[property(get, set = Self::set_large, explicit_notify)]
        is_large: Cell<bool>,
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
            obj.set_overflow(gtk::Overflow::Hidden);
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
            let (width, height) = self.size();
            let size = if orientation == gtk::Orientation::Vertical {
                height
            } else {
                width
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

            // Draw in the room that we were given, which was measured from the
            // aspect ratio of the image, and never at the size of the image
            // itself.
            paintable.snapshot(snapshot, f64::from(obj.width()), f64::from(obj.height()));
        }
    }

    impl CustomEmoticon {
        /// The size that the image should be presented at, in pixels.
        pub(super) fn size(&self) -> (i32, i32) {
            if self.is_large.get() {
                return self.sticker_size();
            }

            let height = self.line_size();
            let ratio = self
                .intrinsic_size()
                .map_or(1.0, |(width, height)| f64::from(width) / f64::from(height))
                // Among words, an image much wider than it is tall would push
                // the rest of the message out of the way.
                .min(MAX_ASPECT_RATIO);

            ((f64::from(height) * ratio).round() as i32, height)
        }

        /// The size of a line of text, in pixels.
        fn line_size(&self) -> i32 {
            let metrics = self.obj().pango_context().metrics(None, None);
            let line_height = (metrics.ascent() + metrics.descent()) / pango::SCALE;

            if line_height <= 0 {
                return FALLBACK_HEIGHT;
            }

            (f64::from(line_height) * HEIGHT_FACTOR).round() as i32
        }

        /// The size of the image itself, in pixels.
        fn intrinsic_size(&self) -> Option<(i32, i32)> {
            self.paintable
                .borrow()
                .as_ref()
                .map(|paintable| (paintable.intrinsic_width(), paintable.intrinsic_height()))
                .filter(|(width, height)| *width > 0 && *height > 0)
        }

        /// The size to present the image at when it is alone in its message.
        ///
        /// This is what the timeline does with a sticker: the size of the
        /// image, bounded, and never enlarged past it.
        fn sticker_size(&self) -> (i32, i32) {
            let max = THUMBNAIL_MAX_DIMENSIONS;
            // Before the image is loaded, assume it is square, which the
            // specification asks stickers to be.
            let (width, height) = self
                .intrinsic_size()
                .unwrap_or((max.height.cast_signed(), max.height.cast_signed()));

            let scale = (f64::from(max.width) / f64::from(width))
                .min(f64::from(max.height) / f64::from(height))
                .min(1.0);

            (
                ((f64::from(width) * scale).round() as i32).max(1),
                ((f64::from(height) * scale).round() as i32).max(1),
            )
        }

        /// Set whether this emoticon is alone in its message.
        fn set_large(&self, large: bool) {
            if self.is_large.get() == large {
                return;
            }

            self.is_large.set(large);
            // The image is loaded again, at the size it is now presented at.
            self.update_size();

            let obj = self.obj();
            obj.queue_resize();
            obj.notify_is_large();
        }

        /// Reload the image if the size it is presented at changed.
        fn update_size(&self) {
            let (width, height) = self.size();
            if self.loaded_height.get() == height {
                return;
            }
            self.loaded_height.set(height);

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load(width, height).await;
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

        /// Load the image at the given size.
        async fn load(&self, width: i32, height: i32) {
            let obj = self.obj();
            let uri =
                OwnedMxcUri::from(self.uri.get().expect("URI should be initialized").as_str());
            let client = self
                .session
                .get()
                .expect("session should be initialized")
                .client();

            let height = u32::try_from(height).unwrap_or(FALLBACK_HEIGHT.cast_unsigned());
            let width = u32::try_from(width).unwrap_or(height);
            let dimensions = FrameDimensions { width, height }
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

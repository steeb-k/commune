use gtk::{gdk, glib, prelude::*, subclass::prelude::*};
use tracing::debug;

use crate::{
    components::AnimatedImagePaintable,
    utils::{
        CountedRef,
        klipy::Gif,
        media::{
            FrameDimensions,
            image::{IMAGE_QUEUE, ImageRequestPriority},
        },
    },
};

/// The height at which every GIF is presented, in pixels.
///
/// GIFs are not square and cropping them hides what is in them, so they all
/// share a height and keep their aspect ratio. The flow box then packs rows of
/// varying widths, which is how GIF pickers usually present them.
const GIF_HEIGHT: i32 = 90;
/// The smallest width a GIF is presented at, in pixels.
const MIN_GIF_WIDTH: i32 = 60;
/// The largest width a GIF is presented at, in pixels.
///
/// A very wide GIF is cropped rather than allowed to take a whole row.
const MAX_GIF_WIDTH: i32 = 200;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::GifButton)]
    pub struct GifButton {
        /// The GIF presented by this button.
        #[property(get, construct_only)]
        pub(super) gif: OnceCell<Gif>,
        pub(super) picture: gtk::Picture,
        /// The reference that keeps an animated preview playing.
        ///
        /// An [`AnimatedImagePaintable`] only advances while something holds
        /// one of these, so without it every preview sits on its first frame.
        animation_ref: RefCell<Option<CountedRef>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GifButton {
        const NAME: &'static str = "GifButton";
        type Type = super::GifButton;
        type ParentType = gtk::Button;
    }

    #[glib::derived_properties]
    impl ObjectImpl for GifButton {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            let title = self.gif().title();

            self.picture.set_can_shrink(true);
            self.picture.set_content_fit(gtk::ContentFit::Cover);

            obj.set_child(Some(&self.picture));
            obj.set_has_frame(false);
            // The flow box would otherwise stretch the button to the width of its
            // column, which is what made the previews ragged.
            obj.set_halign(gtk::Align::Center);
            obj.set_valign(gtk::Align::Center);
            obj.set_overflow(gtk::Overflow::Hidden);
            obj.set_tooltip_text(Some(&title));
            obj.update_property(&[gtk::accessible::Property::Label(&title)]);
            obj.add_css_class("gif-button");

            // Something to look at until the preview arrives. The API ships a
            // blurred thumbnail inline with the results, so this costs no
            // request of its own.
            if let Some(texture) = self.blur_texture() {
                self.picture.set_paintable(Some(&texture));
            } else {
                obj.add_css_class("loading");
            }
        }
    }

    impl WidgetImpl for GifButton {
        /// Report the size this GIF is presented at, and nothing else.
        ///
        /// The size of the child must not be allowed to decide this. A
        /// decoded preview is whatever pixel size the source happened to be,
        /// so letting `GtkPicture` report its natural size made the buttons
        /// different heights and far larger than asked for.
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = if orientation == gtk::Orientation::Horizontal {
                self.width()
            } else {
                GIF_HEIGHT
            };

            (size, size, -1, -1)
        }

        fn map(&self) {
            self.parent_map();
            self.update_animation_ref();
        }

        fn unmap(&self) {
            self.parent_unmap();
            self.update_animation_ref();
        }
    }

    impl ButtonImpl for GifButton {}

    impl GifButton {
        /// The GIF presented by this button.
        fn gif(&self) -> &Gif {
            self.gif.get().expect("GIF should be initialized")
        }

        /// The width at which this GIF is presented.
        pub(super) fn width(&self) -> i32 {
            let Some(preview) = self.gif().preview().filter(|p| p.height != 0) else {
                return GIF_HEIGHT;
            };

            let width =
                f64::from(GIF_HEIGHT) * f64::from(preview.width) / f64::from(preview.height);
            (width.round() as i32).clamp(MIN_GIF_WIDTH, MAX_GIF_WIDTH)
        }

        /// The blurred placeholder that the API ships with the results.
        fn blur_texture(&self) -> Option<gdk::Texture> {
            let data_uri = self.gif().blur_preview()?;
            // It is a `data:` URI, so the payload is whatever follows the comma.
            let (_, payload) = data_uri.split_once(',')?;
            let bytes = glib::base64_decode(payload);

            gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
                .inspect_err(|error| debug!("Could not decode the placeholder of a GIF: {error}"))
                .ok()
        }

        /// Hold or drop the reference that keeps an animated preview playing,
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

        /// Download and present the preview of the GIF.
        pub(super) async fn load(&self) {
            let Some(url) = self.gif().preview().map(|preview| preview.url.clone()) else {
                self.obj().set_sensitive(false);
                return;
            };

            // The decoder is asked for the size this is presented at. It only ever
            // scales down, so a smaller source is left alone.
            let dimensions = FrameDimensions {
                width: self.width().cast_unsigned(),
                height: GIF_HEIGHT.cast_unsigned(),
            };

            let handle =
                IMAGE_QUEUE.add_http_request(url, Some(dimensions), ImageRequestPriority::Default);

            match handle.await {
                Ok(image) => {
                    self.obj().remove_css_class("loading");
                    self.picture
                        .set_paintable(Some(&gdk::Paintable::from(image)));
                    self.update_animation_ref();
                }
                Err(error) => {
                    // A GIF we cannot present is not worth an error to the user, but it
                    // must not be sendable either.
                    debug!("Could not load the preview of a GIF: {error}");
                    self.obj().set_sensitive(false);
                }
            }
        }
    }
}

glib::wrapper! {
    /// A button to send a GIF from the GIF search.
    pub struct GifButton(ObjectSubclass<imp::GifButton>)
        @extends gtk::Widget, gtk::Button,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl GifButton {
    /// Create a new `GifButton` for the given GIF.
    pub(crate) fn new(gif: &Gif) -> Self {
        glib::Object::builder().property("gif", gif).build()
    }

    /// Download and present the preview of the GIF.
    ///
    /// The page drives this rather than the button doing it on its own, so
    /// that a page of previews is fetched a few at a time instead of all at
    /// once.
    pub(crate) async fn load(&self) {
        self.imp().load().await;
    }
}

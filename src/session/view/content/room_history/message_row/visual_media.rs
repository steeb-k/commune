use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gdk,
    glib::{self, clone},
    CompositeTemplate,
};
use matrix_sdk::Client;
use ruma::api::client::media::get_content_thumbnail::v3::Method;
use tracing::warn;

use super::ContentFormat;
use crate::{
    components::{AnimatedImagePaintable, VideoPlayer},
    gettext_f,
    session::model::Session,
    spawn,
    utils::{
        matrix::VisualMediaMessage,
        media::image::{ImageDimensions, ImageRequestPriority, ThumbnailSettings},
        CountedRef, LoadingState,
    },
};

const MAX_THUMBNAIL_WIDTH: i32 = 600;
const MAX_THUMBNAIL_HEIGHT: i32 = 400;
const FALLBACK_WIDTH: i32 = 480;
const FALLBACK_HEIGHT: i32 = 360;
const MAX_COMPACT_THUMBNAIL_WIDTH: i32 = 75;
const MAX_COMPACT_THUMBNAIL_HEIGHT: i32 = 50;

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/visual_media.ui"
    )]
    #[properties(wrapper_type = super::MessageVisualMedia)]
    pub struct MessageVisualMedia {
        #[template_child]
        pub media: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub overlay_error: TemplateChild<gtk::Image>,
        #[template_child]
        pub overlay_spinner: TemplateChild<adw::Spinner>,
        /// The intended display width of the media.
        #[property(get, default = -1, minimum = -1)]
        pub width: Cell<i32>,
        /// The intended display height of the media.
        #[property(get, default = -1, minimum = -1)]
        pub height: Cell<i32>,
        /// The loading state of the media.
        #[property(get, builder(LoadingState::default()))]
        pub state: Cell<LoadingState>,
        /// Whether to display this media in a compact format.
        #[property(get)]
        pub compact: Cell<bool>,
        paintable_animation_ref: RefCell<Option<CountedRef>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageVisualMedia {
        const NAME: &'static str = "ContentMessageVisualMedia";
        type Type = super::MessageVisualMedia;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("message-visual-media");
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageVisualMedia {
        fn dispose(&self) {
            self.media.unparent();
        }
    }

    impl WidgetImpl for MessageVisualMedia {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let original_width = self.width.get();
            let original_height = self.height.get();

            let compact = self.obj().compact();
            let (max_width, max_height) = if compact {
                (MAX_COMPACT_THUMBNAIL_WIDTH, MAX_COMPACT_THUMBNAIL_HEIGHT)
            } else {
                (MAX_THUMBNAIL_WIDTH, MAX_THUMBNAIL_HEIGHT)
            };

            // -1 means illimited size, and we know we can't go bigger than the max.
            let for_size = if for_size == -1 {
                if orientation == gtk::Orientation::Vertical {
                    max_height
                } else {
                    max_width
                }
            } else {
                for_size
            };

            let (original, max, fallback, original_other, max_other) =
                if orientation == gtk::Orientation::Vertical {
                    (
                        original_height,
                        max_height,
                        FALLBACK_HEIGHT,
                        original_width,
                        max_width,
                    )
                } else {
                    (
                        original_width,
                        max_width,
                        FALLBACK_WIDTH,
                        original_height,
                        max_height,
                    )
                };

            // Limit other side to max size.
            let other = for_size.min(max_other);

            let nat = if original > 0 {
                // We don't want the paintable to be upscaled.
                let other = other.min(original_other);
                other * original / original_other
            } else if let Some(child) = self.media.child() {
                // Get the natural size of the data.
                child.measure(orientation, other).1
            } else {
                fallback
            };

            // Limit this side to max size.
            let size = nat.min(max);
            (0, size, -1, -1)
        }

        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.media.child() {
                // We need to allocate just enough width to the child so it doesn't expand.
                let original_width = self.width.get();
                let original_height = self.height.get();
                let width = if original_height > 0 && original_width > 0 {
                    height * original_width / original_height
                } else {
                    // Get the natural width of the media data.
                    child.measure(gtk::Orientation::Horizontal, height).1
                };

                self.media.allocate(width, height, baseline, None);
            } else {
                self.media.allocate(width, height, baseline, None);
            }
        }

        fn map(&self) {
            self.parent_map();
            self.update_animated_paintable_state();
        }

        fn unmap(&self) {
            self.parent_unmap();
            self.update_animated_paintable_state();
        }
    }

    impl MessageVisualMedia {
        /// Set the intended display width of the media.
        pub(super) fn set_width(&self, width: i32) {
            if self.width.get() == width {
                return;
            }

            self.width.set(width);
            self.obj().notify_width();
        }

        /// Set the intended display height of the media.
        pub(super) fn set_height(&self, height: i32) {
            if self.height.get() == height {
                return;
            }

            self.height.set(height);
            self.obj().notify_height();
        }

        /// Set the state of the media.
        pub(super) fn set_state(&self, state: LoadingState) {
            if self.state.get() == state {
                return;
            }

            match state {
                LoadingState::Loading | LoadingState::Initial => {
                    self.overlay_spinner.set_visible(true);
                    self.overlay_error.set_visible(false);
                }
                LoadingState::Ready => {
                    self.overlay_spinner.set_visible(false);
                    self.overlay_error.set_visible(false);
                }
                LoadingState::Error => {
                    self.overlay_spinner.set_visible(false);
                    self.overlay_error.set_visible(true);
                }
            }

            self.state.set(state);
            self.obj().notify_state();
        }

        /// Update the state of the animated paintable, if any.
        pub(super) fn update_animated_paintable_state(&self) {
            self.paintable_animation_ref.take();

            let Some(paintable) = self
                .media
                .child()
                .and_downcast::<gtk::Picture>()
                .and_then(|p| p.paintable())
                .and_downcast::<AnimatedImagePaintable>()
            else {
                return;
            };

            if self.obj().is_mapped() {
                self.paintable_animation_ref
                    .replace(Some(paintable.animation_ref()));
            }
        }

        /// Set whether to display this media in a compact format.
        pub(super) fn set_compact(&self, compact: bool) {
            if self.compact.get() == compact {
                return;
            }

            self.compact.set(compact);

            if compact {
                self.media.add_css_class("compact");
            } else {
                self.media.remove_css_class("compact");
            }

            self.obj().notify_compact();
        }
    }
}

glib::wrapper! {
    /// A widget displaying a visual media (image or video) message in the timeline.
    pub struct MessageVisualMedia(ObjectSubclass<imp::MessageVisualMedia>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl MessageVisualMedia {
    /// Create a new visual media message.
    pub fn new() -> Self {
        glib::Object::new()
    }

    #[template_callback]
    fn handle_release(&self) {
        self.activate_action("message-row.show-media", None)
            .unwrap();
    }

    /// Display the given visual media message.
    pub fn set_media_message(
        &self,
        media_message: VisualMediaMessage,
        session: &Session,
        format: ContentFormat,
    ) {
        let (width, height) = media_message.dimensions().unzip();
        let compact = matches!(format, ContentFormat::Compact | ContentFormat::Ellipsized);

        let imp = self.imp();
        imp.set_width(width.and_then(|w| w.try_into().ok()).unwrap_or(-1));
        imp.set_height(height.and_then(|h| h.try_into().ok()).unwrap_or(-1));
        imp.set_compact(compact);

        self.build(media_message, session);
    }

    /// Build the content for the given media message.
    fn build(&self, media_message: VisualMediaMessage, session: &Session) {
        let filename = media_message.filename();

        let accessible_label = if filename.is_empty() {
            match &media_message {
                VisualMediaMessage::Image(_) => gettext("Image"),
                VisualMediaMessage::Sticker(_) => gettext("Sticker"),
                VisualMediaMessage::Video(_) => gettext("Video"),
            }
        } else {
            match &media_message {
                VisualMediaMessage::Image(_) => {
                    gettext_f("Image: {filename}", &[("filename", &filename)])
                }
                VisualMediaMessage::Sticker(_) => {
                    gettext_f("Sticker: {filename}", &[("filename", &filename)])
                }
                VisualMediaMessage::Video(_) => {
                    gettext_f("Video: {filename}", &[("filename", &filename)])
                }
            }
        };
        self.update_property(&[gtk::accessible::Property::Label(&accessible_label)]);

        self.imp().set_state(LoadingState::Loading);

        let client = session.client();

        spawn!(
            glib::Priority::LOW,
            clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    obj.build_inner(media_message, client).await;
                }
            )
        );
    }

    async fn build_inner(&self, media_message: VisualMediaMessage, client: Client) {
        let imp = self.imp();

        match &media_message {
            VisualMediaMessage::Image(_) | VisualMediaMessage::Sticker(_) => {
                let is_sticker = matches!(&media_message, VisualMediaMessage::Sticker(_));
                let filename = media_message.filename();

                let scale_factor = self.scale_factor();
                let settings = ThumbnailSettings {
                    dimensions: ImageDimensions {
                        width: u32::try_from(MAX_THUMBNAIL_WIDTH * scale_factor)
                            .unwrap_or_default(),
                        height: u32::try_from(MAX_THUMBNAIL_HEIGHT * scale_factor)
                            .unwrap_or_default(),
                    },
                    method: Method::Scale,
                    animated: true,
                    prefer_thumbnail: false,
                };

                let image = match media_message
                    .thumbnail(client, settings, ImageRequestPriority::Default)
                    .await
                {
                    Ok(Some(image)) => image,
                    Ok(None) => unreachable!("Image messages should always have a fallback"),
                    Err(error) => {
                        imp.overlay_error.set_tooltip_text(Some(&error.to_string()));
                        imp.set_state(LoadingState::Error);

                        return;
                    }
                };

                let child = if let Some(child) = imp.media.child().and_downcast::<gtk::Picture>() {
                    child
                } else {
                    let child = gtk::Picture::new();
                    imp.media.set_child(Some(&child));
                    child
                };
                child.set_paintable(Some(&gdk::Paintable::from(image)));

                child.set_tooltip_text(Some(&filename));
                if is_sticker {
                    imp.media.remove_css_class("opaque-bg");
                } else {
                    imp.media.add_css_class("opaque-bg");
                }
            }
            VisualMediaMessage::Video(_) => {
                let file = match media_message.into_tmp_file(&client).await {
                    Ok(file) => file,
                    Err(error) => {
                        warn!("Could not retrieve media file: {error}");
                        imp.overlay_error
                            .set_tooltip_text(Some(&gettext("Could not retrieve media")));
                        imp.set_state(LoadingState::Error);

                        return;
                    }
                };

                let child = if let Some(child) = imp.media.child().and_downcast::<VideoPlayer>() {
                    child
                } else {
                    let child = VideoPlayer::new();
                    imp.media.set_child(Some(&child));
                    child
                };
                child.set_compact(self.compact());
                child.play_media_file(file);
            }
        };

        imp.update_animated_paintable_state();
        imp.set_state(LoadingState::Ready);
    }

    /// Get the texture displayed by this widget, if any.
    pub fn texture(&self) -> Option<gdk::Texture> {
        let paintable = self
            .imp()
            .media
            .child()
            .and_downcast::<gtk::Picture>()
            .and_then(|p| p.paintable())?;

        if let Some(paintable) = paintable.downcast_ref::<AnimatedImagePaintable>() {
            paintable.current_texture()
        } else if let Ok(texture) = paintable.downcast::<gdk::Texture>() {
            Some(texture)
        } else {
            None
        }
    }
}

impl Default for MessageVisualMedia {
    fn default() -> Self {
        Self::new()
    }
}

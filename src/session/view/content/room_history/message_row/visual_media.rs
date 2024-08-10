use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gdk, gio,
    glib::{self, clone},
    CompositeTemplate,
};
use matrix_sdk::{
    media::{MediaEventContent, MediaThumbnailSettings},
    ruma::{
        api::client::media::get_content_thumbnail::v3::Method,
        events::{
            room::message::{ImageMessageEventContent, VideoMessageEventContent},
            sticker::StickerEventContent,
        },
    },
};
use tracing::warn;

use super::ContentFormat;
use crate::{
    components::{ImagePaintable, Spinner, VideoPlayer},
    gettext_f,
    session::model::Session,
    spawn, spawn_tokio,
    utils::{uint_to_i32, LoadingState},
};

const MAX_THUMBNAIL_WIDTH: i32 = 600;
const MAX_THUMBNAIL_HEIGHT: i32 = 400;
const FALLBACK_WIDTH: i32 = 480;
const FALLBACK_HEIGHT: i32 = 360;
const MAX_COMPACT_THUMBNAIL_WIDTH: i32 = 75;
const MAX_COMPACT_THUMBNAIL_HEIGHT: i32 = 50;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "VisualMediaType")]
pub enum VisualMediaType {
    Image = 0,
    Sticker = 1,
    Video = 2,
}

mod imp {
    use std::cell::Cell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/visual_media.ui"
    )]
    #[properties(wrapper_type = super::MessageVisualMedia)]
    pub struct MessageVisualMedia {
        /// The intended display width of the media.
        #[property(get, set = Self::set_width, explicit_notify, default = -1, minimum = -1)]
        pub width: Cell<i32>,
        /// The intended display height of the media.
        #[property(get, set = Self::set_height, explicit_notify, default = -1, minimum = -1)]
        pub height: Cell<i32>,
        /// The loading state of the media.
        #[property(get, builder(LoadingState::default()))]
        pub state: Cell<LoadingState>,
        /// Whether to display this media in a compact format.
        #[property(get)]
        pub compact: Cell<bool>,
        #[template_child]
        pub media: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub overlay_error: TemplateChild<gtk::Image>,
        #[template_child]
        pub overlay_spinner: TemplateChild<Spinner>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageVisualMedia {
        const NAME: &'static str = "ContentMessageVisualMedia";
        type Type = super::MessageVisualMedia;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

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
                self.media.allocate(width, height, baseline, None)
            }
        }
    }

    impl MessageVisualMedia {
        /// Set the intended display width of the media.
        fn set_width(&self, width: i32) {
            if self.width.get() == width {
                return;
            }

            self.width.set(width);
            self.obj().notify_width();
        }

        /// Set the intended display height of the media.
        fn set_height(&self, height: i32) {
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

    /// Set whether to display this media in a compact format.
    fn set_compact(&self, compact: bool) {
        self.imp().compact.set(compact);
        self.notify_compact();
    }

    /// Display the given `image`, in a `compact` format or not.
    pub fn image(
        &self,
        image: ImageMessageEventContent,
        filename: String,
        session: &Session,
        format: ContentFormat,
    ) {
        let info = image.info.as_deref();
        let width = uint_to_i32(info.and_then(|info| info.width));
        let height = uint_to_i32(info.and_then(|info| info.height));
        let compact = matches!(format, ContentFormat::Compact | ContentFormat::Ellipsized);

        self.set_width(width);
        self.set_height(height);
        self.set_compact(compact);
        self.build(image, filename, VisualMediaType::Image, session);
    }

    /// Display the given `sticker`, in a `compact` format or not.
    pub fn sticker(&self, sticker: StickerEventContent, session: &Session, format: ContentFormat) {
        let info = &sticker.info;
        let width = uint_to_i32(info.width);
        let height = uint_to_i32(info.height);
        let body = sticker.body.clone();
        let compact = matches!(format, ContentFormat::Compact | ContentFormat::Ellipsized);

        self.set_width(width);
        self.set_height(height);
        self.set_compact(compact);
        self.build(sticker, body, VisualMediaType::Sticker, session);
    }

    /// Display the given `video`, in a `compact` format or not.
    pub fn video(
        &self,
        video: VideoMessageEventContent,
        filename: String,
        session: &Session,
        format: ContentFormat,
    ) {
        let info = &video.info.as_deref();
        let width = uint_to_i32(info.and_then(|info| info.width));
        let height = uint_to_i32(info.and_then(|info| info.height));
        let compact = matches!(format, ContentFormat::Compact | ContentFormat::Ellipsized);

        self.set_width(width);
        self.set_height(height);
        self.set_compact(compact);
        self.build(video, filename, VisualMediaType::Video, session);
    }

    fn build<C>(&self, content: C, filename: String, media_type: VisualMediaType, session: &Session)
    where
        C: MediaEventContent + Send + Sync + Clone + 'static,
    {
        let accessible_label = if !filename.is_empty() {
            match media_type {
                VisualMediaType::Image => {
                    gettext_f("Image: {filename}", &[("filename", &filename)])
                }
                VisualMediaType::Sticker => {
                    gettext_f("Sticker: {filename}", &[("filename", &filename)])
                }
                VisualMediaType::Video => {
                    gettext_f("Video: {filename}", &[("filename", &filename)])
                }
            }
        } else {
            match media_type {
                VisualMediaType::Image => gettext("Image"),
                VisualMediaType::Sticker => gettext("Sticker"),
                VisualMediaType::Video => gettext("Video"),
            }
        };
        self.update_property(&[gtk::accessible::Property::Label(&accessible_label)]);

        self.imp().set_state(LoadingState::Loading);
        let scale_factor = self.scale_factor();

        let media = session.client().media();
        let handle = spawn_tokio!(async move {
            let thumbnail =
                if media_type != VisualMediaType::Video && content.thumbnail_source().is_some() {
                    media
                        .get_thumbnail(
                            &content,
                            MediaThumbnailSettings::new(
                                Method::Scale,
                                ((MAX_THUMBNAIL_WIDTH * scale_factor) as u32).into(),
                                ((MAX_THUMBNAIL_HEIGHT * scale_factor) as u32).into(),
                            ),
                            true,
                        )
                        .await
                        .ok()
                        .flatten()
                } else {
                    None
                };

            if let Some(data) = thumbnail {
                Ok(Some(data))
            } else {
                media.get_file(&content, true).await
            }
        });

        spawn!(
            glib::Priority::LOW,
            clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    let imp = obj.imp();

                    match handle.await.unwrap() {
                        Ok(Some(data)) => {
                            match media_type {
                                VisualMediaType::Image | VisualMediaType::Sticker => {
                                    match ImagePaintable::from_bytes(
                                        &glib::Bytes::from(&data),
                                        None,
                                    ) {
                                        Ok(texture) => {
                                            let child = if let Some(child) =
                                                imp.media.child().and_downcast::<gtk::Picture>()
                                            {
                                                child
                                            } else {
                                                let child = gtk::Picture::new();
                                                imp.media.set_child(Some(&child));
                                                child
                                            };
                                            child.set_paintable(Some(&texture));

                                            child.set_tooltip_text(Some(&filename));
                                            if media_type == VisualMediaType::Sticker {
                                                if imp.media.has_css_class("content-thumbnail") {
                                                    imp.media.remove_css_class("content-thumbnail");
                                                }
                                            } else if !imp.media.has_css_class("content-thumbnail")
                                            {
                                                imp.media.add_css_class("content-thumbnail");
                                            }
                                        }
                                        Err(error) => {
                                            warn!("Image file not supported: {error}");
                                            imp.overlay_error.set_tooltip_text(Some(&gettext(
                                                "Image file not supported",
                                            )));
                                            imp.set_state(LoadingState::Error);
                                        }
                                    }
                                }
                                VisualMediaType::Video => {
                                    // The GStreamer backend of GtkVideo doesn't work with input
                                    // streams so we need to
                                    // store the file. See: https://gitlab.gnome.org/GNOME/gtk/-/issues/4062
                                    let (file, _) = gio::File::new_tmp(None::<String>).unwrap();
                                    file.replace_contents(
                                        &data,
                                        None,
                                        false,
                                        gio::FileCreateFlags::REPLACE_DESTINATION,
                                        gio::Cancellable::NONE,
                                    )
                                    .unwrap();

                                    let child = if let Some(child) =
                                        imp.media.child().and_downcast::<VideoPlayer>()
                                    {
                                        child
                                    } else {
                                        let child = VideoPlayer::new();
                                        imp.media.set_child(Some(&child));
                                        child
                                    };
                                    child.set_compact(obj.compact());
                                    child.play_media_file(file)
                                }
                            };

                            imp.set_state(LoadingState::Ready);
                        }
                        Ok(None) => {
                            warn!("Could not retrieve invalid media file");
                            imp.overlay_error
                                .set_tooltip_text(Some(&gettext("Could not retrieve media")));
                            imp.set_state(LoadingState::Error);
                        }
                        Err(error) => {
                            warn!("Could not retrieve media file: {error}");
                            imp.overlay_error
                                .set_tooltip_text(Some(&gettext("Could not retrieve media")));
                            imp.set_state(LoadingState::Error);
                        }
                    }
                }
            )
        );
    }

    /// Get the texture displayed by this widget, if any.
    pub fn texture(&self) -> Option<gdk::Texture> {
        self.imp()
            .media
            .child()
            .and_downcast::<gtk::Picture>()
            .and_then(|p| p.paintable())
            .and_downcast::<ImagePaintable>()
            .and_then(|p| p.current_frame())
    }
}

impl Default for MessageVisualMedia {
    fn default() -> Self {
        Self::new()
    }
}

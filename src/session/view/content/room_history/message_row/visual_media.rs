use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gdk,
    glib::{self, clone},
    CompositeTemplate,
};
use matrix_sdk::Client;
use ruma::api::client::media::get_content_thumbnail::v3::Method;
use tracing::{error, warn};

use super::ContentFormat;
use crate::{
    components::{AnimatedImagePaintable, VideoPlayer},
    gettext_f,
    session::model::Session,
    spawn,
    utils::{
        matrix::VisualMediaMessage,
        media::{
            image::{ImageRequestPriority, ThumbnailSettings, THUMBNAIL_MAX_DIMENSIONS},
            FrameDimensions,
        },
        CountedRef, File, LoadingState,
    },
};

/// The dimensions to use for the media until we know its size.
const FALLBACK_DIMENSIONS: FrameDimensions = FrameDimensions {
    width: 480,
    height: 360,
};
/// The maximum dimensions allowed for the media in its compact form.
const MAX_COMPACT_DIMENSIONS: FrameDimensions = FrameDimensions {
    width: 75,
    height: 50,
};
/// The name of the media stack page.
const MEDIA_PAGE: &str = "media";

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
        overlay: TemplateChild<gtk::Overlay>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        spinner: TemplateChild<adw::Spinner>,
        #[template_child]
        error: TemplateChild<gtk::Image>,
        /// The supposed dimensions of the media.
        dimensions: Cell<Option<FrameDimensions>>,
        /// The loading state of the media.
        #[property(get, builder(LoadingState::default()))]
        state: Cell<LoadingState>,
        /// Whether to display this media in a compact format.
        #[property(get)]
        compact: Cell<bool>,
        /// The current video file, if any.
        file: RefCell<Option<File>>,
        paintable_animation_ref: RefCell<Option<CountedRef>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageVisualMedia {
        const NAME: &'static str = "ContentMessageVisualMedia";
        type Type = super::MessageVisualMedia;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

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
            self.overlay.unparent();
        }
    }

    impl WidgetImpl for MessageVisualMedia {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            // Get the maximum size for the current state.
            let max_size = if self.compact.get() {
                MAX_COMPACT_DIMENSIONS
            } else {
                THUMBNAIL_MAX_DIMENSIONS
            };
            let max = max_size.dimension_for_orientation(orientation);
            let max_for_size = i32::try_from(max_size.dimension_for_other_orientation(orientation))
                .unwrap_or(i32::MAX);

            // Limit for_size to the max.
            let for_size = if for_size == -1 {
                // -1 means unlimited.
                max_for_size
            } else {
                for_size.min(max_for_size)
            };

            // Use the size measured by the media child when we can, it is the natural size
            // of the media.
            if self.stack.visible_child_name().as_deref() == Some(MEDIA_PAGE) {
                if let Some(child) = self.media_child::<gtk::Widget>() {
                    // Get the intrinsic size of the media to avoid upscaling it. It is the size
                    // returned by GtkPicture when for_size is -1.
                    let other_orientation = if orientation == gtk::Orientation::Vertical {
                        gtk::Orientation::Horizontal
                    } else {
                        gtk::Orientation::Vertical
                    };
                    let (_, intrinsic_for_size, ..) = child.measure(other_orientation, -1);

                    let (_, nat, ..) = child.measure(orientation, for_size.min(intrinsic_for_size));

                    if nat != 0 {
                        // Limit the returned size to the max.
                        return (0, nat.min(max.try_into().unwrap_or(i32::MAX)), -1, -1);
                    }
                }
            }

            // Limit the wanted size to the max size.
            let for_size = u32::try_from(for_size).unwrap_or(0);
            let wanted_size = if orientation == gtk::Orientation::Vertical {
                FrameDimensions {
                    width: for_size,
                    height: max,
                }
            } else {
                FrameDimensions {
                    width: max,
                    height: for_size,
                }
            };

            // Use the size from the info or the fallback size.
            let media_size = self.dimensions.get().unwrap_or(FALLBACK_DIMENSIONS);
            let nat = media_size
                .scale_to_fit(wanted_size, gtk::ContentFit::ScaleDown)
                .dimension_for_orientation(orientation);

            (0, nat.try_into().unwrap_or(i32::MAX), -1, -1)
        }

        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.overlay.allocate(width, height, baseline, None);
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

    #[gtk::template_callbacks]
    impl MessageVisualMedia {
        /// The media child of the given type, if any.
        pub(super) fn media_child<T: IsA<gtk::Widget>>(&self) -> Option<T> {
            self.stack.child_by_name(MEDIA_PAGE).and_downcast()
        }

        /// Set the media child.
        ///
        /// Removes the previous media child if one was set.
        fn set_media_child(&self, child: &impl IsA<gtk::Widget>) {
            if let Some(prev_child) = self.stack.child_by_name(MEDIA_PAGE) {
                self.stack.remove(&prev_child);
            }

            self.stack.add_named(child, Some(MEDIA_PAGE));
        }

        /// Set the state of the media.
        fn set_state(&self, state: LoadingState) {
            if self.state.get() == state {
                return;
            }

            match state {
                LoadingState::Loading | LoadingState::Initial => {
                    self.stack.set_visible_child_name("placeholder");
                    self.spinner.set_visible(true);
                    self.error.set_visible(false);
                }
                LoadingState::Ready => {
                    self.stack.set_visible_child_name(MEDIA_PAGE);
                    self.spinner.set_visible(false);
                    self.error.set_visible(false);
                }
                LoadingState::Error => {
                    self.spinner.set_visible(false);
                    self.error.set_visible(true);
                }
            }

            self.state.set(state);
            self.obj().notify_state();
        }

        /// Update the state of the animated paintable, if any.
        fn update_animated_paintable_state(&self) {
            self.paintable_animation_ref.take();

            let Some(paintable) = self
                .media_child::<gtk::Picture>()
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
        fn set_compact(&self, compact: bool) {
            if self.compact.get() == compact {
                return;
            }

            self.compact.set(compact);

            if compact {
                self.overlay.add_css_class("compact");
            } else {
                self.overlay.remove_css_class("compact");
            }

            self.obj().notify_compact();
        }

        /// Build the content for the given media message.
        pub(super) fn build(
            &self,
            media_message: VisualMediaMessage,
            session: &Session,
            format: ContentFormat,
        ) {
            self.file.take();
            self.dimensions.set(media_message.dimensions());

            let compact = matches!(format, ContentFormat::Compact | ContentFormat::Ellipsized);
            self.set_compact(compact);

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
            self.obj()
                .update_property(&[gtk::accessible::Property::Label(&accessible_label)]);

            self.set_state(LoadingState::Loading);

            let client = session.client();
            spawn!(
                glib::Priority::LOW,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        match &media_message {
                            VisualMediaMessage::Image(_) | VisualMediaMessage::Sticker(_) => {
                                imp.build_image(&media_message, client).await;
                            }
                            VisualMediaMessage::Video(_) => {
                                imp.build_video(media_message, &client).await;
                            }
                        }

                        imp.update_animated_paintable_state();
                    }
                )
            );
        }

        /// Build the content for the image in the given media message.
        async fn build_image(&self, media_message: &VisualMediaMessage, client: Client) {
            // Disable the copy-image action while the image is loading.
            if matches!(media_message, VisualMediaMessage::Image(_)) {
                self.enable_copy_image_action(false);
            }

            let scale_factor = self.obj().scale_factor();

            let settings = ThumbnailSettings {
                dimensions: FrameDimensions::thumbnail_max_dimensions(scale_factor),
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
                    self.set_error(&error.to_string());
                    return;
                }
            };

            let child = if let Some(child) = self.media_child::<gtk::Picture>() {
                child
            } else {
                let child = gtk::Picture::builder()
                    .content_fit(gtk::ContentFit::ScaleDown)
                    .build();
                self.set_media_child(&child);
                child
            };
            child.set_paintable(Some(&gdk::Paintable::from(image)));

            child.set_tooltip_text(Some(&media_message.filename()));
            if matches!(&media_message, VisualMediaMessage::Sticker(_)) {
                self.overlay.remove_css_class("opaque-bg");
            } else {
                self.overlay.add_css_class("opaque-bg");
            }

            self.set_state(LoadingState::Ready);

            // Enable the copy-image action now that the image is loaded.
            if matches!(media_message, VisualMediaMessage::Image(_)) {
                self.enable_copy_image_action(true);
            }
        }

        /// Enable or disable the context menu action to copy the image.
        fn enable_copy_image_action(&self, enable: bool) {
            if self.compact.get() {
                // In its compact form the message does not have actions.
                return;
            }

            if self
                .obj()
                .activate_action(
                    "room-history-row.enable-copy-image",
                    Some(&enable.to_variant()),
                )
                .is_err()
            {
                error!("Could not change state of copy-image action: `room-history-row.enable-copy-image` action not found");
            }
        }

        /// Build the content for the video in the given media message.
        async fn build_video(&self, media_message: VisualMediaMessage, client: &Client) {
            let file = match media_message.into_tmp_file(client).await {
                Ok(file) => file,
                Err(error) => {
                    warn!("Could not retrieve video: {error}");
                    self.set_error(&gettext("Could not retrieve media"));
                    return;
                }
            };

            let child = if let Some(child) = self.media_child::<VideoPlayer>() {
                child
            } else {
                let child = VideoPlayer::new();
                child.connect_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |player| {
                        imp.video_state_changed(player);
                    }
                ));
                self.set_media_child(&child);
                child
            };

            child.set_compact(self.compact.get());
            child.play_video_file(file.as_gfile());
            self.file.replace(Some(file));
        }

        /// Set the given error message for this media.
        fn set_error(&self, message: &str) {
            self.error.set_tooltip_text(Some(message));
            self.set_state(LoadingState::Error);
        }

        /// Handle when the state of the video changed.
        fn video_state_changed(&self, player: &VideoPlayer) {
            match player.state() {
                LoadingState::Initial | LoadingState::Loading => {
                    self.set_state(LoadingState::Loading);
                }
                LoadingState::Ready => self.set_state(LoadingState::Ready),
                LoadingState::Error => {
                    let error = player.error();
                    self.set_error(
                        error
                            .map(|e| e.to_string())
                            .as_deref()
                            .unwrap_or(&gettext("An unexpected error occurred")),
                    );
                }
            }
        }

        /// Handle when the media was clicked.
        #[template_callback]
        fn handle_clicked(&self) {
            let _ = self.obj().activate_action("message-row.show-media", None);
        }
    }
}

glib::wrapper! {
    /// A widget displaying a visual media (image or video) message in the timeline.
    pub struct MessageVisualMedia(ObjectSubclass<imp::MessageVisualMedia>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

impl MessageVisualMedia {
    /// Create a new visual media message.
    pub(crate) fn new() -> Self {
        glib::Object::new()
    }

    /// Display the given visual media message.
    pub(crate) fn set_media_message(
        &self,
        media_message: VisualMediaMessage,
        session: &Session,
        format: ContentFormat,
    ) {
        self.imp().build(media_message, session, format);
    }

    /// Get the texture displayed by this widget, if any.
    pub(crate) fn texture(&self) -> Option<gdk::Texture> {
        let paintable = self
            .imp()
            .media_child::<gtk::Picture>()
            .and_then(|p| p.paintable())?;

        if let Some(paintable) = paintable.downcast_ref::<AnimatedImagePaintable>() {
            paintable.current_texture()
        } else {
            paintable.downcast().ok()
        }
    }
}

impl Default for MessageVisualMedia {
    fn default() -> Self {
        Self::new()
    }
}

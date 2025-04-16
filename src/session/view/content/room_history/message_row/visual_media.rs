use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gdk, glib, glib::clone, CompositeTemplate};
use ruma::api::client::media::get_content_thumbnail::v3::Method;
use tracing::{error, warn};

use super::{content::MessageCacheKey, ContentFormat};
use crate::{
    components::{AnimatedImagePaintable, VideoPlayer},
    gettext_f,
    session::model::Room,
    spawn,
    utils::{
        key_bindings,
        matrix::{VisualMediaMessage, VisualMediaType},
        media::{
            image::{ImageRequestPriority, ThumbnailSettings, THUMBNAIL_MAX_DIMENSIONS},
            FrameDimensions,
        },
        CountedRef, File, LoadingState, TemplateCallbacks,
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
/// The name of the placeholder stack page.
const PLACEHOLDER_PAGE: &str = "placeholder";
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
        preview_instructions: TemplateChild<gtk::Box>,
        #[template_child]
        preview_instructions_icon: TemplateChild<gtk::Image>,
        #[template_child]
        spinner: TemplateChild<adw::Spinner>,
        #[template_child]
        hide_preview_button: TemplateChild<gtk::Button>,
        #[template_child]
        error: TemplateChild<gtk::Image>,
        /// The room where the message was sent.
        room: glib::WeakRef<Room>,
        join_rule_handler: RefCell<Option<glib::SignalHandlerId>>,
        session_settings_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// The visual media message to display.
        media_message: RefCell<Option<VisualMediaMessage>>,
        /// The cache key for the current media message.
        ///
        /// We only try to reload the media if the key changes. This is to avoid
        /// reloading the media when a local echo changes to a remote echo.
        cache_key: RefCell<MessageCacheKey>,
        /// The loading state of the media.
        #[property(get, builder(LoadingState::default()))]
        state: Cell<LoadingState>,
        /// Whether to display this media in a compact format.
        #[property(get)]
        compact: Cell<bool>,
        /// Whether the media can be activated.
        ///
        /// If the media is activatable and it is not using the compact format,
        /// clicking on the media opens the media viewer.
        #[property(get)]
        activatable: Cell<bool>,
        gesture_click: glib::WeakRef<gtk::GestureClick>,
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
            TemplateCallbacks::bind_template_callbacks(klass);

            klass.set_css_name("message-visual-media");
            klass.set_accessible_role(gtk::AccessibleRole::Group);

            klass.install_action("message-visual-media.activate", None, |obj, _, _| {
                obj.imp().activate();
            });
            key_bindings::add_activate_bindings(klass, "message-visual-media.activate");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageVisualMedia {
        fn dispose(&self) {
            self.clear();
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
            let max_for_size = max_size
                .dimension_for_other_orientation(orientation)
                .try_into()
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

                    let (min, nat, ..) =
                        child.measure(orientation, for_size.min(intrinsic_for_size));

                    if nat != 0 {
                        // Limit the returned size to the max.
                        let max = max.try_into().unwrap_or(i32::MAX);

                        return (min.min(max), nat.min(max), -1, -1);
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
            let media_size = self
                .media_message
                .borrow()
                .as_ref()
                .and_then(VisualMediaMessage::dimensions)
                .unwrap_or(FALLBACK_DIMENSIONS);
            let nat = media_size
                .scale_to_fit(wanted_size, gtk::ContentFit::ScaleDown)
                .dimension_for_orientation(orientation)
                .try_into()
                .unwrap_or(i32::MAX);

            (0, nat, -1, -1)
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
        fn set_media_child(&self, child: Option<&impl IsA<gtk::Widget>>) {
            if let Some(prev_child) = self.stack.child_by_name(MEDIA_PAGE) {
                self.stack.remove(&prev_child);
            }

            if let Some(child) = child {
                self.stack.add_named(child, Some(MEDIA_PAGE));
            }
        }

        /// Set the state of the media.
        fn set_state(&self, state: LoadingState) {
            if self.state.get() == state {
                return;
            }

            self.state.set(state);

            self.update_visible_page();
            self.obj().notify_state();
        }

        /// Update the visible page for the current state.
        fn update_visible_page(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };

            let state = self.state.get();

            self.preview_instructions
                .set_visible(state == LoadingState::Initial);
            self.spinner.set_visible(state == LoadingState::Loading);
            self.hide_preview_button.set_visible(
                state == LoadingState::Ready
                    && !session.settings().should_room_show_media_previews(&room),
            );
            self.error.set_visible(state == LoadingState::Error);

            let visible_page = match state {
                LoadingState::Initial | LoadingState::Loading => Some(PLACEHOLDER_PAGE),
                LoadingState::Ready => Some(MEDIA_PAGE),
                LoadingState::Error => None,
            };
            if let Some(visible_page) = visible_page {
                self.stack.set_visible_child_name(visible_page);
            }
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

            let icon_size = if compact {
                gtk::IconSize::Normal
            } else {
                gtk::IconSize::Large
            };
            self.preview_instructions_icon.set_icon_size(icon_size);

            self.update_gesture_click();
            self.obj().notify_compact();
        }

        /// Set whether the media can be activated.
        fn set_activatable(&self, activatable: bool) {
            if self.activatable.get() == activatable {
                return;
            }

            self.activatable.set(activatable);

            self.update_gesture_click();
            self.obj().notify_activatable();
        }

        /// Add or remove the click controller given the current state.
        fn update_gesture_click(&self) {
            let needs_controller = self.activatable.get() && !self.compact.get();
            let gesture_click = self.gesture_click.upgrade();

            if needs_controller && gesture_click.is_none() {
                let gesture_click = gtk::GestureClick::new();

                gesture_click.connect_released(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _, _, _| {
                        imp.activate();
                    }
                ));

                self.gesture_click.set(Some(&gesture_click));
                self.overlay.add_controller(gesture_click);
            } else if let Some(gesture_click) = gesture_click {
                self.gesture_click.set(None);
                self.overlay.remove_controller(&gesture_click);
            }
        }

        /// Set the cache key with the given value.
        ///
        /// Returns `true` if the media should be reloaded.
        fn set_cache_key(&self, key: MessageCacheKey) -> bool {
            let should_reload = self.cache_key.borrow().should_reload(&key);

            self.cache_key.replace(key);

            should_reload
        }

        /// Set the visual media message to display.
        pub(super) fn set_media_message(
            &self,
            media_message: VisualMediaMessage,
            room: &Room,
            format: ContentFormat,
            cache_key: MessageCacheKey,
        ) {
            if !self.set_cache_key(cache_key) {
                // We do not need to reload the media.
                return;
            }

            // Reset the widget.
            self.clear();
            self.set_state(LoadingState::Initial);

            let compact = matches!(format, ContentFormat::Compact | ContentFormat::Ellipsized);
            self.set_compact(compact);

            let Some(session) = room.session() else {
                return;
            };

            let join_rule_handler = room.join_rule().connect_anyone_can_join_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_media();
                }
            ));
            self.join_rule_handler.replace(Some(join_rule_handler));

            let session_settings_handler = session
                .settings()
                .connect_media_previews_enabled_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_media();
                    }
                ));
            self.session_settings_handler
                .replace(Some(session_settings_handler));

            self.room.set(Some(room));
            self.media_message.replace(Some(media_message));

            self.update_accessible_label();
            self.update_preview_instructions_icon();
            self.update_media();
        }

        /// Update the accessible label for the current state.
        fn update_accessible_label(&self) {
            let Some((filename, visual_media_type)) =
                self.media_message.borrow().as_ref().map(|media_message| {
                    (media_message.filename(), media_message.visual_media_type())
                })
            else {
                return;
            };

            let accessible_label = if filename.is_empty() {
                match visual_media_type {
                    VisualMediaType::Image => gettext("Image"),
                    VisualMediaType::Sticker => gettext("Sticker"),
                    VisualMediaType::Video => gettext("Video"),
                }
            } else {
                match visual_media_type {
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
            };
            self.obj()
                .update_property(&[gtk::accessible::Property::Label(&accessible_label)]);
        }

        /// Update the preview instructions icon for the current state.
        fn update_preview_instructions_icon(&self) {
            let Some(content_type) = self
                .media_message
                .borrow()
                .as_ref()
                .map(VisualMediaMessage::content_type)
            else {
                return;
            };

            self.preview_instructions_icon
                .set_icon_name(Some(content_type.icon_name()));
        }

        /// Update the media for the current state.
        fn update_media(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };

            if session.settings().should_room_show_media_previews(&room) {
                // Only load the media if it was not loaded before.
                if self.state.get() == LoadingState::Initial {
                    self.show_media();
                }
            } else {
                self.hide_media();
            }
        }

        /// Hide the media.
        #[template_callback]
        fn hide_media(&self) {
            self.set_state(LoadingState::Initial);
            self.set_media_child(None::<&gtk::Widget>);
            self.file.take();
            self.set_activatable(true);
        }

        /// Show the media.
        fn show_media(&self) {
            let Some(media_message) = self.media_message.borrow().clone() else {
                return;
            };

            self.set_state(LoadingState::Loading);

            let activatable = matches!(
                media_message,
                VisualMediaMessage::Image(_) | VisualMediaMessage::Video(_)
            );
            self.set_activatable(activatable);

            spawn!(
                glib::Priority::LOW,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        match &media_message {
                            VisualMediaMessage::Image(_) | VisualMediaMessage::Sticker(_) => {
                                imp.build_image(&media_message).await;
                            }
                            VisualMediaMessage::Video(_) => {
                                imp.build_video(media_message).await;
                            }
                        }

                        imp.update_animated_paintable_state();
                    }
                )
            );
        }

        /// Build the content for the image in the given media message.
        async fn build_image(&self, media_message: &VisualMediaMessage) {
            let Some(client) = self
                .room
                .upgrade()
                .and_then(|room| room.session())
                .map(|session| session.client())
            else {
                return;
            };

            if self.state.get() != LoadingState::Loading {
                // Something occurred after the task was spawned, cancel the task.
                return;
            }

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

            if self.state.get() != LoadingState::Loading {
                // Something occurred while the image was loading, cancel the task.
                return;
            }

            let child = if let Some(child) = self.media_child::<gtk::Picture>() {
                child
            } else {
                let child = gtk::Picture::builder()
                    .content_fit(gtk::ContentFit::ScaleDown)
                    .build();
                self.set_media_child(Some(&child));
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
                .activate_action("event-row.enable-copy-image", Some(&enable.to_variant()))
                .is_err()
            {
                error!("Could not change state of copy-image action: `event-row.enable-copy-image` action not found");
            }
        }

        /// Build the content for the video in the given media message.
        async fn build_video(&self, media_message: VisualMediaMessage) {
            let Some(client) = self
                .room
                .upgrade()
                .and_then(|room| room.session())
                .map(|session| session.client())
            else {
                return;
            };

            if self.state.get() != LoadingState::Loading {
                // Something occurred after the task was spawned, cancel the task.
                return;
            }

            let file = match media_message.into_tmp_file(&client).await {
                Ok(file) => file,
                Err(error) => {
                    warn!("Could not retrieve video: {error}");
                    self.set_error(&gettext("Could not retrieve media"));
                    return;
                }
            };

            if self.state.get() != LoadingState::Loading {
                // Something occurred while the video was loading, cancel the task.
                return;
            }

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
                self.set_media_child(Some(&child));
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

        /// Reset the state of this widget.
        fn clear(&self) {
            self.file.take();

            if let Some(room) = self.room.upgrade() {
                if let Some(handler) = self.join_rule_handler.take() {
                    room.join_rule().disconnect(handler);
                }

                if let Some(handler) = self.session_settings_handler.take() {
                    if let Some(session) = room.session() {
                        session.settings().disconnect(handler);
                    }
                }
            }
        }

        /// Handle when the widget is activated.
        fn activate(&self) {
            if self.state.get() == LoadingState::Initial {
                self.show_media();
            } else if self
                .obj()
                .activate_action("message-row.show-media", None)
                .is_err()
            {
                error!("Could not activate `message-row.show-media` action");
            }
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

    /// Set the visual media message to display.
    pub(crate) fn set_media_message(
        &self,
        media_message: VisualMediaMessage,
        room: &Room,
        format: ContentFormat,
        cache_key: MessageCacheKey,
    ) {
        self.imp()
            .set_media_message(media_message, room, format, cache_key);
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

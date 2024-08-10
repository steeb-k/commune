use gtk::{gdk, glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use matrix_sdk::media::{MediaEventContent, MediaThumbnailSettings};
use ruma::{
    api::client::media::get_content_thumbnail::v3::Method,
    events::room::message::{ImageMessageEventContent, VideoMessageEventContent},
};
use tracing::warn;

use super::{HistoryViewerEvent, VisualMediaHistoryViewer};
use crate::{
    spawn, spawn_tokio,
    utils::{add_activate_binding_action, matrix::MediaMessage},
};

/// The default size requested by a thumbnail.
const THUMBNAIL_SIZE: u32 = 300;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/history_viewer/visual_media_item.ui"
    )]
    #[properties(wrapper_type = super::VisualMediaItem)]
    pub struct VisualMediaItem {
        /// The file event.
        #[property(get, set = Self::set_event, explicit_notify, nullable)]
        pub event: RefCell<Option<HistoryViewerEvent>>,
        pub overlay_icon: RefCell<Option<gtk::Image>>,
        #[template_child]
        pub overlay: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub picture: TemplateChild<gtk::Picture>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VisualMediaItem {
        const NAME: &'static str = "ContentVisualMediaHistoryViewerItem";
        type Type = super::VisualMediaItem;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("visual-media-history-viewer-item");

            klass.install_action("visual-media-item.activate", None, |obj, _, _| {
                obj.activate();
            });

            add_activate_binding_action(klass, "visual-media-item.activate");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for VisualMediaItem {
        fn dispose(&self) {
            self.overlay.unparent();
        }
    }

    impl WidgetImpl for VisualMediaItem {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            // Keep the widget squared
            let (min, ..) = self.overlay.measure(orientation, for_size);
            (min, for_size.max(min), -1, -1)
        }

        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.overlay.allocate(width, height, baseline, None);
        }
    }

    impl VisualMediaItem {
        /// Set the media event.
        fn set_event(&self, event: Option<HistoryViewerEvent>) {
            if *self.event.borrow() == event {
                return;
            }

            self.event.replace(event);
            self.update();

            self.obj().notify_event();
        }

        /// Update this item for the current state.
        fn update(&self) {
            let Some(message_content) = self.event.borrow().as_ref().map(|e| e.message_content())
            else {
                return;
            };

            let filename = message_content.filename();
            match message_content {
                MediaMessage::Image(content) => {
                    self.show_image(content, filename);
                }
                MediaMessage::Video(content) => {
                    self.show_video(content, filename);
                }
                _ => {}
            }
        }

        /// Show the given image with this item.
        fn show_image(&self, image: ImageMessageEventContent, filename: String) {
            if let Some(icon) = self.overlay_icon.take() {
                self.overlay.remove_overlay(&icon);
            }

            self.obj().set_tooltip_text(Some(&filename));

            self.load_thumbnail(image);
        }

        /// Show the given video with this item.
        fn show_video(&self, video: VideoMessageEventContent, filename: String) {
            if self.overlay_icon.borrow().is_none() {
                let icon = gtk::Image::builder()
                    .icon_name("media-playback-start-symbolic")
                    .css_classes(vec!["osd".to_string()])
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .accessible_role(gtk::AccessibleRole::Presentation)
                    .build();

                self.overlay.add_overlay(&icon);
                self.overlay_icon.replace(Some(icon));
            }

            self.obj().set_tooltip_text(Some(&filename));

            self.load_thumbnail(video);
        }

        /// Load the thumbnail for the given media event content.
        fn load_thumbnail<C>(&self, content: C)
        where
            C: MediaEventContent + Send + Sync + Clone + 'static,
        {
            let Some(session) = self
                .event
                .borrow()
                .as_ref()
                .and_then(|e| e.room())
                .and_then(|r| r.session())
            else {
                return;
            };

            let media = session.client().media();
            let handle = spawn_tokio!(async move {
                let thumbnail = if content.thumbnail_source().is_some() {
                    media
                        .get_thumbnail(
                            &content,
                            MediaThumbnailSettings::new(
                                Method::Scale,
                                THUMBNAIL_SIZE.into(),
                                THUMBNAIL_SIZE.into(),
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
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        match handle.await.unwrap() {
                            Ok(Some(data)) => {
                                match gdk::Texture::from_bytes(&glib::Bytes::from(&data)) {
                                    Ok(texture) => {
                                        imp.picture.set_paintable(Some(&texture));
                                    }
                                    Err(error) => {
                                        warn!("Image file not supported: {}", error);
                                    }
                                }
                            }
                            Ok(None) => {
                                warn!("Could not retrieve invalid media file");
                            }
                            Err(error) => {
                                warn!("Could not retrieve media file: {}", error);
                            }
                        }
                    }
                )
            );
        }
    }
}

glib::wrapper! {
    /// A row presenting a visual media (image or video) event.
    pub struct VisualMediaItem(ObjectSubclass<imp::VisualMediaItem>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl VisualMediaItem {
    /// Construct a new empty `VisualMediaItem`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The item was activated.
    #[template_callback]
    fn activate(&self) {
        let media_history_viewer = self
            .ancestor(VisualMediaHistoryViewer::static_type())
            .and_downcast::<VisualMediaHistoryViewer>()
            .unwrap();
        media_history_viewer.show_media(self);
    }
}

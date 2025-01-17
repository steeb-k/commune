use gtk::{gdk, glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use ruma::api::client::media::get_content_thumbnail::v3::Method;

use super::{HistoryViewerEvent, VisualMediaHistoryViewer};
use crate::{
    spawn,
    utils::{
        key_bindings,
        matrix::VisualMediaMessage,
        media::{
            image::{ImageRequestPriority, ThumbnailSettings},
            FrameDimensions,
        },
    },
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

            key_bindings::add_activate_bindings(klass, "visual-media-item.activate");
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
            let Some(media_message) = self
                .event
                .borrow()
                .as_ref()
                .and_then(HistoryViewerEvent::visual_media_message)
            else {
                return;
            };

            let show_overlay = matches!(media_message, VisualMediaMessage::Video(_));
            self.show_video_overlay(show_overlay);

            self.obj().set_tooltip_text(Some(&media_message.filename()));

            spawn!(
                glib::Priority::LOW,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.load_thumbnail(media_message).await;
                    }
                )
            );
        }

        /// Set whether to show the video overlay.
        fn show_video_overlay(&self, show: bool) {
            if show && self.overlay_icon.borrow().is_none() {
                let icon = gtk::Image::builder()
                    .icon_name("media-playback-start-symbolic")
                    .css_classes(vec!["osd".to_string()])
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .accessible_role(gtk::AccessibleRole::Presentation)
                    .build();

                self.overlay.add_overlay(&icon);
                self.overlay_icon.replace(Some(icon));
            } else if !show {
                if let Some(icon) = self.overlay_icon.take() {
                    self.overlay.remove_overlay(&icon);
                }
            }
        }

        /// Load the thumbnail for the given media message.
        async fn load_thumbnail(&self, media_message: VisualMediaMessage) {
            let Some(session) = self
                .event
                .borrow()
                .as_ref()
                .and_then(HistoryViewerEvent::room)
                .and_then(|r| r.session())
            else {
                return;
            };

            let client = session.client();

            let scale_factor = u32::try_from(self.obj().scale_factor()).unwrap_or(1);
            let size = THUMBNAIL_SIZE * scale_factor;
            let dimensions = FrameDimensions {
                width: size,
                height: size,
            };

            let settings = ThumbnailSettings {
                dimensions,
                method: Method::Scale,
                animated: false,
                prefer_thumbnail: false,
            };

            if let Ok(Some(image)) = media_message
                .thumbnail(client, settings, ImageRequestPriority::Default)
                .await
            {
                self.picture
                    .set_paintable(Some(&gdk::Paintable::from(image)));
            }
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

use gettextrs::gettext;
use gtk::{gdk, glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use matrix_sdk::{
    media::{MediaEventContent, MediaThumbnailSize},
    ruma::{
        api::client::media::get_content_thumbnail::v3::Method,
        events::room::message::{ImageMessageEventContent, MessageType, VideoMessageEventContent},
        uint,
    },
};
use tracing::warn;

use super::{HistoryViewerEvent, MediaHistoryViewer};
use crate::{session::model::Session, spawn, spawn_tokio, utils::add_activate_binding_action};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/history_viewer/media_item.ui"
    )]
    #[properties(wrapper_type = super::MediaItem)]
    pub struct MediaItem {
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
    impl ObjectSubclass for MediaItem {
        const NAME: &'static str = "ContentMediaHistoryViewerItem";
        type Type = super::MediaItem;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("media-history-viewer-item");

            klass.install_action("media-item.activate", None, |obj, _, _| {
                obj.activate();
            });

            add_activate_binding_action(klass, "media-item.activate");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MediaItem {
        fn dispose(&self) {
            self.overlay.unparent();
        }
    }

    impl WidgetImpl for MediaItem {
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

    impl MediaItem {
        /// Set the media event.
        fn set_event(&self, event: Option<HistoryViewerEvent>) {
            if *self.event.borrow() == event {
                return;
            }
            let obj = self.obj();

            if let Some(event) = &event {
                let Some(room) = event.room() else {
                    return;
                };
                let Some(session) = room.session() else {
                    return;
                };
                match event.message_content() {
                    MessageType::Image(content) => {
                        obj.show_image(content, &session);
                    }
                    MessageType::Video(content) => {
                        obj.show_video(content, &session);
                    }
                    _ => {}
                }
            }

            self.event.replace(event);
            obj.notify_event();
        }
    }
}

glib::wrapper! {
    /// A row presenting a media (image or video) event.
    pub struct MediaItem(ObjectSubclass<imp::MediaItem>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl MediaItem {
    fn show_image(&self, image: ImageMessageEventContent, session: &Session) {
        let imp = self.imp();

        if let Some(icon) = imp.overlay_icon.take() {
            imp.overlay.remove_overlay(&icon);
        }

        let filename = Some(image.body.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| gettext("Unnamed image"));

        self.set_tooltip_text(Some(&filename));

        self.load_thumbnail(image, session);
    }

    fn show_video(&self, video: VideoMessageEventContent, session: &Session) {
        let imp = self.imp();

        if imp.overlay_icon.borrow().is_none() {
            let icon = gtk::Image::builder()
                .icon_name("media-playback-start-symbolic")
                .css_classes(vec!["osd".to_string()])
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .accessible_role(gtk::AccessibleRole::Presentation)
                .build();

            imp.overlay.add_overlay(&icon);
            imp.overlay_icon.replace(Some(icon));
        }

        let filename = Some(video.body.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| gettext("Unnamed video"));

        self.set_tooltip_text(Some(&filename));

        self.load_thumbnail(video, session);
    }

    fn load_thumbnail<C>(&self, content: C, session: &Session)
    where
        C: MediaEventContent + Send + Sync + Clone + 'static,
    {
        let media = session.client().media();
        let handle = spawn_tokio!(async move {
            let thumbnail = if content.thumbnail_source().is_some() {
                media
                    .get_thumbnail(
                        &content,
                        MediaThumbnailSize {
                            method: Method::Scale,
                            width: uint!(300),
                            height: uint!(300),
                        },
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
            clone!(@weak self as obj => async move {
                let imp = obj.imp();

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
            })
        );
    }

    #[template_callback]
    fn activate(&self) {
        let media_history_viewer = self
            .ancestor(MediaHistoryViewer::static_type())
            .and_downcast::<MediaHistoryViewer>()
            .unwrap();
        media_history_viewer.show_media(self);
    }
}

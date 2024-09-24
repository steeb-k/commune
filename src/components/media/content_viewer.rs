use adw::{prelude::*, subclass::prelude::*};
use geo_uri::GeoUri;
use gettextrs::gettext;
use gtk::{gdk, gio, glib, glib::clone, CompositeTemplate};
use tracing::warn;

use super::{AnimatedImagePaintable, AudioPlayer, LocationViewer};
use crate::{
    components::ContextMenuBin,
    prelude::*,
    spawn,
    utils::{media::image::load_image, CountedRef},
};

#[derive(Debug, Default, Clone, Copy)]
pub enum ContentType {
    Image,
    Audio,
    Video,
    #[default]
    Unknown,
}

impl ContentType {
    pub fn icon_name(&self) -> &'static str {
        match self {
            ContentType::Image => "image-symbolic",
            ContentType::Audio => "audio-symbolic",
            ContentType::Video => "video-symbolic",
            ContentType::Unknown => "document-symbolic",
        }
    }
}

impl From<&str> for ContentType {
    fn from(string: &str) -> Self {
        match string {
            "image" => Self::Image,
            "audio" => Self::Audio,
            "video" => Self::Video,
            _ => Self::Unknown,
        }
    }
}

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/media/content_viewer.ui")]
    #[properties(wrapper_type = super::MediaContentViewer)]
    pub struct MediaContentViewer {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub viewer: TemplateChild<adw::Bin>,
        #[template_child]
        pub fallback: TemplateChild<adw::StatusPage>,
        /// Whether to play the media content automatically.
        #[property(get, construct_only)]
        pub autoplay: Cell<bool>,
        paintable_animation_ref: RefCell<Option<CountedRef>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MediaContentViewer {
        const NAME: &'static str = "MediaContentViewer";
        type Type = super::MediaContentViewer;
        type ParentType = ContextMenuBin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_css_name("media-content-viewer");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MediaContentViewer {
        fn constructed(&self) {
            self.parent_constructed();

            self.viewer.connect_map(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_animated_paintable_state();
                }
            ));
            self.viewer.connect_unmap(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_animated_paintable_state();
                }
            ));
        }
    }

    impl WidgetImpl for MediaContentViewer {}
    impl ContextMenuBinImpl for MediaContentViewer {}

    impl MediaContentViewer {
        /// Update the state of the animated paintable, if any.
        pub(super) fn update_animated_paintable_state(&self) {
            self.paintable_animation_ref.take();

            let Some(paintable) = self
                .viewer
                .child()
                .and_downcast::<gtk::Picture>()
                .and_then(|p| p.paintable())
                .and_downcast::<AnimatedImagePaintable>()
            else {
                return;
            };

            if self.viewer.is_mapped() {
                self.paintable_animation_ref
                    .replace(Some(paintable.animation_ref()));
            }
        }
    }
}

glib::wrapper! {
    /// Widget to view any media file.
    pub struct MediaContentViewer(ObjectSubclass<imp::MediaContentViewer>)
        @extends gtk::Widget, ContextMenuBin, @implements gtk::Accessible;
}

impl MediaContentViewer {
    pub fn new(autoplay: bool) -> Self {
        glib::Object::builder()
            .property("autoplay", autoplay)
            .build()
    }

    pub fn stop_playback(&self) {
        if let Some(stream) = self
            .imp()
            .viewer
            .child()
            .and_downcast::<gtk::Video>()
            .and_then(|v| v.media_stream())
        {
            if stream.is_playing() {
                stream.pause();
                stream.seek(0);
            }
        }
    }

    /// Show the loading screen.
    pub fn show_loading(&self) {
        self.imp().stack.set_visible_child_name("loading");
    }

    /// Show the viewer.
    fn show_viewer(&self) {
        self.imp().stack.set_visible_child_name("viewer");
    }

    /// Show the fallback message for the given content type.
    pub fn show_fallback(&self, content_type: ContentType) {
        let imp = self.imp();
        let fallback = &imp.fallback;

        let title = match content_type {
            ContentType::Image => gettext("Image not Viewable"),
            ContentType::Audio => gettext("Audio Clip not Playable"),
            ContentType::Video => gettext("Video not Playable"),
            ContentType::Unknown => gettext("File not Viewable"),
        };
        fallback.set_title(&title);
        fallback.set_icon_name(Some(content_type.icon_name()));

        imp.stack.set_visible_child_name("fallback");
    }

    /// View the given image as bytes.
    ///
    /// If you have an image file, you can also use
    /// [`MediaContentViewer::view_file()`].
    pub fn view_image(&self, image: &impl IsA<gdk::Paintable>) {
        self.show_loading();

        let imp = self.imp();

        let picture = if let Some(picture) = imp.viewer.child().and_downcast::<gtk::Picture>() {
            picture
        } else {
            let picture = gtk::Picture::builder()
                .valign(gtk::Align::Center)
                .halign(gtk::Align::Center)
                .build();
            imp.viewer.set_child(Some(&picture));
            picture
        };

        picture.set_paintable(Some(image));
        imp.update_animated_paintable_state();
        self.show_viewer();
    }

    /// View the given file.
    pub fn view_file(&self, file: gio::File) {
        self.show_loading();

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                obj.view_file_inner(file).await;
            }
        ));
    }

    async fn view_file_inner(&self, file: gio::File) {
        let imp = self.imp();

        let file_info = file
            .query_info_future(
                gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await
            .ok();

        let content_type: ContentType = file_info
            .as_ref()
            .and_then(|info| info.content_type())
            .and_then(|content_type| gio::content_type_get_mime_type(&content_type))
            .and_then(|mime| mime.split('/').next().map(Into::into))
            .unwrap_or_default();

        match content_type {
            ContentType::Image => match load_image(file, None).await {
                Ok(texture) => {
                    self.view_image(&texture);
                    return;
                }
                Err(error) => {
                    warn!("Could not load image from file: {error}");
                }
            },
            ContentType::Audio => {
                let audio = if let Some(audio) = imp.viewer.child().and_downcast::<AudioPlayer>() {
                    audio
                } else {
                    let audio = AudioPlayer::new();
                    audio.add_css_class("toolbar");
                    audio.add_css_class("osd");
                    audio.set_autoplay(self.autoplay());
                    audio.set_valign(gtk::Align::Center);
                    audio.set_halign(gtk::Align::Center);
                    imp.viewer.set_child(Some(&audio));
                    audio
                };

                audio.set_file(Some(&file));
                imp.update_animated_paintable_state();
                self.show_viewer();
                return;
            }
            ContentType::Video => {
                let video = if let Some(video) = imp.viewer.child().and_downcast::<gtk::Video>() {
                    video
                } else {
                    let video = gtk::Video::builder()
                        .autoplay(self.autoplay())
                        .valign(gtk::Align::Center)
                        .halign(gtk::Align::Center)
                        .build();
                    imp.viewer.set_child(Some(&video));
                    video
                };

                video.set_file(Some(&file));
                imp.update_animated_paintable_state();
                self.show_viewer();
                return;
            }
            _ => {}
        }

        self.show_fallback(content_type);
    }

    /// View the given location as a geo URI.
    pub fn view_location(&self, geo_uri: &GeoUri) {
        let imp = self.imp();

        let location = if let Some(location) = imp.viewer.child().and_downcast::<LocationViewer>() {
            location
        } else {
            let location = LocationViewer::new();
            imp.viewer.set_child(Some(&location));
            location
        };

        location.set_location(geo_uri);
        imp.update_animated_paintable_state();
        self.show_viewer();
    }

    /// Get the texture displayed by this widget, if any.
    pub fn texture(&self) -> Option<gdk::Texture> {
        let paintable = self
            .imp()
            .viewer
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

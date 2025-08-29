use adw::{prelude::*, subclass::prelude::*};
use geo_uri::GeoUri;
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use super::{AnimatedImagePaintable, AudioPlayer, LocationViewer};
use crate::{
    components::ContextMenuBin,
    prelude::*,
    utils::{CountedRef, File, media::image::IMAGE_QUEUE},
};

/// The types of content supported by the [`MediaContentViewer`].
#[derive(Debug, Default, Clone, Copy)]
pub enum ContentType {
    /// An image.
    Image,
    /// An audio file.
    Audio,
    /// A video.
    Video,
    /// An other content type.
    ///
    /// These types are not supported and will result in a fallback screen.
    #[default]
    Other,
}

impl ContentType {
    /// The name of the icon to represent this content type.
    pub(crate) fn icon_name(self) -> &'static str {
        match self {
            ContentType::Image => "image-symbolic",
            ContentType::Audio => "audio-symbolic",
            ContentType::Video => "video-symbolic",
            ContentType::Other => "document-symbolic",
        }
    }
}

impl From<&str> for ContentType {
    fn from(string: &str) -> Self {
        match string {
            "image" => Self::Image,
            "audio" => Self::Audio,
            "video" => Self::Video,
            _ => Self::Other,
        }
    }
}

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/media/content_viewer.ui")]
    #[properties(wrapper_type = super::MediaContentViewer)]
    pub struct MediaContentViewer {
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        viewer: TemplateChild<adw::Bin>,
        #[template_child]
        fallback: TemplateChild<adw::StatusPage>,
        /// Whether to play the media content automatically.
        #[property(get, construct_only)]
        autoplay: Cell<bool>,
        /// The current media file.
        file: RefCell<Option<File>>,
        paintable_animation_ref: RefCell<Option<CountedRef>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MediaContentViewer {
        const NAME: &'static str = "MediaContentViewer";
        type Type = super::MediaContentViewer;
        type ParentType = ContextMenuBin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_css_name("media-content-viewer");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MediaContentViewer {}

    impl WidgetImpl for MediaContentViewer {}
    impl ContextMenuBinImpl for MediaContentViewer {}

    #[gtk::template_callbacks]
    impl MediaContentViewer {
        /// Update the visible child.
        pub(super) fn set_visible_child(&self, name: &str) {
            self.stack.set_visible_child_name(name);
        }

        /// The media child of the given type, if any.
        pub(super) fn media_child<T: IsA<gtk::Widget>>(&self) -> Option<T> {
            self.viewer.child().and_downcast()
        }

        /// Show the fallback message for the given content type.
        pub(super) fn show_fallback(&self, content_type: ContentType) {
            self.file.take();

            let title = match content_type {
                ContentType::Image => gettext("Image not Viewable"),
                ContentType::Audio => gettext("Audio Clip not Playable"),
                ContentType::Video => gettext("Video not Playable"),
                ContentType::Other => gettext("File not Viewable"),
            };
            self.fallback.set_title(&title);
            self.fallback.set_icon_name(Some(content_type.icon_name()));

            self.set_visible_child("fallback");
        }

        /// View the given image as bytes.
        ///
        /// If you have an image file, you can also use
        /// [`MediaContentViewer::view_file()`].
        pub(super) fn view_image(&self, image: &gdk::Paintable) {
            self.set_visible_child("loading");
            self.file.take();

            let picture = if let Some(picture) = self.media_child::<gtk::Picture>() {
                picture
            } else {
                let picture = gtk::Picture::builder()
                    .valign(gtk::Align::Center)
                    .halign(gtk::Align::Center)
                    .build();
                self.viewer.set_child(Some(&picture));
                picture
            };

            picture.set_paintable(Some(image));
            self.update_animated_paintable_state();
            self.set_visible_child("viewer");
        }

        /// View the given file.
        pub(super) async fn view_file(&self, file: File, content_type: Option<ContentType>) {
            self.set_visible_child("loading");
            self.file.replace(Some(file.clone()));

            let content_type = if let Some(content_type) = content_type {
                content_type
            } else {
                let file_info = file
                    .as_gfile()
                    .query_info_future(
                        gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
                        gio::FileQueryInfoFlags::NONE,
                        glib::Priority::DEFAULT,
                    )
                    .await
                    .ok();

                file_info
                    .as_ref()
                    .and_then(gio::FileInfo::content_type)
                    .and_then(|content_type| gio::content_type_get_mime_type(&content_type))
                    .and_then(|mime| mime.split('/').next().map(Into::into))
                    .unwrap_or_default()
            };

            match content_type {
                ContentType::Image => {
                    let handle = IMAGE_QUEUE.add_file_request(file, None).await;
                    if let Ok(image) = handle.await {
                        self.view_image(&gdk::Paintable::from(image));
                        return;
                    }
                }
                ContentType::Audio => {
                    let audio = if let Some(audio) = self.media_child::<AudioPlayer>() {
                        audio
                    } else {
                        let audio = AudioPlayer::new();
                        audio.add_css_class("toolbar");
                        audio.add_css_class("osd");
                        audio.set_autoplay(self.autoplay.get());
                        audio.set_valign(gtk::Align::Center);
                        audio.set_halign(gtk::Align::Center);
                        self.viewer.set_child(Some(&audio));
                        audio
                    };

                    audio.set_file(Some(&file.as_gfile()));
                    self.update_animated_paintable_state();
                    self.set_visible_child("viewer");
                    return;
                }
                ContentType::Video => {
                    let video = if let Some(video) = self.media_child::<gtk::Video>() {
                        video
                    } else {
                        let video = gtk::Video::builder()
                            .autoplay(self.autoplay.get())
                            .valign(gtk::Align::Center)
                            .halign(gtk::Align::Center)
                            .build();
                        self.viewer.set_child(Some(&video));
                        video
                    };

                    video.set_file(Some(&file.as_gfile()));
                    self.update_animated_paintable_state();
                    self.set_visible_child("viewer");
                    return;
                }
                // Other types are not supported.
                ContentType::Other => {}
            }

            self.show_fallback(content_type);
        }

        /// View the given location as a geo URI.
        pub(super) fn view_location(&self, geo_uri: &GeoUri) {
            let location = self.viewer.child_or_default::<LocationViewer>();

            location.set_location(geo_uri);
            self.update_animated_paintable_state();
            self.set_visible_child("viewer");
        }

        /// Update the state of the animated paintable, if any.
        #[template_callback]
        fn update_animated_paintable_state(&self) {
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

        /// Stop the playback, if the media is a video.
        pub(super) fn stop_playback(&self) {
            if let Some(stream) = self
                .media_child::<gtk::Video>()
                .and_then(|v| v.media_stream())
                && stream.is_playing()
            {
                stream.pause();
                stream.seek(0);
            }
        }
    }
}

glib::wrapper! {
    /// Widget to view any media file.
    pub struct MediaContentViewer(ObjectSubclass<imp::MediaContentViewer>)
        @extends gtk::Widget, ContextMenuBin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl MediaContentViewer {
    pub fn new(autoplay: bool) -> Self {
        glib::Object::builder()
            .property("autoplay", autoplay)
            .build()
    }

    /// Stop the playback, if the media is a video.
    pub(crate) fn stop_playback(&self) {
        self.imp().stop_playback();
    }

    /// Show the loading screen.
    pub(crate) fn show_loading(&self) {
        self.imp().set_visible_child("loading");
    }

    /// Show the fallback message for the given content type.
    pub(crate) fn show_fallback(&self, content_type: ContentType) {
        self.imp().show_fallback(content_type);
    }

    /// View the given image as bytes.
    ///
    /// If you have an image file, you can also use
    /// [`MediaContentViewer::view_file()`].
    pub(crate) fn view_image(&self, image: &impl IsA<gdk::Paintable>) {
        self.imp().view_image(image.upcast_ref());
    }

    /// View the given file.
    ///
    /// If the content type is not provided, it will be guessed from the file.
    pub(crate) async fn view_file(&self, file: File, content_type: Option<ContentType>) {
        self.imp().view_file(file, content_type).await;
    }

    /// View the given location as a geo URI.
    pub(crate) fn view_location(&self, geo_uri: &GeoUri) {
        self.imp().view_location(geo_uri);
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
            paintable.downcast::<gdk::Texture>().ok()
        }
    }
}

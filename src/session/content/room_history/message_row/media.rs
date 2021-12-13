use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gdk,
    gdk_pixbuf::Pixbuf,
    gio,
    glib::{self, clone},
    subclass::prelude::*,
    CompositeTemplate,
};
use log::warn;
use matrix_sdk::{
    media::{MediaEventContent, MediaThumbnailSize},
    ruma::{
        api::client::r0::media::get_content_thumbnail::Method,
        events::{
            room::message::{ImageMessageEventContent, VideoMessageEventContent},
            sticker::StickerEventContent,
        },
        uint,
    },
};

use crate::{
    components::VideoPlayer,
    session::Session,
    spawn, spawn_tokio,
    utils::{cache_dir, uint_to_i32},
};

const MAX_THUMBNAIL_WIDTH: i32 = 600;
const MAX_THUMBNAIL_HEIGHT: i32 = 400;
const FALLBACK_WIDTH: i32 = 480;
const FALLBACK_HEIGHT: i32 = 360;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "MediaType")]
pub enum MediaType {
    Image = 0,
    Sticker = 1,
    Video = 2,
}

impl Default for MediaType {
    fn default() -> Self {
        Self::Image
    }
}

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "MediaState")]
pub enum MediaState {
    Initial = 0,
    Loading = 1,
    Ready = 2,
    Error = 3,
}

impl Default for MediaState {
    fn default() -> Self {
        Self::Initial
    }
}

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content-message-media.ui")]
    pub struct MessageMedia {
        /// The type of media previewed with this image.
        pub media_type: Cell<MediaType>,
        /// The intended display width of the full image.
        pub width: Cell<i32>,
        /// The intended display height of the full image.
        pub height: Cell<i32>,
        /// The "body" of the image to show as a tooltip. Only used for stickers.
        pub body: RefCell<Option<String>>,
        /// The state of the media.
        pub state: Cell<MediaState>,
        #[template_child]
        pub media: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub overlay_error: TemplateChild<gtk::Image>,
        #[template_child]
        pub overlay_spinner: TemplateChild<gtk::Spinner>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageMedia {
        const NAME: &'static str = "ContentMessageMedia";
        type Type = super::MessageMedia;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MessageMedia {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_enum(
                        "media-type",
                        "Media Type",
                        "The type of media previewed",
                        MediaType::static_type(),
                        MediaType::default() as i32,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_int(
                        "width",
                        "Width",
                        "The intended display width of the media",
                        -1,
                        i32::MAX,
                        -1,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_int(
                        "height",
                        "Height",
                        "The intended display height of the media",
                        -1,
                        i32::MAX,
                        -1,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_string(
                        "body",
                        "Body",
                        "The 'body' of the media to show as a tooltip",
                        None,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_enum(
                        "state",
                        "State",
                        "The state of the media",
                        MediaState::static_type(),
                        MediaState::default() as i32,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "media-type" => {
                    self.media_type.set(value.get().unwrap());
                }
                "width" => {
                    self.width.set(value.get().unwrap());
                }
                "height" => {
                    self.height.set(value.get().unwrap());
                }
                "body" => {
                    self.body.replace(value.get().unwrap());
                }
                "state" => {
                    obj.set_state(value.get().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "media-type" => obj.media_type().to_value(),
                "width" => self.width.get().to_value(),
                "height" => self.height.get().to_value(),
                "body" => obj.body().to_value(),
                "state" => obj.state().to_value(),
                _ => unimplemented!(),
            }
        }

        fn dispose(&self, _obj: &Self::Type) {
            self.media.unparent();
        }
    }

    impl WidgetImpl for MessageMedia {
        fn measure(
            &self,
            _obj: &Self::Type,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let original_width = self.width.get();
            let original_height = self.height.get();

            let (original, max, fallback, original_other, max_other) =
                if orientation == gtk::Orientation::Vertical {
                    (
                        original_height,
                        MAX_THUMBNAIL_HEIGHT,
                        FALLBACK_HEIGHT,
                        original_width,
                        MAX_THUMBNAIL_WIDTH,
                    )
                } else {
                    (
                        original_width,
                        MAX_THUMBNAIL_WIDTH,
                        FALLBACK_WIDTH,
                        original_height,
                        MAX_THUMBNAIL_HEIGHT,
                    )
                };

            // Limit other side to max size.
            let other = for_size.min(max_other);

            let nat = if original > 0 && original > 0 {
                // We don't want the paintable to be upscaled.
                let other = other.min(original_other);
                other * original / original_other
            } else if let Some(child) = self.media.child() {
                // Get the natural size of the data.
                child.measure(orientation, other).1
            } else {
                fallback
            };

            // Limit this size to 400 pixels.
            let size = nat.min(max);
            (0, size, -1, -1)
        }

        fn request_mode(&self, _obj: &Self::Type) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn size_allocate(&self, _obj: &Self::Type, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.media.child() {
                // We need to allocate just enough width to the child so it doesn't expand.
                let original_width = self.width.get();
                let original_height = self.height.get();
                let width = if original_height > 0 && original_width > 0 {
                    height * original_width / original_height
                } else {
                    // Get the natural width of the image data.
                    child.measure(gtk::Orientation::Horizontal, height).1
                };

                self.media.allocate(width, height, baseline, None);
            } else {
                self.media.allocate(width, height, baseline, None)
            }
        }
    }
}

glib::wrapper! {
    /// A widget displaying a media message in the timeline.
    pub struct MessageMedia(ObjectSubclass<imp::MessageMedia>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

impl MessageMedia {
    pub fn image(image: ImageMessageEventContent, session: &Session) -> Self {
        let info = image.info.as_deref();
        let width = uint_to_i32(info.and_then(|info| info.width));
        let height = uint_to_i32(info.and_then(|info| info.height));

        let self_: Self = glib::Object::new(&[("width", &width), ("height", &height)])
            .expect("Failed to create MessageMedia");
        self_.build(image, session);
        self_
    }

    pub fn sticker(sticker: StickerEventContent, session: &Session) -> Self {
        let info = &sticker.info;
        let width = uint_to_i32(info.width);
        let height = uint_to_i32(info.height);

        let self_: Self = glib::Object::new(&[
            ("media-type", &MediaType::Sticker),
            ("width", &width),
            ("height", &height),
            ("body", &sticker.body),
        ])
        .expect("Failed to create MessageMedia");
        self_.build(sticker, session);
        self_
    }

    pub fn video(video: VideoMessageEventContent, session: &Session) -> Self {
        let info = &video.info.as_deref();
        let width = uint_to_i32(info.and_then(|info| info.width));
        let height = uint_to_i32(info.and_then(|info| info.height));

        let self_: Self = glib::Object::new(&[
            ("media-type", &MediaType::Video),
            ("width", &width),
            ("height", &height),
            ("body", &video.body),
        ])
        .expect("Failed to create MessageMedia");
        self_.build(video, session);
        self_
    }

    pub fn media_type(&self) -> MediaType {
        let priv_ = imp::MessageMedia::from_instance(self);
        priv_.media_type.get()
    }

    pub fn body(&self) -> Option<String> {
        let priv_ = imp::MessageMedia::from_instance(self);
        priv_.body.borrow().clone()
    }

    pub fn state(&self) -> MediaState {
        let priv_ = imp::MessageMedia::from_instance(self);
        priv_.state.get()
    }

    fn set_state(&self, state: MediaState) {
        let priv_ = imp::MessageMedia::from_instance(self);

        if self.state() == state {
            return;
        }

        match state {
            MediaState::Loading | MediaState::Initial => {
                priv_.overlay_spinner.set_visible(true);
                priv_.overlay_error.set_visible(false);
            }
            MediaState::Ready => {
                priv_.overlay_spinner.set_visible(false);
                priv_.overlay_error.set_visible(false);
            }
            MediaState::Error => {
                priv_.overlay_spinner.set_visible(false);
                priv_.overlay_error.set_visible(true);
            }
        }

        priv_.state.set(state);
        self.notify("state");
    }

    fn build<C>(&self, content: C, session: &Session)
    where
        C: MediaEventContent + Send + Sync + Clone + 'static,
    {
        self.set_state(MediaState::Loading);

        let media_type = self.media_type();
        let client = session.client();
        let handle = spawn_tokio!(async move {
            let thumbnail = if media_type != MediaType::Video && content.thumbnail().is_some() {
                client
                    .get_thumbnail(
                        content.clone(),
                        MediaThumbnailSize {
                            method: Method::Scale,
                            width: uint!(320),
                            height: uint!(240),
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
                client.get_file(content, true).await
            }
        });

        spawn!(
            glib::PRIORITY_LOW,
            clone!(@weak self as obj => async move {
                let priv_ = imp::MessageMedia::from_instance(&obj);

                match handle.await.unwrap() {
                    Ok(Some(data)) => {
                        let child: gtk::Widget = match media_type {
                            MediaType::Image | MediaType::Sticker => {
                                let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(&data));
                                let texture = Pixbuf::from_stream(&stream, gio::NONE_CANCELLABLE)
                                    .ok()
                                    .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf));
                                let child = gtk::Picture::for_paintable(texture.as_ref());

                                if media_type == MediaType::Sticker {
                                    child.set_tooltip_text(obj.body().as_deref());
                                    priv_.media.remove_css_class("thumbnail");
                                }
                                child.upcast()
                            }
                            MediaType::Video => {
                                // The GStreamer backend of GtkVideo doesn't work with input streams so
                                // we need to store the file.
                                // See: https://gitlab.gnome.org/GNOME/gtk/-/issues/4062
                                let mut path = cache_dir();
                                path.push(obj.body().unwrap());
                                let file = gio::File::for_path(path);
                                file.replace_contents(
                                    &data,
                                    None,
                                    false,
                                    gio::FileCreateFlags::REPLACE_DESTINATION,
                                    gio::NONE_CANCELLABLE,
                                )
                                .unwrap();
                                let media_file = gtk::MediaFile::for_file(&file);
                                media_file.set_muted(true);
                                media_file.connect_prepared_notify(|media_file| media_file.play());

                                VideoPlayer::new(&media_file).upcast()
                            }
                        };

                        priv_.media.set_child(Some(&child));
                        obj.set_state(MediaState::Ready);
                    }
                    Ok(None) => {
                        warn!("Could not retrieve invalid media file");
                        priv_.overlay_error.set_tooltip_text(Some(&gettext("Could not retrieve media")));
                        obj.set_state(MediaState::Error);
                    }
                    Err(error) => {
                        warn!("Could not retrieve media file: {}", error);
                        priv_.overlay_error.set_tooltip_text(Some(&gettext("Could not retrieve media")));
                        obj.set_state(MediaState::Error);
                    }
                }
            })
        );
    }
}

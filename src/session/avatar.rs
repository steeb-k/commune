use std::path::Path;

use gtk::{gdk, gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use log::{debug, error, info};
use matrix_sdk::room::Room as MatrixRoom;
use matrix_sdk::ruma::events::room::avatar::RoomAvatarEventContent;
use matrix_sdk::ruma::events::AnyStateEventContent;
use matrix_sdk::Client;
use matrix_sdk::{
    media::{MediaFormat, MediaRequest, MediaThumbnailSize, MediaType},
    ruma::{api::client::r0::media::get_content_thumbnail::Method, identifiers::MxcUri},
};

use crate::{spawn, spawn_tokio};

use crate::session::Session;

mod imp {
    use super::*;
    use glib::object::WeakRef;
    use once_cell::{sync::Lazy, unsync::OnceCell};
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default)]
    pub struct Avatar {
        pub image: RefCell<Option<gdk::Paintable>>,
        pub needed_size: Cell<i32>,
        pub url: RefCell<Option<Box<MxcUri>>>,
        pub display_name: RefCell<Option<String>>,
        pub session: OnceCell<WeakRef<Session>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Avatar {
        const NAME: &'static str = "Avatar";
        type Type = super::Avatar;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for Avatar {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::new(
                        "image",
                        "Image",
                        "The user defined image if any",
                        gdk::Paintable::static_type(),
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpecInt::new(
                        "needed-size",
                        "Needed Size",
                        "The size needed of the user defined image. If -1 no image will be loaded",
                        -1,
                        i32::MAX,
                        -1,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpecString::new(
                        "url",
                        "Url",
                        "The url of the Avatar",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpecString::new(
                        "display-name",
                        "Display Name",
                        "The display name used for this avatar",
                        None,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpecObject::new(
                        "session",
                        "Session",
                        "The session",
                        Session::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
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
                "needed-size" => obj.set_needed_size(value.get().unwrap()),
                "url" => obj.set_url(value.get::<Option<&str>>().unwrap().map(Into::into)),
                "session" => self
                    .session
                    .set(value.get::<Session>().unwrap().downgrade())
                    .unwrap(),
                "display-name" => {
                    let _ = obj.set_display_name(value.get().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "image" => obj.image().to_value(),
                "needed-size" => obj.needed_size().to_value(),
                "url" => obj.url().map_or_else(
                    || {
                        let none: Option<&str> = None;
                        none.to_value()
                    },
                    |url| url.as_str().to_value(),
                ),
                "display-name" => obj.display_name().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    /// Object holding information about a User’s or Room’s `Avatar`.
    pub struct Avatar(ObjectSubclass<imp::Avatar>);
}

impl Avatar {
    pub fn new(session: &Session, url: Option<&MxcUri>) -> Self {
        glib::Object::new(&[
            ("session", session),
            ("url", &url.map(|url| url.to_string())),
        ])
        .expect("Failed to create Avatar")
    }

    fn session(&self) -> Session {
        let priv_ = imp::Avatar::from_instance(self);
        priv_.session.get().unwrap().upgrade().unwrap()
    }

    pub fn image(&self) -> Option<gdk::Paintable> {
        let priv_ = imp::Avatar::from_instance(self);
        priv_.image.borrow().clone()
    }

    fn set_image_data(&self, data: Option<Vec<u8>>) {
        let priv_ = imp::Avatar::from_instance(self);

        let image = data
            .and_then(|data| gdk::Texture::from_bytes(&glib::Bytes::from(&data)).ok())
            .map(|texture| texture.upcast());
        priv_.image.replace(image);
        self.notify("image");
    }

    fn load(&self) {
        // Don't do anything here if we don't need the avatar
        if self.needed_size() < 0 {
            return;
        }

        let needed_size = self.needed_size() as u16;

        if let Some(url) = self.url() {
            let client = self.session().client();
            let request = MediaRequest {
                media_type: MediaType::Uri(url),
                format: MediaFormat::Thumbnail(MediaThumbnailSize {
                    width: needed_size.into(),
                    height: needed_size.into(),
                    method: Method::Scale,
                }),
            };
            let handle =
                spawn_tokio!(async move { client.get_media_content(&request, true).await });

            spawn!(
                glib::PRIORITY_LOW,
                clone!(@weak self as obj => async move {
                    match handle.await.unwrap() {
                        Ok(data) => obj.set_image_data(Some(data)),
                        Err(error) => error!("Couldn’t fetch avatar: {}", error),
                    };
                })
            );
        }
    }

    pub fn set_display_name(&self, display_name: Option<String>) {
        let priv_ = imp::Avatar::from_instance(self);
        if self.display_name() == display_name {
            return;
        }

        priv_.display_name.replace(display_name);

        self.notify("display-name");
    }

    pub fn display_name(&self) -> Option<String> {
        let priv_ = imp::Avatar::from_instance(self);
        priv_.display_name.borrow().clone()
    }

    /// Set the needed size.
    /// Only the biggest size will be stored
    pub fn set_needed_size(&self, size: i32) {
        let priv_ = imp::Avatar::from_instance(self);

        if priv_.needed_size.get() < size {
            priv_.needed_size.set(size);

            if priv_.needed_size.get() > -1 {
                self.load();
            }
        }

        self.notify("needed-size");
    }

    /// Get the biggest needed size
    pub fn needed_size(&self) -> i32 {
        let priv_ = imp::Avatar::from_instance(self);
        priv_.needed_size.get()
    }

    pub fn set_url(&self, url: Option<Box<MxcUri>>) {
        let priv_ = imp::Avatar::from_instance(self);

        if priv_.url.borrow().as_ref() == url.as_ref() {
            return;
        }

        let has_url = url.is_some();
        priv_.url.replace(url);

        if has_url {
            self.load();
        } else {
            self.set_image_data(None);
        }

        self.notify("url");
    }

    pub fn url(&self) -> Option<Box<MxcUri>> {
        let priv_ = imp::Avatar::from_instance(self);
        priv_.url.borrow().to_owned()
    }
}

/// Uploads the given file and sets the room avatar.
///
/// Removes the avatar if `filename` is None.
pub async fn update_room_avatar_from_file<P>(
    matrix_client: &Client,
    matrix_room: &MatrixRoom,
    filename: Option<&P>,
) -> Result<Option<Box<MxcUri>>, AvatarError>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    let joined_room = match matrix_room {
        MatrixRoom::Joined(joined_room) => joined_room,
        _ => return Err(AvatarError::NotAMember),
    };

    let mut content = RoomAvatarEventContent::new();

    let uri = if let Some(filename) = filename {
        Some(upload_avatar(matrix_client, filename).await?)
    } else {
        debug!("Removing room avatar");
        None
    };
    content.url = uri.clone();

    joined_room
        .send_state_event(AnyStateEventContent::RoomAvatar(content), "")
        .await?;
    Ok(uri)
}

/// Returns the URI of the room avatar after uploading it.
async fn upload_avatar<P>(matrix_client: &Client, filename: &P) -> Result<Box<MxcUri>, AvatarError>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    debug!("Getting mime type of file {:?}", filename);
    let image = tokio::fs::read(filename).await?;
    let content_type = gio::content_type_guess(Option::<String>::None, &image)
        .0
        .to_string();

    info!("Uploading avatar from file {:?}", filename);
    // TODO: Use blurhash
    let response = matrix_client
        .upload(&content_type.parse()?, &mut image.as_slice())
        .await?;
    Ok(response.content_uri)
}

/// Error occuring when updating an avatar.
#[derive(Debug)]
pub enum AvatarError {
    Filesystem(std::io::Error),
    Upload(matrix_sdk::Error),
    NotAMember,
    UnknownFiletype(mime::FromStrError),
}

impl std::fmt::Display for AvatarError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use AvatarError::*;
        match self {
            Filesystem(e) => write!(f, "Could not open room avatar file: {}", e),
            Upload(e) => write!(f, "Could not upload room avatar: {}", e),
            NotAMember => write!(f, "Room avatar can’t be changed when not a member."),
            UnknownFiletype(e) => write!(f, "Room avatar file has an unknown filetype: {}", e),
        }
    }
}

impl From<std::io::Error> for AvatarError {
    fn from(err: std::io::Error) -> Self {
        Self::Filesystem(err)
    }
}

impl From<matrix_sdk::Error> for AvatarError {
    fn from(err: matrix_sdk::Error) -> Self {
        Self::Upload(err)
    }
}

impl From<matrix_sdk::HttpError> for AvatarError {
    fn from(err: matrix_sdk::HttpError) -> Self {
        Self::Upload(matrix_sdk::Error::Http(err))
    }
}

impl From<mime::FromStrError> for AvatarError {
    fn from(err: mime::FromStrError) -> Self {
        Self::UnknownFiletype(err)
    }
}

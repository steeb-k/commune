use std::path::Path;

use gtk::{gdk, gdk_pixbuf::Pixbuf, gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use log::{debug, error, info};
use matrix_sdk::room::Room as MatrixRoom;
use matrix_sdk::ruma::events::room::avatar::AvatarEventContent;
use matrix_sdk::ruma::events::AnyStateEventContent;
use matrix_sdk::Client;
use matrix_sdk::{
    media::{MediaFormat, MediaRequest, MediaType},
    ruma::identifiers::MxcUri,
};

use crate::utils::do_async;

use crate::session::Session;

mod imp {
    use super::*;
    use glib::object::WeakRef;
    use once_cell::sync::{Lazy, OnceCell};
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default)]
    pub struct Avatar {
        pub image: RefCell<Option<gdk::Paintable>>,
        pub needed: Cell<bool>,
        pub url: RefCell<Option<MxcUri>>,
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
                    glib::ParamSpec::new_object(
                        "image",
                        "Image",
                        "The user defined image if any",
                        gdk::Paintable::static_type(),
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_object(
                        "needed",
                        "Needed",
                        "Whether the user defined image needs to be loaded",
                        gdk::Paintable::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_string(
                        "url",
                        "Url",
                        "The url of the Avatar",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name used for this avatar",
                        None,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_object(
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
                "needed" => obj.set_needed(value.get().unwrap()),
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
                "needed" => obj.needed().to_value(),
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
    pub fn new(session: &Session, url: Option<MxcUri>) -> Self {
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

        let image = if let Some(data) = data {
            let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(&data));
            Pixbuf::from_stream(&stream, gio::NONE_CANCELLABLE)
                .ok()
                .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf).upcast())
        } else {
            None
        };
        priv_.image.replace(image);
        self.notify("image");
    }

    fn load(&self) {
        // Don't do anything here if we don't need the avatar
        if !self.needed() {
            return;
        }

        if let Some(url) = self.url() {
            let client = self.session().client();
            let request = MediaRequest {
                media_type: MediaType::Uri(url),
                format: MediaFormat::File,
            };
            do_async(
                glib::PRIORITY_LOW,
                async move { client.get_media_content(&request, true).await },
                clone!(@weak self as obj => move |result| async move {
                    // FIXME: We should retry if the request failed
                    match result {
                        Ok(data) => obj.set_image_data(Some(data)),
                        Err(error) => error!("Couldn’t fetch avatar: {}", error),
                    };
                }),
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

    pub fn set_needed(&self, needed: bool) {
        let priv_ = imp::Avatar::from_instance(self);
        if self.needed() == needed {
            return;
        }

        priv_.needed.set(needed);

        if needed {
            self.load();
        }

        self.notify("needed");
    }

    pub fn needed(&self) -> bool {
        let priv_ = imp::Avatar::from_instance(self);
        priv_.needed.get()
    }

    pub fn set_url(&self, url: Option<MxcUri>) {
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

    pub fn url(&self) -> Option<MxcUri> {
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
) -> Result<Option<MxcUri>, AvatarError>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    let joined_room = match matrix_room {
        MatrixRoom::Joined(joined_room) => joined_room,
        _ => return Err(AvatarError::NotAMember),
    };

    let mut content = AvatarEventContent::new();

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
async fn upload_avatar<P>(matrix_client: &Client, filename: &P) -> Result<MxcUri, AvatarError>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    debug!("Getting mime type of file {:?}", filename);
    let image = tokio::fs::read(filename).await?;
    let content_type = gio::content_type_guess(None, &image).0.to_string();

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

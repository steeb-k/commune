use gtk::{glib, prelude::*, subclass::prelude::*};

use super::PackImage;
use crate::{
    components::{AvatarImage, AvatarUriSource, PillSource},
    prelude::*,
    session::Session,
};

/// The height of a custom emoticon in a message, in pixels.
///
/// The specification requires the attribute, for the clients that do not
/// support image packs, and recommends this value. A client that does support
/// them presents the image at a size that suits the font instead.
const HTML_HEIGHT: u32 = 32;

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::EmoticonSource)]
    pub struct EmoticonSource {
        /// The image that this emoticon sends.
        #[property(get, construct_only)]
        pub(super) image: OnceCell<PackImage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EmoticonSource {
        const NAME: &'static str = "EmoticonSource";
        type Type = super::EmoticonSource;
        type ParentType = PillSource;
    }

    #[glib::derived_properties]
    impl ObjectImpl for EmoticonSource {}

    impl PillSourceImpl for EmoticonSource {
        fn identifier(&self) -> String {
            self.image
                .get()
                .expect("image should be initialized")
                .shortcode()
        }
    }
}

glib::wrapper! {
    /// A [`PillSource`] for an image of a pack, sent inline in a message.
    ///
    /// Like [`AtRoom`](crate::components::AtRoom), this is not a mention, it
    /// is only presented like one while the message is composed.
    pub struct EmoticonSource(ObjectSubclass<imp::EmoticonSource>) @extends PillSource;
}

impl EmoticonSource {
    /// Create a new `EmoticonSource` for the given image.
    pub(crate) fn new(session: &Session, image: &PackImage) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("display-name", image.shortcode())
            .property("image", image)
            .build();

        // The images of a pack are not avatars, but they are loaded the same
        // way, and this is what a pill presents.
        // The info of the image is not the one an avatar takes, and it is only
        // a hint for the size to download, so it is left out.
        let avatar_image = AvatarImage::new(
            session,
            AvatarUriSource::Room,
            Some(image.uri().clone()),
            None,
        );
        obj.avatar_data().set_image(Some(avatar_image));

        obj
    }

    /// The shortcode that identifies this emoticon in its pack.
    pub(crate) fn shortcode(&self) -> String {
        self.image().shortcode()
    }

    /// The text to send for this emoticon in the plain body of a message.
    pub(crate) fn to_plain(&self) -> String {
        format!(":{}:", self.shortcode())
    }

    /// The HTML to send for this emoticon in the formatted body of a message.
    pub(crate) fn to_html(&self) -> String {
        let image = self.image();
        let shortcode = self.shortcode();

        format!(
            r#"<img data-mx-emoticon src="{src}" alt="{alt}" title="{title}" height="{HTML_HEIGHT}">"#,
            src = image.uri().as_str().escape_markup(),
            alt = image.body().escape_markup(),
            title = shortcode.escape_markup(),
        )
    }
}

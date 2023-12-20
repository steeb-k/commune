use gtk::{gdk, glib, prelude::*, subclass::prelude::*};
use tracing::warn;

mod avatar_image;

pub use self::avatar_image::{AvatarImage, AvatarUriSource};
use crate::{
    application::Application,
    utils::notifications::{paintable_as_notification_icon, string_as_notification_icon},
};

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::AvatarData)]
    pub struct AvatarData {
        /// The data of the user-defined image.
        #[property(get, set = Self::set_image, explicit_notify, nullable)]
        pub image: RefCell<Option<AvatarImage>>,
        /// The display name used as a fallback for this avatar.
        #[property(get, set = Self::set_display_name, explicit_notify, nullable)]
        pub display_name: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AvatarData {
        const NAME: &'static str = "AvatarData";
        type Type = super::AvatarData;
    }

    #[glib::derived_properties]
    impl ObjectImpl for AvatarData {}

    impl AvatarData {
        /// Set the data of the user-defined image.
        fn set_image(&self, image: Option<AvatarImage>) {
            if *self.image.borrow() == image {
                return;
            }

            self.image.replace(image);
            self.obj().notify_image();
        }

        /// Set the display name used as a fallback for this avatar.
        fn set_display_name(&self, display_name: Option<String>) {
            if *self.display_name.borrow() == display_name {
                return;
            }

            self.display_name.replace(display_name);
            self.obj().notify_display_name();
        }
    }
}

glib::wrapper! {
    /// Data about a User’s or Room’s avatar.
    pub struct AvatarData(ObjectSubclass<imp::AvatarData>);
}

impl AvatarData {
    /// Construct a new empty `AvatarData`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Constructs an `AvatarData` with the given image data.
    pub fn with_image(image: AvatarImage) -> Self {
        glib::Object::builder().property("image", image).build()
    }

    /// Get this avatar as a notification icon.
    ///
    /// Returns `None` if an error occurred while generating the icon.
    pub fn as_notification_icon(&self) -> Option<gdk::Texture> {
        let window = Application::default().active_window()?.upcast();

        let icon = if let Some(paintable) = self.image().and_then(|i| i.paintable()) {
            paintable_as_notification_icon(paintable.upcast_ref(), &window)
        } else {
            string_as_notification_icon(&self.display_name().unwrap_or_default(), &window)
        };

        match icon {
            Ok(icon) => Some(icon),
            Err(error) => {
                warn!("Failed to generate icon for notification: {error}");
                None
            }
        }
    }
}

impl Default for AvatarData {
    fn default() -> Self {
        Self::new()
    }
}

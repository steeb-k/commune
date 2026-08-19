use gtk::{gio, glib, prelude::*, subclass::prelude::*};

use super::{
    PackImage,
    events::{PackContent, PackUsage},
};
use crate::{prelude::*, session::Room};

/// The event type that an image pack in the state of a room is defined under.
///
/// A pack is written back under the type that it was read from, so that
/// editing a pack that another client defined does not leave a second copy of
/// it behind under the other name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoomPackKind {
    /// `im.ponies.room_emotes`, from MSC2545, which is the one we create.
    Unstable,
    /// `m.room.image_pack`, from the specification.
    Stable,
}

/// Where an image pack comes from.
///
/// Always the state of a room: the specification has no personal pack, and
/// expects one to be a room pack enabled everywhere instead.
#[derive(Debug, Clone)]
pub(crate) struct ImagePackSource {
    /// The room that defines the pack.
    pub(crate) room: Room,
    /// The state key that identifies the pack in that room.
    pub(crate) state_key: String,
    /// The event type that the pack is defined under.
    pub(crate) kind: RoomPackKind,
}

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ImagePack)]
    pub struct ImagePack {
        /// Where this pack comes from.
        pub(super) source: OnceCell<ImagePackSource>,
        /// The content of this pack.
        pub(super) content: OnceCell<PackContent>,
        /// The images of this pack.
        #[property(get = Self::images)]
        images: OnceCell<gio::ListStore>,
        /// The name of this pack, as shown to the user.
        #[property(get = Self::display_name)]
        display_name: PhantomData<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImagePack {
        const NAME: &'static str = "ImagePack";
        type Type = super::ImagePack;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImagePack {}

    impl ImagePack {
        /// Where this pack comes from.
        pub(super) fn source(&self) -> &ImagePackSource {
            self.source.get().expect("source should be initialized")
        }

        /// The content of this pack.
        pub(super) fn content(&self) -> &PackContent {
            self.content.get().expect("content should be initialized")
        }

        /// The images of this pack.
        ///
        /// The specification does not define an order for the images of a
        /// pack, so they are sorted by shortcode to at least be stable.
        fn images(&self) -> gio::ListStore {
            self.images
                .get_or_init(|| {
                    let images = gio::ListStore::new::<PackImage>();

                    for (shortcode, data) in &self.content().images {
                        images.append(&PackImage::new(shortcode.clone(), data.clone()));
                    }

                    images
                })
                .clone()
        }

        /// The name of this pack, as shown to the user.
        fn display_name(&self) -> String {
            if let Some(display_name) = &self.content().pack.display_name {
                return display_name.clone();
            }

            self.source().room.display_name()
        }
    }
}

glib::wrapper! {
    /// An image pack.
    pub struct ImagePack(ObjectSubclass<imp::ImagePack>);
}

impl ImagePack {
    /// Create a new `ImagePack` with the given source and content.
    pub(crate) fn new(source: ImagePackSource, content: PackContent) -> Self {
        let obj = glib::Object::new::<Self>();

        let imp = obj.imp();
        imp.source.set(source).expect("source is not initialized");
        imp.content
            .set(content)
            .expect("content is not initialized");

        obj
    }

    /// Where this pack comes from.
    pub(crate) fn source(&self) -> &ImagePackSource {
        self.imp().source()
    }

    /// A copy of the content of this pack.
    ///
    /// The content of a pack is not changed in place: an editor works on a
    /// copy and sends the result, and the pack is built again from what comes
    /// back through sync.
    pub(crate) fn content(&self) -> PackContent {
        self.imp().content().clone()
    }

    /// Who to credit for this pack.
    pub(crate) fn attribution(&self) -> Option<&str> {
        self.imp().content().pack.attribution.as_deref()
    }

    /// Whether this pack can be used for the given usage.
    pub(crate) fn has_usage(&self, usage: &PackUsage) -> bool {
        self.imp().content().pack.has_usage(usage)
    }

    /// Whether this pack has no images.
    pub(crate) fn is_empty(&self) -> bool {
        self.imp().content().images.is_empty()
    }
}

pub(crate) use commune_core::session::RoomPackKind;
use commune_core::session::{ImagePack as CoreImagePack, ImagePackSource as CoreImagePackSource};
use gtk::{gio, glib, prelude::*, subclass::prelude::*};

use super::{PackImage, events::PackContent};
use crate::{
    prelude::*,
    session::{Room, Session},
};

/// Where an image pack comes from.
///
/// Always the state of a room: the specification has no personal pack, and
/// expects one to be a room pack enabled everywhere instead. This is the
/// core's [`ImagePackSource`](CoreImagePackSource) with the room the
/// interface presents.
#[derive(Debug, Clone)]
pub(crate) struct ImagePackSource {
    /// The room that defines the pack.
    pub(crate) room: Room,
    /// The state key that identifies the pack in that room.
    pub(crate) state_key: String,
    /// The event type that the pack is defined under.
    pub(crate) kind: RoomPackKind,
}

impl ImagePackSource {
    /// This source, as the core names it.
    pub(crate) fn to_core(&self) -> CoreImagePackSource {
        CoreImagePackSource {
            room: self.room.core().clone(),
            state_key: self.state_key.clone(),
            kind: self.kind,
        }
    }
}

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ImagePack)]
    pub struct ImagePack {
        /// The pack, as the core holds it.
        pub(super) core: OnceCell<CoreImagePack>,
        /// Where this pack comes from, with the room the interface presents.
        pub(super) source: OnceCell<ImagePackSource>,
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
        /// The pack, as the core holds it.
        pub(super) fn core(&self) -> &CoreImagePack {
            self.core.get().expect("core should be initialized")
        }

        /// Where this pack comes from.
        pub(super) fn source(&self) -> &ImagePackSource {
            self.source.get().expect("source should be initialized")
        }

        /// The images of this pack.
        ///
        /// The specification does not define an order for the images of a
        /// pack, so they are sorted by shortcode to at least be stable.
        fn images(&self) -> gio::ListStore {
            self.images
                .get_or_init(|| {
                    let images = gio::ListStore::new::<PackImage>();

                    for (shortcode, data) in &self.core().content.images {
                        images.append(&PackImage::new(shortcode.clone(), data.clone()));
                    }

                    images
                })
                .clone()
        }

        /// The name of this pack, as shown to the user.
        ///
        /// The core answers when the pack or its room is named; the other
        /// names of a room are sentences, and those are the interface's.
        fn display_name(&self) -> String {
            self.core()
                .display_name()
                .unwrap_or_else(|| self.source().room.display_name())
        }
    }
}

glib::wrapper! {
    /// An image pack.
    ///
    /// The pack is the core's; this presents it.
    pub struct ImagePack(ObjectSubclass<imp::ImagePack>);
}

impl ImagePack {
    /// Present the given pack of the core.
    ///
    /// `None` when the room that defines it is not in the list, which the
    /// core already ruled out for the packs it returns.
    pub(crate) fn from_core(session: &Session, pack: CoreImagePack) -> Option<Self> {
        let room = session.room_list().get(pack.source.room.room_id())?;
        let source = ImagePackSource {
            room,
            state_key: pack.source.state_key.clone(),
            kind: pack.source.kind,
        };

        let obj = glib::Object::new::<Self>();

        let imp = obj.imp();
        imp.core.set(pack).expect("core is not initialized");
        imp.source.set(source).expect("source is not initialized");

        Some(obj)
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
        self.imp().core().content.clone()
    }

    /// Who to credit for this pack.
    pub(crate) fn attribution(&self) -> Option<&str> {
        self.imp().core().attribution()
    }
}

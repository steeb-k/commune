use gettextrs::gettext;
use gtk::{glib, subclass::prelude::*};
use ruma::OwnedRoomId;

use crate::{components::PillSource, prelude::*};

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct AtRoom {
        /// The ID of the room currently represented.
        pub room_id: OnceCell<OwnedRoomId>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AtRoom {
        const NAME: &'static str = "AtRoom";
        type Type = super::AtRoom;
        type ParentType = PillSource;
    }

    impl ObjectImpl for AtRoom {}

    impl PillSourceImpl for AtRoom {
        fn identifier(&self) -> String {
            gettext("Notify the whole room")
        }
    }
}

glib::wrapper! {
    /// A helper `PillSource` to represent an `@room` mention.
    pub struct AtRoom(ObjectSubclass<imp::AtRoom>) @extends PillSource;
}

impl AtRoom {
    /// Constructs an empty `@room` mention.
    pub fn new(room_id: OwnedRoomId) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("display-name", "@room")
            .build();

        obj.imp().room_id.set(room_id).unwrap();

        obj
    }

    /// The ID of the room currently represented.
    pub fn room_id(&self) -> &OwnedRoomId {
        self.imp().room_id.get().unwrap()
    }
}

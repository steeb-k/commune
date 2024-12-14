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
        room_id: OnceCell<OwnedRoomId>,
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

    impl AtRoom {
        /// Set the ID of the room currently represented.
        pub(super) fn set_room_id(&self, room_id: OwnedRoomId) {
            self.room_id.set(room_id).expect("room ID is uninitialized");
        }

        /// The ID of the room currently represented.
        pub(super) fn room_id(&self) -> &OwnedRoomId {
            self.room_id.get().expect("room ID is initialized")
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

        obj.imp().set_room_id(room_id);

        obj
    }

    /// The ID of the room currently represented.
    pub fn room_id(&self) -> &OwnedRoomId {
        self.imp().room_id()
    }
}

use commune_core::session::PeekedMessage as CorePeekedMessage;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::OwnedRoomId;
use tokio::task::AbortHandle;

use crate::{
    session::Session,
    spawn, spawn_tokio,
    utils::{LoadingState, matrix::timestamp_to_date},
};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomPeek)]
    pub struct RoomPeek {
        /// The messages that were read, oldest first.
        #[property(get = Self::list_owned)]
        list: OnceCell<gio::ListStore>,
        /// The session to make the request with.
        session: glib::WeakRef<Session>,
        /// The ID of the room being read.
        room_id: RefCell<Option<OwnedRoomId>>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// The abort handle for the current request.
        abort_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomPeek {
        const NAME: &'static str = "RoomPeek";
        type Type = super::RoomPeek;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomPeek {
        fn dispose(&self) {
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }
        }
    }

    impl RoomPeek {
        /// The messages that were read, oldest first.
        fn list(&self) -> &gio::ListStore {
            self.list
                .get_or_init(gio::ListStore::new::<super::PeekedMessage>)
        }

        /// The owned list of messages that were read.
        fn list_owned(&self) -> gio::ListStore {
            self.list().clone()
        }

        /// Whether the list has no messages in it.
        pub(super) fn is_empty(&self) -> bool {
            self.list().n_items() == 0
        }

        /// Set the room to read, and read it.
        pub(super) fn set_room(&self, session: &Session, room_id: OwnedRoomId) {
            if self.room_id.borrow().as_ref() == Some(&room_id)
                && self.loading_state.get() != LoadingState::Error
            {
                // The same room was already read, or is being read.
                return;
            }

            self.session.set(Some(session));
            self.room_id.replace(Some(room_id));

            self.reload();
        }

        /// Read the current room again, from scratch.
        pub(super) fn reload(&self) {
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }

        /// Set the loading state.
        fn set_loading_state(&self, state: LoadingState) {
            if self.loading_state.get() == state {
                return;
            }

            self.loading_state.set(state);
            self.obj().notify_loading_state();
        }

        /// Read the last messages of the current room.
        async fn load(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let Some(room_id) = self.room_id.borrow().clone() else {
                return;
            };

            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }
            self.list().remove_all();
            self.set_loading_state(LoadingState::Loading);

            let core = session.core().clone();
            let room_id_clone = room_id.clone();
            let handle = spawn_tokio!(async move { core.peek_room(&room_id_clone).await });
            self.abort_handle.replace(Some(handle.abort_handle()));

            let Ok(result) = handle.await else {
                // The request was aborted.
                return;
            };

            self.abort_handle.take();

            if self.room_id.borrow().as_deref() != Some(&*room_id) {
                // We are reading a different room now, ignore the response.
                return;
            }

            match result {
                Ok(messages) => {
                    let messages = messages
                        .into_iter()
                        .map(super::PeekedMessage::new)
                        .collect::<Vec<_>>();
                    self.list().extend_from_slice(&messages);
                    self.set_loading_state(LoadingState::Ready);
                }
                // Already logged by the core, at the level it deserves: this
                // is the expected outcome for most rooms.
                Err(_) => self.set_loading_state(LoadingState::Error),
            }
        }
    }
}

glib::wrapper! {
    /// The last messages of a room that can be read without joining it.
    ///
    /// This is a peek, in the sense of the Matrix specification: the room's
    /// `m.room.history_visibility` is `world_readable`, so `/messages` answers
    /// for somebody who is not a member. Nothing here syncs, nothing here
    /// paginates, and nothing here can be replied to. The read is the core's;
    /// this presents it.
    pub struct RoomPeek(ObjectSubclass<imp::RoomPeek>);
}

impl RoomPeek {
    /// Construct a new empty `RoomPeek`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Whether no message was read.
    pub(crate) fn is_empty(&self) -> bool {
        self.imp().is_empty()
    }

    /// Read the last messages of the given room.
    pub(crate) fn set_room(&self, session: &Session, room_id: OwnedRoomId) {
        self.imp().set_room(session, room_id);
    }
}

impl Default for RoomPeek {
    fn default() -> Self {
        Self::new()
    }
}

mod message_imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct PeekedMessage {
        /// The message, as the core read it.
        core: OnceCell<CorePeekedMessage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PeekedMessage {
        const NAME: &'static str = "PeekedMessage";
        type Type = super::PeekedMessage;
    }

    impl ObjectImpl for PeekedMessage {}

    impl PeekedMessage {
        /// Set the message this presents.
        pub(super) fn set_message(&self, message: CorePeekedMessage) {
            self.core
                .set(message)
                .expect("message should be uninitialized");
        }

        /// The message, as the core read it.
        pub(super) fn core(&self) -> &CorePeekedMessage {
            self.core.get().expect("message should be initialized")
        }
    }
}

glib::wrapper! {
    /// One message read from a room that was not joined.
    pub struct PeekedMessage(ObjectSubclass<message_imp::PeekedMessage>);
}

impl PeekedMessage {
    /// Construct a new `PeekedMessage` for the given message of the core.
    fn new(message: CorePeekedMessage) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp().set_message(message);
        obj
    }

    /// The name to present the sender of this message under.
    ///
    /// Falls back to the user ID, which is what a room whose member list we
    /// cannot read leaves us with.
    pub(crate) fn sender_name(&self) -> String {
        self.imp().core().sender_name()
    }

    /// The timestamp of this message, as a `GDateTime`.
    pub(crate) fn timestamp(&self) -> glib::DateTime {
        timestamp_to_date(self.imp().core().timestamp())
    }

    /// The textual content of this message.
    pub(crate) fn body(&self) -> String {
        self.imp().core().body().to_owned()
    }
}

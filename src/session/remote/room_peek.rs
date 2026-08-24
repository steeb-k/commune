use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{
    OwnedRoomId, OwnedUserId, RoomId,
    api::client::{
        filter::{LazyLoadOptions, RoomEventFilter},
        message::get_message_events,
    },
    assign,
    events::{
        AnyStateEvent, AnySyncTimelineEvent, MessageLikeEventType,
        room::message::OriginalSyncRoomMessageEvent,
    },
};
use tokio::task::AbortHandle;
use tracing::debug;

use crate::{
    session::Session,
    spawn, spawn_tokio,
    utils::{
        LoadingState,
        matrix::{original_message_event_from_raw, timestamp_to_date},
    },
};

/// How many messages to show.
///
/// This is a taste of the room, not its history: there is no scrollback, so
/// the number is whatever fills a dialog and stops.
const PEEK_LIMIT: u32 = 20;

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        collections::HashMap,
    };

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

            // Only messages, and only the member events of the people who sent
            // them: a preview has no room for state changes, and lazy-loading
            // is the only way to learn a sender's name without being able to
            // ask for the member list of a room we are not in.
            let filter = assign!(RoomEventFilter::default(), {
                types: Some(vec![MessageLikeEventType::RoomMessage.to_string()]),
                lazy_load_options: LazyLoadOptions::Enabled {
                    include_redundant_members: false,
                },
            });
            let request = assign!(get_message_events::v3::Request::backward(room_id.clone()), {
                limit: PEEK_LIMIT.into(),
                filter,
            });

            let client = session.client();
            let handle = spawn_tokio!(async move { client.send(request).await });
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
                Ok(response) => self.add_messages(&room_id, &response),
                Err(error) => {
                    // A room that is not `world_readable`, or that this
                    // homeserver does not have, answers with an error here.
                    // This is the expected outcome for most rooms.
                    debug!("Could not read the messages of room `{room_id}`: {error}");
                    self.set_loading_state(LoadingState::Error);
                }
            }
        }

        /// Add the messages from the given response to the list.
        fn add_messages(&self, room_id: &RoomId, response: &get_message_events::v3::Response) {
            let names = sender_names(&response.state);

            // The request walks backwards from the end of the room, so the
            // response is newest first and the list wants the opposite.
            let messages = response
                .chunk
                .iter()
                .rev()
                .filter_map(|raw| {
                    // The JSON of a timeline event is the JSON of a sync
                    // timeline event plus a `room_id`, so it deserializes as
                    // one. There is no `JsonCastable` for the pair.
                    let event = original_message_event_from_raw(
                        raw.cast_ref_unchecked::<AnySyncTimelineEvent>(),
                    )?;
                    let sender_name = names.get(&event.sender).cloned();

                    Some(super::PeekedMessage::new(event, sender_name))
                })
                .collect::<Vec<_>>();

            if messages.is_empty() {
                debug!("Nothing readable in the last messages of room `{room_id}`");
            }

            self.list().extend_from_slice(&messages);
            self.set_loading_state(LoadingState::Ready);
        }
    }

    /// The display name to use for each sender named by the given member
    /// events.
    ///
    /// A name shared by two people is not used for either of them, as the spec
    /// requires: in a room we cannot see the member list of, the user ID is the
    /// only thing left that tells them apart.
    fn sender_names(state: &[ruma::serde::Raw<AnyStateEvent>]) -> HashMap<OwnedUserId, String> {
        let mut names = HashMap::new();
        let mut counts: HashMap<String, usize> = HashMap::new();

        for raw_event in state {
            let Ok(AnyStateEvent::RoomMember(event)) = raw_event.deserialize() else {
                continue;
            };
            let Some(event) = event.as_original() else {
                continue;
            };
            let Some(name) = event
                .content
                .displayname
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };

            *counts.entry(name.to_owned()).or_default() += 1;
            names.insert(event.state_key.clone(), name.to_owned());
        }

        names.retain(|_, name| counts.get(name).copied().unwrap_or_default() == 1);
        names
    }
}

glib::wrapper! {
    /// The last messages of a room that can be read without joining it.
    ///
    /// This is a peek, in the sense of the Matrix specification: the room's
    /// `m.room.history_visibility` is `world_readable`, so `/messages` answers
    /// for somebody who is not a member. Nothing here syncs, nothing here
    /// paginates, and nothing here can be replied to.
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
        /// The Matrix event.
        matrix_event: OnceCell<OriginalSyncRoomMessageEvent>,
        /// The display name of the sender, if it is known and unambiguous.
        sender_name: OnceCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PeekedMessage {
        const NAME: &'static str = "PeekedMessage";
        type Type = super::PeekedMessage;
    }

    impl ObjectImpl for PeekedMessage {}

    impl PeekedMessage {
        /// Set the message this presents.
        pub(super) fn set_message(
            &self,
            matrix_event: OriginalSyncRoomMessageEvent,
            sender_name: Option<String>,
        ) {
            self.matrix_event
                .set(matrix_event)
                .expect("Matrix event should be uninitialized");
            self.sender_name
                .set(sender_name)
                .expect("sender name should be uninitialized");
        }

        /// The Matrix event.
        pub(super) fn matrix_event(&self) -> &OriginalSyncRoomMessageEvent {
            self.matrix_event
                .get()
                .expect("Matrix event should be initialized")
        }

        /// The display name of the sender, if it is known and unambiguous.
        pub(super) fn sender_name(&self) -> Option<&str> {
            self.sender_name
                .get()
                .expect("sender name should be initialized")
                .as_deref()
        }
    }
}

glib::wrapper! {
    /// One message read from a room that was not joined.
    pub struct PeekedMessage(ObjectSubclass<message_imp::PeekedMessage>);
}

impl PeekedMessage {
    /// Construct a new `PeekedMessage` for the given event.
    fn new(matrix_event: OriginalSyncRoomMessageEvent, sender_name: Option<String>) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp().set_message(matrix_event, sender_name);
        obj
    }

    /// The name to present the sender of this message under.
    ///
    /// Falls back to the user ID, which is what a room whose member list we
    /// cannot read leaves us with.
    pub(crate) fn sender_name(&self) -> String {
        let imp = self.imp();

        imp.sender_name()
            .map_or_else(|| imp.matrix_event().sender.to_string(), ToOwned::to_owned)
    }

    /// The timestamp of this message, as a `GDateTime`.
    pub(crate) fn timestamp(&self) -> glib::DateTime {
        timestamp_to_date(self.imp().matrix_event().origin_server_ts)
    }

    /// The textual content of this message.
    pub(crate) fn body(&self) -> String {
        self.imp().matrix_event().content.msgtype.body().to_owned()
    }
}

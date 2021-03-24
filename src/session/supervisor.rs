use gtk::glib;
use gtk_macros::send;
use log::error;
use matrix_sdk::{
    self, async_trait,
    events::{
        room::{
            aliases::AliasesEventContent, avatar::AvatarEventContent,
            canonical_alias::CanonicalAliasEventContent, join_rules::JoinRulesEventContent,
            message::MessageEventContent, name::NameEventContent, tombstone::TombstoneEventContent,
        },
        SyncMessageEvent, SyncStateEvent,
    },
    identifiers::RoomId,
    room::Room,
    CustomEvent, EventHandler,
};
use serde_json::value::RawValue as RawJsonValue;
use std::sync::{Arc, RwLock};

/// The `Supervisor` implements the `matrix_sdk::EventHandler`.
///
/// The idea is that the `Supervisor` sends a message to a `channel` when a matrix event is
/// received.
/// Every major UI component should provide a `glib::SyncSender<T>` where `T` is the message the UI
/// compnent excpects to receive for a matrix event.
///
pub struct Supervisor {
    sidebar: glib::SyncSender<RoomId>,
    // TODO: figure out what infromation the content actually needs and should receive from an
    // event
    content: glib::SyncSender<RoomId>,
    /// The ID of the room we want to receive updates for in the content, this is usually the
    /// user visible room.
    pub room_of_intressed: Arc<RwLock<Option<RoomId>>>,
}

impl Supervisor {
    pub fn new(sidebar: glib::SyncSender<RoomId>, content: glib::SyncSender<RoomId>) -> Self {
        Self {
            sidebar,
            content,
            room_of_intressed: Arc::new(RwLock::new(None)),
        }
    }
}

#[async_trait]
impl EventHandler for Supervisor {
    async fn on_room_name(&self, room: Room, _: &SyncStateEvent<NameEventContent>) {
        send!(self.sidebar, room.room_id().clone());
    }

    async fn on_room_canonical_alias(
        &self,
        room: Room,
        _: &SyncStateEvent<CanonicalAliasEventContent>,
    ) {
        send!(self.sidebar, room.room_id().clone());
    }

    async fn on_room_aliases(&self, room: Room, _: &SyncStateEvent<AliasesEventContent>) {
        send!(self.sidebar, room.room_id().clone());
    }

    async fn on_room_avatar(&self, room: Room, _: &SyncStateEvent<AvatarEventContent>) {
        send!(self.sidebar, room.room_id().clone());
    }

    async fn on_room_message(&self, room: Room, _: &SyncMessageEvent<MessageEventContent>) {
        // TODO: get the correct event for new notification count
        send!(self.sidebar, room.room_id().clone());
        send!(self.content, room.room_id().clone());
    }
    async fn on_room_join_rules(&self, room: Room, _: &SyncStateEvent<JoinRulesEventContent>) {
        send!(self.sidebar, room.room_id().clone());
    }

    async fn on_room_tombstone(&self, room: Room, _: &SyncStateEvent<TombstoneEventContent>) {
        send!(self.sidebar, room.room_id().clone());
    }

    async fn on_unrecognized_event(&self, room: Room, _: &RawJsonValue) {
        send!(self.sidebar, room.room_id().clone());
    }

    async fn on_custom_event(&self, room: Room, _: &CustomEvent<'_>) {
        send!(self.sidebar, room.room_id().clone());
    }
}

//! The [image packs] available to a session.
//!
//! [image packs]: https://spec.matrix.org/v1.19/client-server-api/#image-packs

use std::collections::HashSet;

use gettextrs::gettext;
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use indexmap::IndexMap;
use matrix_sdk::{RoomState, deserialized_responses::RawAnySyncOrStrippedState};
use ruma::{
    OwnedRoomId, RoomId,
    api::client::{
        room::{Visibility, create_room, create_room::v3::RoomPreset},
        state::get_state_event_for_key,
    },
    assign,
    events::{
        StaticEventContent,
        tag::{TagInfo, TagName},
    },
};
use tracing::{debug, error, warn};

mod emoticon_source;
mod events;
mod image_pack;
mod pack_image;

use self::events::{
    EmoteRoomsEvent, EmoteRoomsEventContent, EnabledPacks, ImagePackRoomsEvent,
    ImagePackRoomsEventContent, ImagePacksRoomEventContent, RoomEmotesEventContent,
    RoomImagePackEventContent, SyncRoomEmotesEvent, SyncRoomImagePackEvent,
};
pub(crate) use self::{
    emoticon_source::EmoticonSource,
    events::{
        PackContent, PackImage as PackImageData, PackUsage, SHORTCODE_MAX_LEN, is_valid_shortcode,
    },
    image_pack::{ImagePack, ImagePackSource, RoomPackKind},
    pack_image::PackImage,
};
use super::{Room, Session};
use crate::{spawn, spawn_tokio};

/// The event type of the room image packs that we create.
///
/// Exposed so that the permission to change them can be checked without
/// repeating the string.
pub(crate) const ROOM_IMAGE_PACK_EVENT_TYPE: &str = RoomEmotesEventContent::TYPE;

/// The state key of the first image pack of a room.
const FIRST_STATE_KEY: &str = "";

/// The prefix of the state keys of the image packs after the first one.
const STATE_KEY_PREFIX: &str = "pack";

/// The event types of a room image pack, in the order in which they are read.
///
/// The unstable type comes last so that it wins over the stable one, since it
/// is the one that we send.
const ROOM_PACK_TYPES: &[(RoomPackKind, &str)] = &[
    (RoomPackKind::Stable, RoomImagePackEventContent::TYPE),
    (RoomPackKind::Unstable, RoomEmotesEventContent::TYPE),
];

/// Read the image packs defined in the state of the given room.
///
/// Packs are keyed by their state key, and carry the event type that they were
/// read from, so that they can be written back under the same one. A pack
/// defined under both the stable and the unstable event type is only returned
/// once.
async fn room_state_packs(room: &Room) -> IndexMap<String, (RoomPackKind, PackContent)> {
    let matrix_room = room.matrix_room().clone();

    let handle = spawn_tokio!(async move {
        let mut raw_events = Vec::new();

        for (kind, event_type) in ROOM_PACK_TYPES {
            match matrix_room.get_state_events((*event_type).into()).await {
                Ok(events) => raw_events.extend(events.into_iter().map(|event| (*kind, event))),
                Err(error) => error!("Could not get the image packs of a room: {error}"),
            }
        }

        raw_events
    });
    let raw_events = handle.await.expect("task was not aborted");

    let mut packs = IndexMap::new();

    for (kind, raw_event) in raw_events {
        let RawAnySyncOrStrippedState::Sync(raw_event) = raw_event else {
            // A room that we are only invited to does not expose its packs.
            continue;
        };

        let state_key = match raw_event.get_field::<String>("state_key") {
            Ok(Some(state_key)) => state_key,
            Ok(None) => continue,
            Err(error) => {
                error!("Could not read the state key of an image pack: {error}");
                continue;
            }
        };

        let content = match raw_event.get_field::<PackContent>("content") {
            Ok(Some(content)) => content,
            Ok(None) => continue,
            Err(error) => {
                error!("Could not deserialize an image pack: {error}");
                continue;
            }
        };

        // A redacted pack, or a pack that was deleted, has no images.
        if content.images.is_empty() {
            continue;
        }

        packs.insert(state_key, (kind, content));
    }

    packs
}

/// Ask the homeserver for the image pack with the given state key in the
/// given room.
///
/// Used for a pack that the state store does not have, which happens when the
/// state of the room never reached this device.
async fn fetch_room_pack(session: &Session, room: &Room, state_key: String) -> Option<ImagePack> {
    let client = session.client();
    let room_id = room.room_id().to_owned();
    let key = state_key.clone();

    let handle = spawn_tokio!(async move {
        // The unstable type comes first, so that it wins over the stable one,
        // as it does when the state store is what answers.
        for (kind, event_type) in ROOM_PACK_TYPES.iter().rev() {
            let request = get_state_event_for_key::v3::Request::new(
                room_id.clone(),
                (*event_type).into(),
                key.clone(),
            );

            // Not having the event is the common answer, and not an error
            // worth reporting: a pack is only defined under one of the names.
            let Ok(response) = client.send(request).await else {
                continue;
            };

            match serde_json::from_str::<PackContent>(response.event_or_content.get()) {
                Ok(content) if !content.images.is_empty() => return Some((*kind, content)),
                Ok(_) => {}
                Err(error) => error!("Could not deserialize an image pack: {error}"),
            }
        }

        None
    });

    let (kind, content) = handle.await.expect("task was not aborted")?;

    Some(ImagePack::new(
        ImagePackSource {
            room: room.clone(),
            state_key,
            kind,
        },
        content,
    ))
}

/// An image pack that is used everywhere but cannot be loaded, because the
/// user is not in the room that defines it anymore.
#[derive(Debug, Clone)]
pub(crate) struct UnavailablePack {
    /// The room that defines the pack.
    pub(crate) room_id: OwnedRoomId,
    /// The state key that identifies the pack in that room.
    pub(crate) state_key: String,
}

mod imp {
    use std::cell::RefCell;

    use glib::subclass::Signal;
    use matrix_sdk::event_handler::EventHandlerDropGuard;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ImagePacks)]
    pub struct ImagePacks {
        /// The session that these image packs belong to.
        #[property(get, construct_only)]
        pub(super) session: glib::WeakRef<Session>,
        /// The room image packs enabled globally, under the unstable name.
        ///
        /// This is the one we write to. The two are kept apart so that
        /// disabling a pack can remove it from whichever event holds it.
        pub(super) enabled_packs_unstable: RefCell<EnabledPacks>,
        /// The room image packs enabled globally, under the stable name.
        pub(super) enabled_packs_stable: RefCell<EnabledPacks>,
        drop_guards: RefCell<Vec<EventHandlerDropGuard>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImagePacks {
        const NAME: &'static str = "ImagePacks";
        type Type = super::ImagePacks;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImagePacks {
        fn signals() -> &'static [Signal] {
            static SIGNALS: std::sync::LazyLock<Vec<Signal>> =
                std::sync::LazyLock::new(|| vec![Signal::builder("changed").build()]);
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.init().await;
                }
            ));
        }
    }

    impl ImagePacks {
        /// Load the account data and watch it for changes.
        async fn init(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let client = session.client();

            self.load_enabled_packs().await;

            // The enabled packs are watched under both names, because an event
            // handler only matches the single type of the content that it
            // takes.
            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let unstable_handle = client.add_event_handler(move |_: EmoteRoomsEvent| {
                let obj_weak = obj_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.imp().load_enabled_packs().await;
                                obj.emit_by_name::<()>("changed", &[]);
                            }
                        });
                    });
                }
            });

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let stable_handle = client.add_event_handler(move |_: ImagePackRoomsEvent| {
                let obj_weak = obj_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.imp().load_enabled_packs().await;
                                obj.emit_by_name::<()>("changed", &[]);
                            }
                        });
                    });
                }
            });

            // The packs in the state of a room are not kept here, because they
            // are read from the state store when they are needed, but a change
            // to one of them still has to be announced, under both names.
            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let unstable_room_handle = client.add_event_handler(move |_: SyncRoomEmotesEvent| {
                let obj_weak = obj_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        if let Some(obj) = obj_weak.upgrade() {
                            obj.emit_by_name::<()>("changed", &[]);
                        }
                    });
                }
            });

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let stable_room_handle = client.add_event_handler(move |_: SyncRoomImagePackEvent| {
                let obj_weak = obj_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        if let Some(obj) = obj_weak.upgrade() {
                            obj.emit_by_name::<()>("changed", &[]);
                        }
                    });
                }
            });

            self.drop_guards.replace(
                [
                    unstable_handle,
                    stable_handle,
                    unstable_room_handle,
                    stable_room_handle,
                ]
                .into_iter()
                .map(|handle| client.event_handler_drop_guard(handle))
                .collect(),
            );
        }

        /// Load the globally enabled room image packs from the store.
        pub(super) async fn load_enabled_packs(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let client = session.client();
            let handle = spawn_tokio!(async move {
                let account = client.account();
                let stable = account.account_data::<ImagePackRoomsEventContent>().await;
                let unstable = account.account_data::<EmoteRoomsEventContent>().await;
                (stable, unstable)
            });
            let (stable, unstable) = handle.await.expect("task was not aborted");

            let stable = match stable {
                Ok(Some(raw)) => match raw.deserialize() {
                    Ok(content) => content.rooms,
                    Err(error) => {
                        error!("Could not deserialize the enabled image packs: {error}");
                        return;
                    }
                },
                Ok(None) => EnabledPacks::new(),
                Err(error) => {
                    error!("Could not get the enabled image packs: {error}");
                    return;
                }
            };

            let unstable = match unstable {
                Ok(Some(raw)) => match raw.deserialize() {
                    Ok(content) => content.rooms,
                    Err(error) => {
                        error!("Could not deserialize the enabled image packs: {error}");
                        return;
                    }
                },
                Ok(None) => EnabledPacks::new(),
                Err(error) => {
                    error!("Could not get the enabled image packs: {error}");
                    return;
                }
            };

            self.enabled_packs_stable.replace(stable);
            self.enabled_packs_unstable.replace(unstable);
        }

        /// The room image packs enabled globally, under both names.
        pub(super) fn enabled_packs(&self) -> EnabledPacks {
            let mut enabled_packs = self.enabled_packs_stable.borrow().clone();

            for (room_id, packs) in &*self.enabled_packs_unstable.borrow() {
                enabled_packs
                    .entry(room_id.clone())
                    .or_default()
                    .extend(packs.iter().map(|(key, meta)| (key.clone(), meta.clone())));
            }

            enabled_packs
        }
    }
}

glib::wrapper! {
    /// The image packs available to a [`Session`].
    pub struct ImagePacks(ObjectSubclass<imp::ImagePacks>);
}

impl ImagePacks {
    /// Create a new `ImagePacks` for the given session.
    pub(crate) fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// The image packs that the user can use in the given room, for the given
    /// usage, or for every usage when it is `None`.
    ///
    /// They are in the order the specification asks for: the packs that the
    /// user enabled everywhere, then the packs of the room. A pack that is
    /// both enabled everywhere and defined in the room is only returned once.
    ///
    /// The packs of the canonical space of the room are not included yet.
    pub(crate) async fn packs_for_room(
        &self,
        room: &Room,
        usage: Option<&PackUsage>,
    ) -> Vec<ImagePack> {
        let mut packs = Vec::new();

        let mut push = |pack: ImagePack| {
            if !pack.is_empty() && usage.is_none_or(|usage| pack.has_usage(usage)) {
                packs.push(pack);
            }
        };

        let Some(session) = self.session() else {
            return packs;
        };
        let room_list = session.room_list();
        let enabled_packs = self.imp().enabled_packs();

        let mut seen = Vec::new();

        for (room_id, state_keys) in enabled_packs {
            let Some(source_room) = room_list.get(&room_id) else {
                // The specification expects us to tell the user that they are
                // not in that room anymore, which the pack management does.
                debug!("Ignoring the enabled image packs of the unknown room {room_id}");
                continue;
            };

            let mut room_packs = room_state_packs(&source_room).await;

            for state_key in state_keys.into_keys() {
                let Some((kind, content)) = room_packs.shift_remove(&state_key) else {
                    continue;
                };

                seen.push((room_id.clone(), state_key.clone()));
                push(ImagePack::new(
                    ImagePackSource {
                        room: source_room.clone(),
                        state_key,
                        kind,
                    },
                    content,
                ));
            }
        }

        let room_id = room.room_id().to_owned();
        for (state_key, (kind, content)) in room_state_packs(room).await {
            if seen.contains(&(room_id.clone(), state_key.clone())) {
                continue;
            }

            push(ImagePack::new(
                ImagePackSource {
                    room: room.clone(),
                    state_key,
                    kind,
                },
                content,
            ));
        }

        packs
    }

    /// The image packs that are used everywhere but cannot be loaded.
    ///
    /// The specification expects clients to be aware that the user might not
    /// be in the room that defines a pack anymore, so that they can be told
    /// about it and stop using it.
    pub(crate) fn unavailable_packs(&self) -> Vec<UnavailablePack> {
        let Some(session) = self.session() else {
            return Vec::new();
        };
        let room_list = session.room_list();

        let mut packs = Vec::new();

        for (room_id, state_keys) in self.imp().enabled_packs() {
            // A pack is unavailable when the user left the room that defines
            // it, not when its state has yet to reach this device: that one is
            // asked for instead, by `all_packs`.
            if room_list
                .get(&room_id)
                .is_some_and(|room| room.matrix_room().state() == RoomState::Joined)
            {
                continue;
            }

            for state_key in state_keys.into_keys() {
                packs.push(UnavailablePack {
                    room_id: room_id.clone(),
                    state_key,
                });
            }
        }

        packs
    }

    /// All the image packs defined in the state of the given room, whatever
    /// they are meant to be used for.
    pub(crate) async fn room_packs(room: &Room) -> Vec<ImagePack> {
        room_state_packs(room)
            .await
            .into_iter()
            .map(|(state_key, (kind, content))| {
                ImagePack::new(
                    ImagePackSource {
                        room: room.clone(),
                        state_key,
                        kind,
                    },
                    content,
                )
            })
            .collect()
    }

    /// Every image pack defined in a room that the user is in.
    ///
    /// This is what the pack management presents: a pack lives in a room, and
    /// the room it lives in is not necessarily one the user has open.
    pub(crate) async fn all_packs(&self) -> Vec<ImagePack> {
        let Some(session) = self.session() else {
            return Vec::new();
        };

        let room_list = session.room_list();
        let mut packs = Vec::new();
        let mut seen = HashSet::new();

        for room in room_list.iter::<Room>().flatten() {
            if room.matrix_room().state() != RoomState::Joined {
                continue;
            }

            for pack in Self::room_packs(&room).await {
                seen.insert((
                    pack.source().room.room_id().to_owned(),
                    pack.source().state_key.clone(),
                ));
                packs.push(pack);
            }
        }

        // The loop above only sees what sync happened to bring to this device.
        // The state of a quiet room can predate the store, and then a pack in
        // it is invisible until the room is opened. The packs that are used
        // everywhere are the ones the user acts on here, and where they are is
        // known, so those are asked for rather than waited for.
        for (room_id, state_keys) in self.imp().enabled_packs() {
            let Some(room) = room_list.get(&room_id) else {
                continue;
            };

            for state_key in state_keys.into_keys() {
                if seen.contains(&(room_id.clone(), state_key.clone())) {
                    continue;
                }

                if let Some(pack) = fetch_room_pack(&session, &room, state_key).await {
                    packs.push(pack);
                }
            }
        }

        packs
    }

    /// A state key that no image pack of the given room uses yet.
    ///
    /// The specification does not reserve the empty state key, but the clients
    /// in the wild use it for the pack of a room, so it is taken first.
    pub(crate) async fn unused_state_key(room: &Room) -> String {
        let taken = room_state_packs(room).await;

        if !taken.contains_key(FIRST_STATE_KEY) {
            return FIRST_STATE_KEY.to_owned();
        }

        // Among the state keys that are taken, at most all of them can
        // collide, so one of this many is free.
        (2..=taken.len() + 2)
            .map(|index| format!("{STATE_KEY_PREFIX}-{index}"))
            .find(|key| !taken.contains_key(key))
            .expect("an unused state key should be found")
    }

    /// The room that image packs are created in, creating it if there is none.
    ///
    /// The specification has no personal pack: it expects one to be a pack in
    /// a room, enabled everywhere. A room of one is therefore where a pack of
    /// your own belongs, and sharing it is inviting someone to that room.
    pub(crate) async fn packs_room(&self) -> Result<Room, ()> {
        let Some(session) = self.session() else {
            return Err(());
        };

        if let Some(room) = self.stored_packs_room().await {
            return Ok(room);
        }

        let client = session.client();
        let request = assign!(create_room::v3::Request::new(), {
            name: Some(gettext("Sticker Packs")),
            topic: Some(gettext("The sticker and emoticon packs that you created. Invite someone here to share them.")),
            preset: Some(RoomPreset::PrivateChat),
            visibility: Visibility::Private,
        });

        let handle = spawn_tokio!(async move {
            let matrix_room = client.create_room(request).await?;

            // The room is a container, not a conversation, so it is kept out
            // of the way of the rooms that are.
            if let Err(error) = matrix_room
                .set_tag(TagName::LowPriority, TagInfo::new())
                .await
            {
                warn!("Could not set the tag of the image packs room: {error}");
            }

            Ok::<_, matrix_sdk::Error>(matrix_room.room_id().to_owned())
        });

        let room_id = match handle.await.expect("task was not aborted") {
            Ok(room_id) => room_id,
            Err(error) => {
                error!("Could not create the image packs room: {error}");
                return Err(());
            }
        };

        let Some(room) = session.room_list().get_wait(&room_id, None).await else {
            error!("Could not find the image packs room that was just created");
            return Err(());
        };

        let client = session.client();
        let content = ImagePacksRoomEventContent {
            room_id: room_id.clone(),
        };
        let handle = spawn_tokio!(async move { client.account().set_account_data(content).await });

        if let Err(error) = handle.await.expect("task was not aborted") {
            // The room exists either way, it is only not found again next
            // time, and another one is created.
            warn!("Could not remember the image packs room: {error}");
        }

        Ok(room)
    }

    /// The room that image packs are created in, if it is known and joined.
    async fn stored_packs_room(&self) -> Option<Room> {
        let session = self.session()?;

        let client = session.client();
        let handle = spawn_tokio!(async move {
            client
                .account()
                .account_data::<ImagePacksRoomEventContent>()
                .await
        });

        let room_id = match handle.await.expect("task was not aborted") {
            Ok(Some(raw)) => match raw.deserialize() {
                Ok(content) => content.room_id,
                Err(error) => {
                    error!("Could not deserialize the image packs room: {error}");
                    return None;
                }
            },
            Ok(None) => return None,
            Err(error) => {
                error!("Could not get the image packs room: {error}");
                return None;
            }
        };

        session
            .room_list()
            .get(&room_id)
            .filter(|room| room.matrix_room().state() == RoomState::Joined)
    }

    /// Whether the pack with the given state key in the given room is enabled
    /// globally.
    pub(crate) fn is_pack_enabled(&self, room_id: &RoomId, state_key: &str) -> bool {
        let imp = self.imp();

        imp.enabled_packs_unstable
            .borrow()
            .get(room_id)
            .is_some_and(|packs| packs.contains_key(state_key))
            || imp
                .enabled_packs_stable
                .borrow()
                .get(room_id)
                .is_some_and(|packs| packs.contains_key(state_key))
    }

    /// Enable or disable the pack with the given state key in the given room,
    /// globally.
    ///
    /// Enabling writes the unstable event, which is the one we send. Disabling
    /// has to update whichever events hold the pack, so that it does not come
    /// back on the next load.
    pub(crate) async fn set_pack_enabled(
        &self,
        room_id: &RoomId,
        state_key: &str,
        enabled: bool,
    ) -> Result<(), ()> {
        if self.is_pack_enabled(room_id, state_key) == enabled {
            return Ok(());
        }

        let Some(session) = self.session() else {
            return Err(());
        };
        let imp = self.imp();

        // Read, modify and write, so that the properties we do not know about
        // are preserved, as the specification requires.
        let mut unstable = imp.enabled_packs_unstable.borrow().clone();
        let mut stable = imp.enabled_packs_stable.borrow().clone();

        // The stable event only needs to be written when it is the one that
        // holds the pack that is being disabled.
        let write_stable = !enabled
            && stable
                .get(room_id)
                .is_some_and(|packs| packs.contains_key(state_key));

        if enabled {
            unstable
                .entry(room_id.to_owned())
                .or_default()
                .entry(state_key.to_owned())
                .or_default();
        } else {
            for packs in [&mut unstable, &mut stable] {
                if let Some(room_packs) = packs.get_mut(room_id) {
                    room_packs.remove(state_key);

                    if room_packs.is_empty() {
                        packs.remove(room_id);
                    }
                }
            }
        }

        let client = session.client();
        let unstable_clone = unstable.clone();
        let stable_clone = stable.clone();
        let handle = spawn_tokio!(async move {
            let account = client.account();

            account
                .set_account_data(EmoteRoomsEventContent {
                    rooms: unstable_clone,
                })
                .await?;

            if write_stable {
                account
                    .set_account_data(ImagePackRoomsEventContent {
                        rooms: stable_clone,
                    })
                    .await?;
            }

            Ok::<_, matrix_sdk::Error>(())
        });

        if let Err(error) = handle.await.expect("task was not aborted") {
            error!("Could not change the enabled image packs: {error}");
            return Err(());
        }

        imp.enabled_packs_unstable.replace(unstable);
        imp.enabled_packs_stable.replace(stable);

        self.emit_by_name::<()>("changed", &[]);

        Ok(())
    }

    /// Save the given content as the image pack at the given source.
    ///
    /// A pack is written back under the event type that it was read from, so
    /// that editing a pack that another client created does not leave a second
    /// copy of it behind under the other name.
    pub(crate) async fn save_pack(
        &self,
        source: &ImagePackSource,
        content: PackContent,
    ) -> Result<(), ()> {
        let matrix_room = source.room.matrix_room().clone();
        let state_key = source.state_key.clone();
        let kind = source.kind;

        let handle = spawn_tokio!(async move {
            match kind {
                RoomPackKind::Unstable => {
                    matrix_room
                        .send_state_event_for_key(
                            &state_key,
                            RoomEmotesEventContent { pack: content },
                        )
                        .await
                }
                RoomPackKind::Stable => {
                    matrix_room
                        .send_state_event_for_key(
                            &state_key,
                            RoomImagePackEventContent { pack: content },
                        )
                        .await
                }
            }
        });

        if let Err(error) = handle.await.expect("task was not aborted") {
            error!("Could not save an image pack: {error}");
            return Err(());
        }

        self.emit_by_name::<()>("changed", &[]);

        Ok(())
    }

    /// Delete the image pack at the given source.
    ///
    /// A state event cannot be removed, so a deleted pack is one with no
    /// images, which is also what a redacted pack looks like.
    pub(crate) async fn delete_pack(&self, source: &ImagePackSource) -> Result<(), ()> {
        self.save_pack(source, PackContent::default()).await?;

        // A pack that does not exist anymore should not stay in the list of the
        // packs that are used everywhere. Failing to clean that up does not
        // make the deletion fail: the pack is gone either way, and it is
        // reported as unavailable in the pack list.
        let _ = self
            .set_pack_enabled(source.room.room_id(), &source.state_key, false)
            .await;

        Ok(())
    }

    /// Connect to the signal emitted when the image packs changed.
    pub(crate) fn connect_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

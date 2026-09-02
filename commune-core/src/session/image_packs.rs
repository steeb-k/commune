//! The [image packs] available to a session.
//!
//! The headless counterpart of the application's `ImagePacks` `GObject`
//! (`src/session/image_packs/mod.rs`) and its `ImagePack` value: the packs
//! enabled everywhere, watched under both event names; the packs of a room,
//! read from the state store under both names and written back under the
//! one they were read from; the room this client creates packs in; and the
//! rule that a pack with no images is a deleted one. The event types were
//! the core's already, in [`crate::events::image_packs`].
//!
//! What stayed in the application: the `gio::ListStore` of a pack's
//! images, the pill an emoticon becomes while a message is composed, the
//! thumbnail loader, the pack editor and every sentence — including the
//! name and topic of the packs room, which the embedder hands the core
//! through [`crate::config`] because they are written into a room on the
//! server and never translated again.
//!
//! [image packs]: https://spec.matrix.org/v1.19/client-server-api/#image-packs

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use eyeball::{SharedObservable, Subscriber};
use indexmap::IndexMap;
use matrix_sdk::{
    RoomState, deserialized_responses::RawAnySyncOrStrippedState,
    event_handler::EventHandlerDropGuard,
};
use ruma::{
    OwnedRoomId, RoomId,
    api::client::{
        room::{Visibility, create_room, create_room::v3::RoomPreset},
        state::get_state_event_for_key,
    },
    assign,
    events::{
        StaticEventContent,
        sticker::StickerEventContent,
        tag::{TagInfo, TagName},
    },
};
use tracing::{debug, error, warn};

use super::{Room, WeakSession};
use crate::{
    RUNTIME, UserFacingError, config,
    events::image_packs::{
        EmoteRoomsEvent, EmoteRoomsEventContent, EnabledPacks, ImagePackRoomsEvent,
        ImagePackRoomsEventContent, ImagePacksRoomEventContent, PackContent, PackImage, PackUsage,
        RoomEmotesEventContent, RoomImagePackEventContent, SyncRoomEmotesEvent,
        SyncRoomImagePackEvent, is_valid_shortcode,
    },
    spawn_tokio,
    utils::LoadingState,
};

/// The state key of the first image pack of a room.
const FIRST_STATE_KEY: &str = "";

/// The prefix of the state keys of the image packs after the first one.
const STATE_KEY_PREFIX: &str = "pack";

/// The event types of a room image pack, in the order in which they are read.
///
/// The stable type comes last so that it wins over the unstable one, since it
/// is the one that we send.
const ROOM_PACK_TYPES: &[(RoomPackKind, &str)] = &[
    (RoomPackKind::Unstable, RoomEmotesEventContent::TYPE),
    (RoomPackKind::Stable, RoomImagePackEventContent::TYPE),
];

/// What can go wrong while changing image packs.
#[derive(Debug, thiserror::Error)]
pub enum ImagePacksError {
    /// The session these packs belong to is gone.
    #[error("the session is no longer available")]
    NoSession,
    /// The room that packs are created in could not be created, or was
    /// not found once created.
    #[error("the room to keep the packs in could not be created")]
    PacksRoom,
    /// There is no room that packs are created in, so there is no pack of
    /// this account to change.
    #[error("there is no room to keep the packs in")]
    NoPacksRoom,
    /// No pack has the given state key.
    #[error("no pack has that identifier")]
    UnknownPack,
    /// A shortcode does not follow the grammar of the specification.
    #[error("the shortcode is not valid")]
    InvalidShortcode,
    /// The homeserver refused.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl UserFacingError for ImagePacksError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NoSession => "The session is no longer available.".to_owned(),
            Self::PacksRoom => "Could not create the room to keep your packs in".to_owned(),
            Self::NoPacksRoom => "There is no pack to change yet".to_owned(),
            Self::UnknownPack => "Could not find the pack".to_owned(),
            Self::InvalidShortcode => {
                "A shortcode can only contain letters, digits, dashes and underscores".to_owned()
            }
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// The event type that an image pack in the state of a room is defined under.
///
/// A pack is written back under the type that it was read from, so that
/// editing a pack that another client defined does not leave a second copy of
/// it behind under the other name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomPackKind {
    /// `im.ponies.room_emotes`, from MSC2545, which we read but do not
    /// create.
    Unstable,
    /// `m.room.image_pack`, from the specification, which is the one we
    /// create.
    Stable,
}

/// Where an image pack comes from.
///
/// Always the state of a room: the specification has no personal pack, and
/// expects one to be a room pack enabled everywhere instead.
#[derive(Debug, Clone)]
pub struct ImagePackSource {
    /// The room that defines the pack.
    pub room: Room,
    /// The state key that identifies the pack in that room.
    pub state_key: String,
    /// The event type that the pack is defined under.
    pub kind: RoomPackKind,
}

/// An image pack, as a value.
///
/// The content of a pack is not changed in place: an editor works on a
/// copy and sends the result, and the pack is built again from what comes
/// back through sync.
#[derive(Debug, Clone)]
pub struct ImagePack {
    /// Where this pack comes from.
    pub source: ImagePackSource,
    /// The content of this pack.
    pub content: PackContent,
}

impl ImagePack {
    /// The name of this pack, as shown to the user.
    ///
    /// The pack's own name, else the name of the room that defines it —
    /// `None` when the room's name is one of the sentences the interface
    /// makes itself, since a `RoomDisplayName` that is not `Named` is the
    /// UI's to render.
    #[must_use]
    pub fn display_name(&self) -> Option<String> {
        if let Some(display_name) = &self.content.pack.display_name {
            return Some(display_name.clone());
        }

        match self.source.room.display_name() {
            super::RoomDisplayName::Named(name) => Some(name),
            _ => None,
        }
    }

    /// Who to credit for this pack.
    #[must_use]
    pub fn attribution(&self) -> Option<&str> {
        self.content.pack.attribution.as_deref()
    }

    /// Whether this pack can be used for the given usage.
    #[must_use]
    pub fn has_usage(&self, usage: &PackUsage) -> bool {
        self.content.pack.has_usage(usage)
    }

    /// Whether this pack has no images.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.content.images.is_empty()
    }
}

/// The content to send the given image of a pack as a sticker — the
/// application's `PackImage::sticker_content`.
#[must_use]
pub fn sticker_content(shortcode: &str, image: &PackImage) -> StickerEventContent {
    StickerEventContent::new(
        image.body.clone().unwrap_or_else(|| shortcode.to_owned()),
        image.info.as_deref().cloned().unwrap_or_default(),
        image.url.clone(),
    )
}

/// An image pack that is used everywhere but cannot be loaded, because the
/// user is not in the room that defines it anymore.
#[derive(Debug, Clone)]
pub struct UnavailablePack {
    /// The room that defines the pack.
    pub room_id: OwnedRoomId,
    /// The state key that identifies the pack in that room.
    pub state_key: String,
}

/// The image packs available to a session.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct ImagePacks {
    inner: Arc<ImagePacksInner>,
}

#[derive(Debug)]
struct ImagePacksInner {
    /// The session that these image packs belong to.
    session: WeakSession,
    /// The room image packs enabled globally, under the unstable name.
    ///
    /// This is the one we read from. The two are kept apart so that
    /// disabling a pack can remove it from whichever event holds it.
    enabled_packs_unstable: Mutex<EnabledPacks>,
    /// The room image packs enabled globally, under the stable name.
    enabled_packs_stable: Mutex<EnabledPacks>,
    /// How far the first read of the enabled packs has got.
    state: SharedObservable<LoadingState>,
    /// Bumped whenever the packs changed — the application's `changed`
    /// signal, as a counter a subscriber can wait on.
    changed: SharedObservable<u64>,
    /// The SDK event handlers following the account data and the packs.
    drop_guards: Mutex<Vec<EventHandlerDropGuard>>,
}

impl ImagePacks {
    /// Create the image packs of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        Self {
            inner: Arc::new(ImagePacksInner {
                session,
                enabled_packs_unstable: Mutex::new(EnabledPacks::new()),
                enabled_packs_stable: Mutex::new(EnabledPacks::new()),
                state: SharedObservable::new(LoadingState::Initial),
                changed: SharedObservable::new(0),
                drop_guards: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Load the account data and watch it for changes.
    pub async fn load(&self) {
        self.load_enabled_packs().await;
        self.watch();
    }

    /// Load the account data unless it has already been loaded.
    pub async fn ensure_loaded(&self) {
        if self.inner.state.get() == LoadingState::Ready {
            return;
        }
        self.load().await;
    }

    /// How far the first read of the enabled packs has got.
    #[must_use]
    pub fn state(&self) -> LoadingState {
        self.inner.state.get()
    }

    /// A counter bumped whenever the packs changed: the enabled packs, or a
    /// pack in the state of a room.
    pub fn subscribe_changed(&self) -> Subscriber<u64> {
        self.inner.changed.subscribe()
    }

    /// Say that the packs changed.
    fn notify_changed(&self) {
        self.inner
            .changed
            .update(|count| *count = count.wrapping_add(1));
    }

    /// Watch the account data and the packs of every room for changes.
    ///
    /// The enabled packs are watched under both names, because an event
    /// handler only matches the single type of the content that it takes.
    /// The packs in the state of a room are not kept here, because they are
    /// read from the state store when they are needed, but a change to one
    /// of them still has to be announced, under both names.
    fn watch(&self) {
        let mut guards = self
            .inner
            .drop_guards
            .lock()
            .expect("mutex is not poisoned");
        if !guards.is_empty() {
            return;
        }

        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        let client = session.client();

        let weak = Arc::downgrade(&self.inner);
        let unstable_handle = client.add_event_handler(move |_: EmoteRoomsEvent| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade() {
                    let packs = ImagePacks { inner };
                    packs.load_enabled_packs().await;
                    packs.notify_changed();
                }
            }
        });

        let weak = Arc::downgrade(&self.inner);
        let stable_handle = client.add_event_handler(move |_: ImagePackRoomsEvent| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade() {
                    let packs = ImagePacks { inner };
                    packs.load_enabled_packs().await;
                    packs.notify_changed();
                }
            }
        });

        let weak = Arc::downgrade(&self.inner);
        let unstable_room_handle = client.add_event_handler(move |_: SyncRoomEmotesEvent| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade() {
                    ImagePacks { inner }.notify_changed();
                }
            }
        });

        let weak = Arc::downgrade(&self.inner);
        let stable_room_handle = client.add_event_handler(move |_: SyncRoomImagePackEvent| {
            let weak = weak.clone();
            async move {
                if let Some(inner) = weak.upgrade() {
                    ImagePacks { inner }.notify_changed();
                }
            }
        });

        *guards = [
            unstable_handle,
            stable_handle,
            unstable_room_handle,
            stable_room_handle,
        ]
        .into_iter()
        .map(|handle| client.event_handler_drop_guard(handle))
        .collect();
    }

    /// Load the globally enabled room image packs from the store.
    async fn load_enabled_packs(&self) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };

        self.inner.state.set_if_not_eq(LoadingState::Loading);

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
                Err(deserialize_error) => {
                    error!("Could not deserialize the enabled image packs: {deserialize_error}");
                    self.inner.state.set_if_not_eq(LoadingState::Error);
                    return;
                }
            },
            Ok(None) => EnabledPacks::new(),
            Err(load_error) => {
                error!("Could not get the enabled image packs: {load_error}");
                self.inner.state.set_if_not_eq(LoadingState::Error);
                return;
            }
        };

        let unstable = match unstable {
            Ok(Some(raw)) => match raw.deserialize() {
                Ok(content) => content.rooms,
                Err(deserialize_error) => {
                    error!("Could not deserialize the enabled image packs: {deserialize_error}");
                    self.inner.state.set_if_not_eq(LoadingState::Error);
                    return;
                }
            },
            Ok(None) => EnabledPacks::new(),
            Err(load_error) => {
                error!("Could not get the enabled image packs: {load_error}");
                self.inner.state.set_if_not_eq(LoadingState::Error);
                return;
            }
        };

        *self
            .inner
            .enabled_packs_stable
            .lock()
            .expect("mutex is not poisoned") = stable;
        *self
            .inner
            .enabled_packs_unstable
            .lock()
            .expect("mutex is not poisoned") = unstable;
        self.inner.state.set_if_not_eq(LoadingState::Ready);
    }

    /// The room image packs enabled globally, under both names.
    ///
    /// A pack listed under both keeps the metadata of the stable event,
    /// which is the one we write.
    #[must_use]
    pub fn enabled_packs(&self) -> EnabledPacks {
        let mut enabled_packs = self
            .inner
            .enabled_packs_unstable
            .lock()
            .expect("mutex is not poisoned")
            .clone();

        for (room_id, packs) in &*self
            .inner
            .enabled_packs_stable
            .lock()
            .expect("mutex is not poisoned")
        {
            enabled_packs
                .entry(room_id.clone())
                .or_default()
                .extend(packs.iter().map(|(key, meta)| (key.clone(), meta.clone())));
        }

        enabled_packs
    }

    /// The image packs that the user can use in the given room, for the given
    /// usage, or for every usage when it is `None`.
    ///
    /// They are in the order the specification asks for: the packs that the
    /// user enabled everywhere, then the packs of the room. A pack that is
    /// both enabled everywhere and defined in the room is only returned once.
    ///
    /// The packs of the canonical space of the room are not included yet.
    pub async fn packs_for_room(&self, room: &Room, usage: Option<&PackUsage>) -> Vec<ImagePack> {
        let mut packs = self.enabled_everywhere(usage).await;

        let seen = packs
            .iter()
            .map(|pack| {
                (
                    pack.source.room.room_id().to_owned(),
                    pack.source.state_key.clone(),
                )
            })
            .collect::<Vec<_>>();

        let room_id = room.room_id().to_owned();
        for (state_key, (kind, content)) in room_state_packs(room).await {
            if seen.contains(&(room_id.clone(), state_key.clone())) {
                continue;
            }

            let pack = ImagePack {
                source: ImagePackSource {
                    room: room.clone(),
                    state_key,
                    kind,
                },
                content,
            };
            if !pack.is_empty() && usage.is_none_or(|usage| pack.has_usage(usage)) {
                packs.push(pack);
            }
        }

        packs
    }

    /// The image packs that the user enabled everywhere, for the given
    /// usage, or for every usage when it is `None` — the first half of
    /// [`Self::packs_for_room`], for a caller with no room to name.
    pub async fn enabled_everywhere(&self, usage: Option<&PackUsage>) -> Vec<ImagePack> {
        let mut packs = Vec::new();

        let Some(session) = self.inner.session.upgrade() else {
            return packs;
        };
        let room_list = session.room_list();

        for (room_id, state_keys) in self.enabled_packs() {
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

                let pack = ImagePack {
                    source: ImagePackSource {
                        room: source_room.clone(),
                        state_key,
                        kind,
                    },
                    content,
                };
                if !pack.is_empty() && usage.is_none_or(|usage| pack.has_usage(usage)) {
                    packs.push(pack);
                }
            }
        }

        packs
    }

    /// The image packs that are used everywhere but cannot be loaded.
    ///
    /// The specification expects clients to be aware that the user might not
    /// be in the room that defines a pack anymore, so that they can be told
    /// about it and stop using it.
    #[must_use]
    pub fn unavailable_packs(&self) -> Vec<UnavailablePack> {
        let Some(session) = self.inner.session.upgrade() else {
            return Vec::new();
        };
        let room_list = session.room_list();

        let mut packs = Vec::new();

        for (room_id, state_keys) in self.enabled_packs() {
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
    pub async fn room_packs(room: &Room) -> Vec<ImagePack> {
        room_state_packs(room)
            .await
            .into_iter()
            .map(|(state_key, (kind, content))| ImagePack {
                source: ImagePackSource {
                    room: room.clone(),
                    state_key,
                    kind,
                },
                content,
            })
            .collect()
    }

    /// Every image pack defined in a room that the user is in.
    ///
    /// This is what the pack management presents: a pack lives in a room, and
    /// the room it lives in is not necessarily one the user has open.
    pub async fn all_packs(&self) -> Vec<ImagePack> {
        let Some(session) = self.inner.session.upgrade() else {
            return Vec::new();
        };

        let room_list = session.room_list();
        let mut packs = Vec::new();
        let mut seen = HashSet::new();

        for room in room_list.snapshot() {
            if room.matrix_room().state() != RoomState::Joined {
                continue;
            }

            for pack in Self::room_packs(&room).await {
                seen.insert((
                    pack.source.room.room_id().to_owned(),
                    pack.source.state_key.clone(),
                ));
                packs.push(pack);
            }
        }

        // The loop above only sees what sync happened to bring to this device.
        // The state of a quiet room can predate the store, and then a pack in
        // it is invisible until the room is opened. The packs that are used
        // everywhere are the ones the user acts on here, and where they are is
        // known, so those are asked for rather than waited for.
        for (room_id, state_keys) in self.enabled_packs() {
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
    pub async fn unused_state_key(room: &Room) -> String {
        let taken = room_state_packs_including_empty(room).await;

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
    pub async fn packs_room(&self) -> Result<Room, ImagePacksError> {
        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(ImagePacksError::NoSession)?;

        if let Some(room) = self.stored_packs_room().await {
            return Ok(room);
        }

        let client = session.client();
        let request = assign!(create_room::v3::Request::new(), {
            name: Some(config::packs_room_name().to_owned()),
            topic: Some(config::packs_room_topic().to_owned()),
            preset: Some(RoomPreset::PrivateChat),
            visibility: Visibility::Private,
        });

        let handle = spawn_tokio!(async move {
            let matrix_room = client.create_room(request).await?;

            // The room is a container, not a conversation, so it is kept out
            // of the way of the rooms that are.
            if let Err(tag_error) = matrix_room
                .set_tag(TagName::LowPriority, TagInfo::new())
                .await
            {
                warn!("Could not set the tag of the image packs room: {tag_error}");
            }

            Ok::<_, matrix_sdk::Error>(matrix_room.room_id().to_owned())
        });

        let room_id = match handle.await.expect("task was not aborted") {
            Ok(room_id) => room_id,
            Err(create_error) => {
                error!("Could not create the image packs room: {create_error}");
                return Err(ImagePacksError::Server(Box::new(create_error)));
            }
        };

        let Some(room) = session.room_list().get_wait(&room_id, None).await else {
            error!("Could not find the image packs room that was just created");
            return Err(ImagePacksError::PacksRoom);
        };

        let client = session.client();
        let content = ImagePacksRoomEventContent {
            room_id: room_id.clone(),
        };
        let handle = spawn_tokio!(async move { client.account().set_account_data(content).await });

        if let Err(remember_error) = handle.await.expect("task was not aborted") {
            // The room exists either way, it is only not found again next
            // time, and another one is created.
            warn!("Could not remember the image packs room: {remember_error}");
        }

        Ok(room)
    }

    /// The room that image packs are created in, if it is known and joined.
    pub async fn stored_packs_room(&self) -> Option<Room> {
        let session = self.inner.session.upgrade()?;

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
                Err(deserialize_error) => {
                    error!("Could not deserialize the image packs room: {deserialize_error}");
                    return None;
                }
            },
            Ok(None) => return None,
            Err(load_error) => {
                error!("Could not get the image packs room: {load_error}");
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
    #[must_use]
    pub fn is_pack_enabled(&self, room_id: &RoomId, state_key: &str) -> bool {
        self.inner
            .enabled_packs_unstable
            .lock()
            .expect("mutex is not poisoned")
            .get(room_id)
            .is_some_and(|packs| packs.contains_key(state_key))
            || self
                .inner
                .enabled_packs_stable
                .lock()
                .expect("mutex is not poisoned")
                .get(room_id)
                .is_some_and(|packs| packs.contains_key(state_key))
    }

    /// Enable or disable the pack with the given state key in the given room,
    /// globally.
    ///
    /// Enabling writes the stable event, which is the one we send. Disabling
    /// has to update whichever events hold the pack, so that it does not come
    /// back on the next load.
    pub async fn set_pack_enabled(
        &self,
        room_id: &RoomId,
        state_key: &str,
        enabled: bool,
    ) -> Result<(), ImagePacksError> {
        if self.is_pack_enabled(room_id, state_key) == enabled {
            return Ok(());
        }

        let session = self
            .inner
            .session
            .upgrade()
            .ok_or(ImagePacksError::NoSession)?;

        // Read, modify and write, so that the properties we do not know about
        // are preserved, as the specification requires.
        let mut unstable = self
            .inner
            .enabled_packs_unstable
            .lock()
            .expect("mutex is not poisoned")
            .clone();
        let mut stable = self
            .inner
            .enabled_packs_stable
            .lock()
            .expect("mutex is not poisoned")
            .clone();

        // The unstable event only needs to be written when it is the one that
        // holds the pack that is being disabled.
        let write_unstable = !enabled
            && unstable
                .get(room_id)
                .is_some_and(|packs| packs.contains_key(state_key));

        if enabled {
            stable
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
                .set_account_data(ImagePackRoomsEventContent {
                    rooms: stable_clone,
                })
                .await?;

            if write_unstable {
                account
                    .set_account_data(EmoteRoomsEventContent {
                        rooms: unstable_clone,
                    })
                    .await?;
            }

            Ok::<_, matrix_sdk::Error>(())
        });

        if let Err(set_error) = handle.await.expect("task was not aborted") {
            error!("Could not change the enabled image packs: {set_error}");
            return Err(ImagePacksError::Server(Box::new(set_error)));
        }

        *self
            .inner
            .enabled_packs_unstable
            .lock()
            .expect("mutex is not poisoned") = unstable;
        *self
            .inner
            .enabled_packs_stable
            .lock()
            .expect("mutex is not poisoned") = stable;

        self.notify_changed();

        Ok(())
    }

    /// Save the given content as the image pack at the given source.
    ///
    /// A pack is written back under the event type that it was read from, so
    /// that editing a pack that another client created does not leave a second
    /// copy of it behind under the other name.
    pub async fn save_pack(
        &self,
        source: &ImagePackSource,
        content: PackContent,
    ) -> Result<(), ImagePacksError> {
        if content
            .images
            .keys()
            .any(|shortcode| !is_valid_shortcode(shortcode))
        {
            return Err(ImagePacksError::InvalidShortcode);
        }

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

        if let Err(save_error) = handle.await.expect("task was not aborted") {
            error!("Could not save an image pack: {save_error}");
            return Err(ImagePacksError::Server(Box::new(save_error)));
        }

        self.notify_changed();

        Ok(())
    }

    /// Delete the image pack at the given source.
    ///
    /// A state event cannot be removed, so a deleted pack is one with no
    /// images, which is also what a redacted pack looks like.
    pub async fn delete_pack(&self, source: &ImagePackSource) -> Result<(), ImagePacksError> {
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
}

/// Read the image packs defined in the state of the given room.
///
/// Packs are keyed by their state key, and carry the event type that they were
/// read from, so that they can be written back under the same one. A pack
/// defined under both the stable and the unstable event type is only returned
/// once. A redacted pack, or a pack that was deleted, has no images and is
/// left out.
pub async fn room_state_packs(room: &Room) -> IndexMap<String, (RoomPackKind, PackContent)> {
    let mut packs = room_state_packs_including_empty(room).await;
    packs.retain(|_, (_, content)| !content.images.is_empty());
    packs
}

/// Read the image packs defined in the state of the given room, the empty
/// ones included.
///
/// The application never presents an empty pack: to it, a pack with no
/// images is a deleted one. A caller that creates a pack before it has an
/// image to put in it — which the FFI does — needs to see it again, and
/// this is the read that lets it.
pub(crate) async fn room_state_packs_including_empty(
    room: &Room,
) -> IndexMap<String, (RoomPackKind, PackContent)> {
    let matrix_room = room.matrix_room().clone();

    let handle = spawn_tokio!(async move {
        let mut raw_events = Vec::new();

        for (kind, event_type) in ROOM_PACK_TYPES {
            match matrix_room.get_state_events((*event_type).into()).await {
                Ok(events) => raw_events.extend(events.into_iter().map(|event| (*kind, event))),
                Err(read_error) => {
                    error!("Could not get the image packs of a room: {read_error}");
                }
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
            Err(read_error) => {
                error!("Could not read the state key of an image pack: {read_error}");
                continue;
            }
        };

        let content = match raw_event.get_field::<PackContent>("content") {
            Ok(Some(content)) => content,
            Ok(None) => continue,
            Err(deserialize_error) => {
                error!("Could not deserialize an image pack: {deserialize_error}");
                continue;
            }
        };

        packs.insert(state_key, (kind, content));
    }

    packs
}

/// Ask the homeserver for the image pack with the given state key in the
/// given room.
///
/// Used for a pack that the state store does not have, which happens when the
/// state of the room never reached this device.
async fn fetch_room_pack(
    session: &super::Session,
    room: &Room,
    state_key: String,
) -> Option<ImagePack> {
    let client = session.client();
    let room_id = room.room_id().to_owned();
    let key = state_key.clone();

    let handle = spawn_tokio!(async move {
        // The stable type comes first, so that it wins over the unstable one,
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
                Err(deserialize_error) => {
                    error!("Could not deserialize an image pack: {deserialize_error}");
                }
            }
        }

        None
    });

    let (kind, content) = handle.await.expect("task was not aborted")?;

    Some(ImagePack {
        source: ImagePackSource {
            room: room.clone(),
            state_key,
            kind,
        },
        content,
    })
}

/// Start loading the packs of the given session on the runtime, for a
/// `Session` accessor that cannot await.
pub(crate) fn spawn_load(packs: &ImagePacks) {
    let packs = packs.clone();
    RUNTIME.spawn(async move {
        packs.load().await;
    });
}

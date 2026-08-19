//! The [image packs] available to a session.
//!
//! [image packs]: https://spec.matrix.org/v1.19/client-server-api/#image-packs

use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use indexmap::IndexMap;
use matrix_sdk::deserialized_responses::RawAnySyncOrStrippedState;
use ruma::{OwnedRoomId, RoomId, events::StaticEventContent};
use tracing::{debug, error};

mod events;
mod image_pack;
mod pack_image;

use self::events::{
    EmoteRoomsEvent, EmoteRoomsEventContent, EnabledPacks, ImagePackRoomsEvent,
    ImagePackRoomsEventContent, PackContent, RoomEmotesEventContent, RoomImagePackEventContent,
    UserEmotesEvent, UserEmotesEventContent,
};
pub(crate) use self::{
    events::PackUsage,
    image_pack::{ImagePack, ImagePackSource},
    pack_image::PackImage,
};
use super::{Room, Session};
use crate::{spawn, spawn_tokio};

/// The event types of a room image pack, in the order in which they are read.
///
/// The unstable type comes last so that it wins over the stable one, since it
/// is the one that we send.
const ROOM_PACK_TYPES: &[&str] = &[
    RoomImagePackEventContent::TYPE,
    RoomEmotesEventContent::TYPE,
];

/// Read the image packs defined in the state of the given room.
///
/// Packs are keyed by their state key. A pack defined under both the stable
/// and the unstable event type is only returned once.
async fn room_state_packs(room: &Room) -> IndexMap<String, PackContent> {
    let matrix_room = room.matrix_room().clone();

    let handle = spawn_tokio!(async move {
        let mut raw_events = Vec::new();

        for event_type in ROOM_PACK_TYPES {
            match matrix_room.get_state_events((*event_type).into()).await {
                Ok(events) => raw_events.extend(events),
                Err(error) => error!("Could not get the image packs of a room: {error}"),
            }
        }

        raw_events
    });
    let raw_events = handle.await.expect("task was not aborted");

    let mut packs = IndexMap::new();

    for raw_event in raw_events {
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

        // A redacted pack has no images.
        if content.images.is_empty() {
            continue;
        }

        packs.insert(state_key, content);
    }

    packs
}

/// A room image pack that is enabled globally.
#[derive(Debug, Clone)]
pub(crate) enum EnabledPack {
    /// A pack that we could load.
    Available(ImagePack),
    /// A pack that we could not load, because the user is not in the room
    /// that defines it anymore.
    Unavailable {
        /// The room that defines the pack.
        room_id: OwnedRoomId,
        /// The state key that identifies the pack in that room.
        state_key: String,
    },
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
        /// The personal image pack of the user.
        pub(super) user_pack: RefCell<Option<PackContent>>,
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

            self.load_user_pack().await;
            self.load_enabled_packs().await;

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let user_pack_handle = client.add_event_handler(move |_: UserEmotesEvent| {
                let obj_weak = obj_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.imp().load_user_pack().await;
                                obj.emit_by_name::<()>("changed", &[]);
                            }
                        });
                    });
                }
            });

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

            self.drop_guards.replace(
                [user_pack_handle, unstable_handle, stable_handle]
                    .into_iter()
                    .map(|handle| client.event_handler_drop_guard(handle))
                    .collect(),
            );
        }

        /// Load the personal image pack of the user from the store.
        pub(super) async fn load_user_pack(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let client = session.client();
            let handle = spawn_tokio!(async move {
                client
                    .account()
                    .account_data::<UserEmotesEventContent>()
                    .await
            });

            let content = match handle.await.expect("task was not aborted") {
                Ok(Some(raw)) => match raw.deserialize() {
                    Ok(content) => Some(content.pack),
                    Err(error) => {
                        error!("Could not deserialize the personal image pack: {error}");
                        return;
                    }
                },
                Ok(None) => {
                    debug!("Got no personal image pack");
                    None
                }
                Err(error) => {
                    error!("Could not get the personal image pack: {error}");
                    return;
                }
            };

            self.user_pack.replace(content);
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

    /// The personal image pack of the user, if they have one.
    pub(crate) fn user_pack(&self) -> Option<ImagePack> {
        let content = self.imp().user_pack.borrow().clone()?;
        Some(ImagePack::new(ImagePackSource::User, content))
    }

    /// The image packs that the user can use in the given room, for the given
    /// usage.
    ///
    /// They are in the order in which they should be presented: the personal
    /// pack of the user, then the packs that they enabled globally, then the
    /// packs of the room. A pack that is both enabled globally and defined in
    /// the room is only returned once.
    ///
    /// The packs of the canonical space of the room are not included yet.
    pub(crate) async fn packs_for_room(&self, room: &Room, usage: &PackUsage) -> Vec<ImagePack> {
        let mut packs = Vec::new();

        let mut push = |pack: ImagePack| {
            if !pack.is_empty() && pack.has_usage(usage) {
                packs.push(pack);
            }
        };

        if let Some(pack) = self.user_pack() {
            push(pack);
        }

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
                let Some(content) = room_packs.shift_remove(&state_key) else {
                    continue;
                };

                seen.push((room_id.clone(), state_key.clone()));
                push(ImagePack::new(
                    ImagePackSource::Room {
                        room: source_room.clone(),
                        state_key,
                    },
                    content,
                ));
            }
        }

        let room_id = room.room_id().to_owned();
        for (state_key, content) in room_state_packs(room).await {
            if seen.contains(&(room_id.clone(), state_key.clone())) {
                continue;
            }

            push(ImagePack::new(
                ImagePackSource::Room {
                    room: room.clone(),
                    state_key,
                },
                content,
            ));
        }

        packs
    }

    /// Every room image pack that is enabled globally.
    ///
    /// The specification expects clients to be aware that the user might not
    /// be in the room that defines a pack anymore, so those are returned too,
    /// to be able to remove them.
    pub(crate) async fn enabled_packs(&self) -> Vec<EnabledPack> {
        let Some(session) = self.session() else {
            return Vec::new();
        };
        let room_list = session.room_list();
        let enabled_packs = self.imp().enabled_packs();

        let mut packs = Vec::new();

        for (room_id, state_keys) in enabled_packs {
            let room = room_list.get(&room_id);

            let mut room_packs = match &room {
                Some(room) => room_state_packs(room).await,
                None => IndexMap::new(),
            };

            for state_key in state_keys.into_keys() {
                match (&room, room_packs.shift_remove(&state_key)) {
                    (Some(room), Some(content)) => {
                        packs.push(EnabledPack::Available(ImagePack::new(
                            ImagePackSource::Room {
                                room: room.clone(),
                                state_key,
                            },
                            content,
                        )))
                    }
                    _ => packs.push(EnabledPack::Unavailable {
                        room_id: room_id.clone(),
                        state_key,
                    }),
                }
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
            .map(|(state_key, content)| {
                ImagePack::new(
                    ImagePackSource::Room {
                        room: room.clone(),
                        state_key,
                    },
                    content,
                )
            })
            .collect()
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

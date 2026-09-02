//! The [image packs] available to a session.
//!
//! The packs are the core's ([`commune_core::session::ImagePacks`]): the packs
//! enabled everywhere, the packs of a room, the room this client creates
//! packs in, and the rule that a pack with no images is a deleted one. This
//! presents them: the `changed` signal, and each pack as an object with the
//! room the interface knows.
//!
//! [image packs]: https://spec.matrix.org/v1.19/client-server-api/#image-packs

use commune_core::session::{ImagePack as CoreImagePack, ImagePacks as CoreImagePacks};
use gtk::{glib, glib::closure_local, prelude::*, subclass::prelude::*};
use ruma::RoomId;
use tokio::task::AbortHandle;
use tracing::error;

mod emoticon_source;
mod image_pack;
mod pack_image;

// The `m.image_pack` and `m.emotes` event types, which are the core's: ruma
// `EventContent` definitions and a shortcode validator, with no `glib` type
// and no sentence anywhere in them. Bound to the name the module already
// used, so every path below reads unchanged. See `doc/track3-convergence.md`.
use commune_core::events::image_packs as events;
pub(crate) use commune_core::session::UnavailablePack;

pub(crate) use self::{
    emoticon_source::EmoticonSource,
    events::{
        PackContent, PackImage as PackImageData, PackUsage, SHORTCODE_MAX_LEN, is_valid_shortcode,
    },
    image_pack::{ImagePack, ImagePackSource, RoomPackKind},
    pack_image::PackImage,
};
use super::{Room, Session};
use crate::{core_bridge::ObjectWatcher, spawn_tokio};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ImagePacks)]
    pub struct ImagePacks {
        /// The session that these image packs belong to.
        #[property(get, construct_only)]
        pub(super) session: glib::WeakRef<Session>,
        /// The task following the core's packs.
        watch_handle: RefCell<Option<AbortHandle>>,
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

            self.watch_core();
        }

        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl ImagePacks {
        /// Follow the core's packs: a change to any of them is announced.
        ///
        /// The core reads the enabled packs when the session first asks for
        /// them, which the session does as it is set up.
        fn watch_core(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let core = session.core().image_packs();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(
                    core.subscribe_changed(),
                    |obj: &super::ImagePacks, _count| {
                        obj.emit_by_name::<()>("changed", &[]);
                    },
                )
                .spawn();
            self.watch_handle.replace(Some(handle));
        }
    }
}

glib::wrapper! {
    /// The image packs available to a [`Session`].
    ///
    /// The packs are the core's; this presents them.
    pub struct ImagePacks(ObjectSubclass<imp::ImagePacks>);
}

impl ImagePacks {
    /// Create a new `ImagePacks` for the given session.
    pub(crate) fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// The session and the core's packs, while the session is there.
    fn core(&self) -> Option<(Session, CoreImagePacks)> {
        let session = self.session()?;
        let core = session.core().image_packs().clone();
        Some((session, core))
    }

    /// The given packs of the core, as objects.
    fn present(session: &Session, packs: Vec<CoreImagePack>) -> Vec<ImagePack> {
        packs
            .into_iter()
            .filter_map(|pack| ImagePack::from_core(session, pack))
            .collect()
    }

    /// The image packs that the user can use in the given room, for the given
    /// usage, or for every usage when it is `None`.
    ///
    /// They are in the order the specification asks for: the packs that the
    /// user enabled everywhere, then the packs of the room. A pack that is
    /// both enabled everywhere and defined in the room is only returned once.
    pub(crate) async fn packs_for_room(
        &self,
        room: &Room,
        usage: Option<&PackUsage>,
    ) -> Vec<ImagePack> {
        let Some((session, core)) = self.core() else {
            return Vec::new();
        };
        let core_room = room.core().clone();
        let usage = usage.cloned();

        let handle =
            spawn_tokio!(async move { core.packs_for_room(&core_room, usage.as_ref()).await });
        let packs = handle.await.expect("task was not aborted");

        Self::present(&session, packs)
    }

    /// The image packs that are used everywhere but cannot be loaded.
    ///
    /// The specification expects clients to be aware that the user might not
    /// be in the room that defines a pack anymore, so that they can be told
    /// about it and stop using it.
    pub(crate) fn unavailable_packs(&self) -> Vec<UnavailablePack> {
        self.core()
            .map(|(_, core)| core.unavailable_packs())
            .unwrap_or_default()
    }

    /// Every image pack defined in a room that the user is in.
    ///
    /// This is what the pack management presents: a pack lives in a room, and
    /// the room it lives in is not necessarily one the user has open.
    pub(crate) async fn all_packs(&self) -> Vec<ImagePack> {
        let Some((session, core)) = self.core() else {
            return Vec::new();
        };

        let handle = spawn_tokio!(async move { core.all_packs().await });
        let packs = handle.await.expect("task was not aborted");

        Self::present(&session, packs)
    }

    /// A state key that no image pack of the given room uses yet.
    pub(crate) async fn unused_state_key(room: &Room) -> String {
        let core_room = room.core().clone();

        spawn_tokio!(async move { CoreImagePacks::unused_state_key(&core_room).await })
            .await
            .expect("task was not aborted")
    }

    /// The room that image packs are created in, creating it if there is none.
    ///
    /// The specification has no personal pack: it expects one to be a pack in
    /// a room, enabled everywhere. A room of one is therefore where a pack of
    /// your own belongs, and sharing it is inviting someone to that room.
    pub(crate) async fn packs_room(&self) -> Result<Room, ()> {
        let (session, core) = self.core().ok_or(())?;

        let handle = spawn_tokio!(async move { core.packs_room().await });
        let core_room = handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not get the image packs room: {error}");
            })?;

        // The core waited for the room to reach its list; the list presented
        // here follows that one, and may not have caught up yet.
        let Some(room) = session
            .room_list()
            .get_wait(core_room.room_id(), None)
            .await
        else {
            error!("Could not find the image packs room that was just created");
            return Err(());
        };

        Ok(room)
    }

    /// Whether the pack with the given state key in the given room is enabled
    /// globally.
    pub(crate) fn is_pack_enabled(&self, room_id: &RoomId, state_key: &str) -> bool {
        self.core()
            .is_some_and(|(_, core)| core.is_pack_enabled(room_id, state_key))
    }

    /// Enable or disable the pack with the given state key in the given room,
    /// globally.
    pub(crate) async fn set_pack_enabled(
        &self,
        room_id: &RoomId,
        state_key: &str,
        enabled: bool,
    ) -> Result<(), ()> {
        let (_, core) = self.core().ok_or(())?;
        let room_id = room_id.to_owned();
        let state_key = state_key.to_owned();

        let handle =
            spawn_tokio!(async move { core.set_pack_enabled(&room_id, &state_key, enabled).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not change the enabled image packs: {error}");
            })
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
        let (_, core) = self.core().ok_or(())?;
        let source = source.to_core();

        let handle = spawn_tokio!(async move { core.save_pack(&source, content).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not save an image pack: {error}");
            })
    }

    /// Delete the image pack at the given source.
    ///
    /// A state event cannot be removed, so a deleted pack is one with no
    /// images, which is also what a redacted pack looks like.
    pub(crate) async fn delete_pack(&self, source: &ImagePackSource) -> Result<(), ()> {
        let (_, core) = self.core().ok_or(())?;
        let source = source.to_core();

        let handle = spawn_tokio!(async move { core.delete_pack(&source).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not delete an image pack: {error}");
            })
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

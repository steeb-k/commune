use gtk::{glib, glib::closure_local, prelude::*, subclass::prelude::*};
use matrix_sdk::{deserialized_responses::RawSyncOrStrippedState, reqwest::StatusCode};
use ruma::{
    api::client::{
        alias::{create_alias, delete_alias},
        room,
    },
    events::{room::canonical_alias::RoomCanonicalAliasEventContent, SyncStateEvent},
    OwnedRoomAliasId,
};
use tracing::error;

use super::Room;
use crate::spawn_tokio;

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::Signal;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomAliases)]
    pub struct RoomAliases {
        /// The room these aliases belong to.
        #[property(get)]
        pub room: glib::WeakRef<Room>,
        /// The canonical alias.
        pub canonical_alias: RefCell<Option<OwnedRoomAliasId>>,
        /// The canonical alias, as a string.
        #[property(get = Self::canonical_alias_string)]
        canonical_alias_string: PhantomData<Option<String>>,
        /// The other aliases.
        pub alt_aliases: RefCell<Vec<OwnedRoomAliasId>>,
        /// The other aliases, as a `GtkStringList`.
        #[property(get)]
        pub alt_aliases_model: gtk::StringList,
        /// The alias, as a string.
        ///
        /// If the canonical alias is not set, it can be an alt alias.
        #[property(get = Self::alias_string)]
        alias_string: PhantomData<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomAliases {
        const NAME: &'static str = "RoomAliases";
        type Type = super::RoomAliases;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomAliases {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("changed").build()]);
            SIGNALS.as_ref()
        }
    }

    impl RoomAliases {
        /// Set the canonical alias.
        pub(super) fn set_canonical_alias(&self, canonical_alias: Option<OwnedRoomAliasId>) {
            if *self.canonical_alias.borrow() == canonical_alias {
                return;
            }

            self.canonical_alias.replace(canonical_alias);

            let obj = self.obj();
            obj.notify_canonical_alias_string();
            obj.notify_alias_string();
        }

        /// The canonical alias, as a string.
        fn canonical_alias_string(&self) -> Option<String> {
            self.canonical_alias
                .borrow()
                .as_ref()
                .map(ToString::to_string)
        }

        /// Set the alt aliases.
        pub(super) fn set_alt_aliases(&self, alt_aliases: Vec<OwnedRoomAliasId>) {
            let (pos, removed) = {
                let old_aliases = &*self.alt_aliases.borrow();
                let mut pos = None;

                // Check if aliases were changed in the current list.
                for (i, old_alias) in old_aliases.iter().enumerate() {
                    if !alt_aliases.get(i).is_some_and(|alias| alias == old_alias) {
                        pos = Some(i);
                        break;
                    }
                }

                // Check if aliases were added.
                let old_len = old_aliases.len();
                if pos.is_none() {
                    let new_len = alt_aliases.len();

                    if old_len < new_len {
                        pos = Some(old_len);
                    }
                }

                let Some(pos) = pos else {
                    return;
                };

                let removed = old_len.saturating_sub(pos);

                (pos, removed)
            };

            let additions = alt_aliases.get(pos..).unwrap_or_default().to_owned();
            let additions_str = additions
                .iter()
                .map(|alias| alias.as_str())
                .collect::<Vec<_>>();

            let Ok(pos) = u32::try_from(pos) else {
                return;
            };
            let Ok(removed) = u32::try_from(removed) else {
                return;
            };

            self.alt_aliases.replace(alt_aliases);
            self.alt_aliases_model.splice(pos, removed, &additions_str);

            self.obj().notify_alias_string();
        }

        /// The alias, as a string.
        fn alias_string(&self) -> Option<String> {
            self.canonical_alias_string()
                .or_else(|| self.alt_aliases_model.string(0).map(Into::into))
        }
    }
}

glib::wrapper! {
    /// Aliases of a room.
    pub struct RoomAliases(ObjectSubclass<imp::RoomAliases>);
}

impl RoomAliases {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Initialize these aliases with the given room.
    pub fn init(&self, room: &Room) {
        let imp = self.imp();
        self.imp().room.set(Some(room));

        let matrix_room = room.matrix_room();
        imp.set_canonical_alias(matrix_room.canonical_alias());
        imp.set_alt_aliases(matrix_room.alt_aliases());

        let obj_weak = glib::SendWeakRef::from(self.downgrade());
        matrix_room.add_event_handler(move |_: SyncStateEvent<RoomCanonicalAliasEventContent>| {
            let obj_weak = obj_weak.clone();
            async move {
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    if let Some(obj) = obj_weak.upgrade() {
                        let Some(room) = obj.room() else {
                            return;
                        };
                        let imp = obj.imp();

                        let matrix_room = room.matrix_room();
                        imp.set_canonical_alias(matrix_room.canonical_alias());
                        imp.set_alt_aliases(matrix_room.alt_aliases());

                        obj.emit_by_name::<()>("changed", &[]);
                    }
                });
            }
        });
    }

    /// Get the content of the canonical alias event from the store.
    async fn canonical_alias_event_content(
        &self,
    ) -> Result<Option<RoomCanonicalAliasEventContent>, ()> {
        let Some(room) = self.room() else {
            return Err(());
        };

        let matrix_room = room.matrix_room().clone();
        let handle = spawn_tokio!(async move {
            matrix_room
                .get_state_event_static::<RoomCanonicalAliasEventContent>()
                .await
        });

        let raw_event = match handle.await.unwrap() {
            Ok(Some(RawSyncOrStrippedState::Sync(raw_event))) => raw_event,
            // We shouldn't need to load this is an invited room.
            Ok(_) => return Ok(None),
            Err(error) => {
                error!("Could not get canonical alias event: {error}");
                return Err(());
            }
        };

        match raw_event.deserialize() {
            Ok(SyncStateEvent::Original(event)) => Ok(Some(event.content)),
            // The redacted event doesn't have a content.
            Ok(_) => Ok(None),
            Err(error) => {
                error!("Could not deserialize canonical alias event: {error}");
                Err(())
            }
        }
    }

    /// The canonical alias.
    pub fn canonical_alias(&self) -> Option<OwnedRoomAliasId> {
        self.imp().canonical_alias.borrow().clone()
    }

    /// Remove the given canonical alias.
    ///
    /// Checks that the canonical alias is the correct one before proceeding.
    pub async fn remove_canonical_alias(&self, alias: &OwnedRoomAliasId) -> Result<(), ()> {
        let mut event_content = self
            .canonical_alias_event_content()
            .await?
            .unwrap_or_default();

        // Remove the canonical alias, if it is there.
        if !event_content.alias.take().is_some_and(|a| a == *alias) {
            // Nothing to do.
            return Err(());
        }

        let Some(room) = self.room() else {
            return Err(());
        };

        let matrix_room = room.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(event_content).await });

        match handle.await.unwrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not remove canonical alias: {error}");
                Err(())
            }
        }
    }

    /// Set the given alias to be the canonical alias.
    ///
    /// Removes the given alias from the alt aliases if it is in the list.
    pub async fn set_canonical_alias(&self, alias: OwnedRoomAliasId) -> Result<(), ()> {
        let mut event_content = self
            .canonical_alias_event_content()
            .await?
            .unwrap_or_default();

        if event_content.alias.as_ref().is_some_and(|a| *a == alias) {
            // Nothing to do.
            return Err(());
        }

        let Some(room) = self.room() else {
            return Err(());
        };

        // Remove from the alt aliases, if it is there.
        let alt_alias_pos = event_content.alt_aliases.iter().position(|a| *a == alias);
        if let Some(pos) = alt_alias_pos {
            event_content.alt_aliases.remove(pos);
        }

        // Set as canonical alias.
        if let Some(old_canonical) = event_content.alias.replace(alias) {
            // Move the old canonical alias to the alt aliases, if it is not there already.
            let has_old_canonical = event_content.alt_aliases.contains(&old_canonical);

            if !has_old_canonical {
                event_content.alt_aliases.push(old_canonical);
            }
        }

        let matrix_room = room.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(event_content).await });

        match handle.await.unwrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not set canonical alias: {error}");
                Err(())
            }
        }
    }

    /// The other public aliases.
    pub fn alt_aliases(&self) -> Vec<OwnedRoomAliasId> {
        self.imp().alt_aliases.borrow().clone()
    }

    /// Remove the given alt alias.
    ///
    /// Checks that is in the list of alt aliases before proceeding.
    pub async fn remove_alt_alias(&self, alias: &OwnedRoomAliasId) -> Result<(), ()> {
        let mut event_content = self
            .canonical_alias_event_content()
            .await?
            .unwrap_or_default();

        // Remove from the alt aliases, if it is there.
        let alt_alias_pos = event_content.alt_aliases.iter().position(|a| a == alias);
        if let Some(pos) = alt_alias_pos {
            event_content.alt_aliases.remove(pos);
        } else {
            // Nothing to do.
            return Err(());
        }

        let Some(room) = self.room() else {
            return Err(());
        };

        let matrix_room = room.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(event_content).await });

        match handle.await.unwrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not remove alt alias: {error}");
                Err(())
            }
        }
    }

    /// Set the given alias to be an alt alias.
    ///
    /// Removes the given alias from the alt aliases if it is in the list.
    pub async fn add_alt_alias(&self, alias: OwnedRoomAliasId) -> Result<(), AddAltAliasError> {
        let Ok(event_content) = self.canonical_alias_event_content().await else {
            return Err(AddAltAliasError::Other);
        };

        let mut event_content = event_content.unwrap_or_default();

        // Do nothing if it is already present.
        if event_content.alias.as_ref().is_some_and(|a| *a == alias)
            || event_content.alt_aliases.contains(&alias)
        {
            error!("Cannot add alias already listed");
            return Err(AddAltAliasError::Other);
        }

        let Some(room) = self.room() else {
            return Err(AddAltAliasError::Other);
        };

        let matrix_room = room.matrix_room().clone();

        // Check that the alias exists and points to the proper room.
        let client = matrix_room.client();
        let alias_clone = alias.clone();
        let handle = spawn_tokio!(async move { client.resolve_room_alias(&alias_clone).await });

        match handle.await.unwrap() {
            Ok(response) => {
                if response.room_id != matrix_room.room_id() {
                    error!("Cannot add alias that points to other room");
                    return Err(AddAltAliasError::InvalidRoomId);
                }
            }
            Err(error) => {
                error!("Could not check room alias: {error}");
                if error
                    .as_client_api_error()
                    .is_some_and(|e| e.status_code == StatusCode::NOT_FOUND)
                {
                    return Err(AddAltAliasError::NotRegistered);
                } else {
                    return Err(AddAltAliasError::Other);
                }
            }
        }

        // Add as alt alias.
        event_content.alt_aliases.push(alias);
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(event_content).await });

        match handle.await.unwrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not add alt alias: {error}");
                Err(AddAltAliasError::Other)
            }
        }
    }

    /// The main alias.
    ///
    /// This is the canonical alias if there is one, of the first of the alt
    /// aliases.
    pub fn alias(&self) -> Option<OwnedRoomAliasId> {
        self.canonical_alias()
            .or_else(|| self.imp().alt_aliases.borrow().first().cloned())
    }

    /// Get the local aliases registered on the homeserver.
    pub async fn local_aliases(&self) -> Result<Vec<OwnedRoomAliasId>, ()> {
        let Some(room) = self.room() else {
            return Err(());
        };

        let matrix_room = room.matrix_room();
        let client = matrix_room.client();
        let room_id = matrix_room.room_id().to_owned();

        let handle = spawn_tokio!(async move {
            client
                .send(room::aliases::v3::Request::new(room_id), None)
                .await
        });

        match handle.await.unwrap() {
            Ok(response) => Ok(response.aliases),
            Err(error) => {
                error!("Could not fetch local room aliases: {error}");
                Err(())
            }
        }
    }

    /// Unregister the given local alias.
    pub async fn unregister_local_alias(&self, alias: OwnedRoomAliasId) -> Result<(), ()> {
        let Some(room) = self.room() else {
            return Err(());
        };

        // Check that the alias exists and points to the proper room.
        let matrix_room = room.matrix_room();
        let client = matrix_room.client();

        let request = delete_alias::v3::Request::new(alias);
        let handle = spawn_tokio!(async move { client.send(request, None).await });

        match handle.await.unwrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not unregister local alias: {error}");
                Err(())
            }
        }
    }

    /// Register the given local alias.
    pub async fn register_local_alias(
        &self,
        alias: OwnedRoomAliasId,
    ) -> Result<(), RegisterLocalAliasError> {
        let Some(room) = self.room() else {
            return Err(RegisterLocalAliasError::Other);
        };

        // Check that the alias exists and points to the proper room.
        let matrix_room = room.matrix_room();
        let client = matrix_room.client();
        let room_id = matrix_room.room_id().to_owned();

        let request = create_alias::v3::Request::new(alias, room_id);
        let handle = spawn_tokio!(async move { client.send(request, None).await });

        match handle.await.unwrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not register local alias: {error}");

                if error
                    .as_client_api_error()
                    .is_some_and(|e| e.status_code == StatusCode::CONFLICT)
                {
                    Err(RegisterLocalAliasError::AlreadyInUse)
                } else {
                    Err(RegisterLocalAliasError::Other)
                }
            }
        }
    }

    /// Connect to the signal emitted when the aliases changed.
    pub fn connect_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

impl Default for RoomAliases {
    fn default() -> Self {
        Self::new()
    }
}

/// All high-level errors that can happen when trying to add an alt alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddAltAliasError {
    /// The alias is not registered.
    NotRegistered,
    /// The alias is not registered to this room.
    InvalidRoomId,
    /// An other error occurred.
    Other,
}

/// All high-level errors that can happen when trying to register a local alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterLocalAliasError {
    /// The alias is already registered.
    AlreadyInUse,
    /// An other error occurred.
    Other,
}

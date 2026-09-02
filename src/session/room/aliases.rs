use commune_core::session::{AliasError, AliasesState};
use gtk::{glib, glib::closure_local, prelude::*, subclass::prelude::*};
use ruma::OwnedRoomAliasId;
use tokio::task::AbortHandle;
use tracing::error;

use super::Room;
use crate::{core_bridge::ObjectWatcher, spawn_tokio};

mod imp {
    use std::{cell::RefCell, marker::PhantomData, sync::LazyLock};

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomAliases)]
    pub struct RoomAliases {
        /// The room these aliases belong to.
        #[property(get)]
        room: glib::WeakRef<Room>,
        /// The canonical alias.
        pub(super) canonical_alias: RefCell<Option<OwnedRoomAliasId>>,
        /// The canonical alias, as a string.
        #[property(get = Self::canonical_alias_string)]
        canonical_alias_string: PhantomData<Option<String>>,
        /// The other aliases.
        pub(super) alt_aliases: RefCell<Vec<OwnedRoomAliasId>>,
        /// The other aliases, as a `GtkStringList`.
        #[property(get)]
        alt_aliases_model: gtk::StringList,
        /// The alias, as a string.
        ///
        /// If the canonical alias is not set, it can be an alt alias.
        #[property(get = Self::alias_string)]
        alias_string: PhantomData<Option<String>>,
        /// The task following the core's aliases.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomAliases {
        const NAME: &'static str = "RoomAliases";
        type Type = super::RoomAliases;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomAliases {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("changed").build()]);
            SIGNALS.as_ref()
        }

        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl RoomAliases {
        /// Set the room these aliases belong to, and follow its core
        /// aliases.
        pub(super) fn set_room(&self, room: &Room) {
            self.room.set(Some(room));

            let core = room.core().aliases();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe(), |obj: &super::RoomAliases, state| {
                    obj.imp().set_state(&state);
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.set_state(&core.state());
        }

        /// Mirror the core's aliases.
        fn set_state(&self, state: &AliasesState) {
            let obj = self.obj();
            let _guard = obj.freeze_notify();

            let mut changed = self.set_canonical_alias(state.canonical_alias.clone());
            changed |= self.set_alt_aliases(state.alt_aliases.clone());

            if changed {
                obj.emit_by_name::<()>("changed", &[]);
            }
        }

        /// Set the canonical alias.
        ///
        /// Returns `true` if the alias changed.
        fn set_canonical_alias(&self, canonical_alias: Option<OwnedRoomAliasId>) -> bool {
            if *self.canonical_alias.borrow() == canonical_alias {
                return false;
            }

            self.canonical_alias.replace(canonical_alias);

            let obj = self.obj();
            obj.notify_canonical_alias_string();
            obj.notify_alias_string();
            true
        }

        /// The canonical alias, as a string.
        fn canonical_alias_string(&self) -> Option<String> {
            self.canonical_alias
                .borrow()
                .as_ref()
                .map(ToString::to_string)
        }

        /// Set the alt aliases.
        ///
        /// Returns `true` if the aliases changed.
        fn set_alt_aliases(&self, alt_aliases: Vec<OwnedRoomAliasId>) -> bool {
            // Check quickly if there are any changes first.
            if *self.alt_aliases.borrow() == alt_aliases {
                return false;
            }

            let (pos, removed) = {
                let old_aliases = &*self.alt_aliases.borrow();
                let mut pos = None;

                // Check if aliases were changed in the current list.
                for (i, old_alias) in old_aliases.iter().enumerate() {
                    if alt_aliases.get(i).is_none_or(|alias| alias != old_alias) {
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
                    return false;
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
                return false;
            };
            let Ok(removed) = u32::try_from(removed) else {
                return false;
            };

            self.alt_aliases.replace(alt_aliases);
            self.alt_aliases_model.splice(pos, removed, &additions_str);

            self.obj().notify_alias_string();
            true
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
    ///
    /// The aliases themselves are the core's; this presents them, and hands
    /// the addresses subpage's edits to it.
    pub struct RoomAliases(ObjectSubclass<imp::RoomAliases>);
}

impl RoomAliases {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Initialize these aliases with the given room.
    pub(crate) fn init(&self, room: &Room) {
        self.imp().set_room(room);
    }

    /// The core's room, if it is still around.
    fn core_room(&self) -> Option<commune_core::session::Room> {
        self.room().map(|room| room.core().clone())
    }

    /// The canonical alias.
    pub(crate) fn canonical_alias(&self) -> Option<OwnedRoomAliasId> {
        self.imp().canonical_alias.borrow().clone()
    }

    /// Remove the given canonical alias.
    ///
    /// Checks that the canonical alias is the correct one before proceeding.
    pub(crate) async fn remove_canonical_alias(&self, alias: &OwnedRoomAliasId) -> Result<(), ()> {
        let Some(core) = self.core_room() else {
            return Err(());
        };
        let alias = alias.clone();

        let handle =
            spawn_tokio!(async move { core.aliases().remove_canonical_alias(&alias).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| log_alias_error("remove canonical alias", &error))
    }

    /// Set the given alias to be the canonical alias.
    ///
    /// Removes the given alias from the alt aliases if it is in the list.
    pub(crate) async fn set_canonical_alias(&self, alias: OwnedRoomAliasId) -> Result<(), ()> {
        let Some(core) = self.core_room() else {
            return Err(());
        };

        let handle = spawn_tokio!(async move { core.aliases().set_canonical_alias(alias).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| log_alias_error("set canonical alias", &error))
    }

    /// The other public aliases.
    pub(crate) fn alt_aliases(&self) -> Vec<OwnedRoomAliasId> {
        self.imp().alt_aliases.borrow().clone()
    }

    /// Remove the given alt alias.
    ///
    /// Checks that is in the list of alt aliases before proceeding.
    pub(crate) async fn remove_alt_alias(&self, alias: &OwnedRoomAliasId) -> Result<(), ()> {
        let Some(core) = self.core_room() else {
            return Err(());
        };
        let alias = alias.clone();

        let handle = spawn_tokio!(async move { core.aliases().remove_alt_alias(&alias).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| log_alias_error("remove alt alias", &error))
    }

    /// Set the given alias to be an alt alias.
    ///
    /// The alias must be registered, and to this room.
    pub(crate) async fn add_alt_alias(
        &self,
        alias: OwnedRoomAliasId,
    ) -> Result<(), AddAltAliasError> {
        let Some(core) = self.core_room() else {
            return Err(AddAltAliasError::Other);
        };

        let handle = spawn_tokio!(async move { core.aliases().add_alt_alias(alias).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| match error {
                AliasError::NotRegistered => AddAltAliasError::NotRegistered,
                AliasError::OtherRoom => AddAltAliasError::InvalidRoomId,
                error => {
                    log_alias_error("add alt alias", &error);
                    AddAltAliasError::Other
                }
            })
    }

    /// The main alias.
    ///
    /// This is the canonical alias if there is one, of the first of the alt
    /// aliases.
    pub(crate) fn alias(&self) -> Option<OwnedRoomAliasId> {
        self.canonical_alias()
            .or_else(|| self.imp().alt_aliases.borrow().first().cloned())
    }

    /// Get the local aliases registered on the homeserver.
    pub(crate) async fn local_aliases(&self) -> Result<Vec<OwnedRoomAliasId>, ()> {
        let Some(core) = self.core_room() else {
            return Err(());
        };

        let handle = spawn_tokio!(async move { core.aliases().local_aliases().await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| log_alias_error("fetch local room aliases", &error))
    }

    /// Unregister the given local alias.
    pub(crate) async fn unregister_local_alias(&self, alias: OwnedRoomAliasId) -> Result<(), ()> {
        let Some(core) = self.core_room() else {
            return Err(());
        };

        let handle =
            spawn_tokio!(async move { core.aliases().unregister_local_alias(alias).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| log_alias_error("unregister local alias", &error))
    }

    /// Register the given local alias.
    pub(crate) async fn register_local_alias(
        &self,
        alias: OwnedRoomAliasId,
    ) -> Result<(), RegisterLocalAliasError> {
        let Some(core) = self.core_room() else {
            return Err(RegisterLocalAliasError::Other);
        };

        let handle = spawn_tokio!(async move { core.aliases().register_local_alias(alias).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| match error {
                AliasError::AlreadyInUse => RegisterLocalAliasError::AlreadyInUse,
                error => {
                    log_alias_error("register local alias", &error);
                    RegisterLocalAliasError::Other
                }
            })
    }

    /// Connect to the signal emitted when the aliases changed.
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

impl Default for RoomAliases {
    fn default() -> Self {
        Self::new()
    }
}

/// Log a refused or failed alias edit.
///
/// The core already says what the homeserver said; a refusal that changes
/// nothing is the caller's to explain.
fn log_alias_error(action: &str, error: &AliasError) {
    if !matches!(error, AliasError::NothingToDo) {
        error!("Could not {action}: {error}");
    }
}

/// All high-level errors that can happen when trying to add an alt alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddAltAliasError {
    /// The alias is not registered.
    NotRegistered,
    /// The alias is not registered to this room.
    InvalidRoomId,
    /// An other error occurred.
    Other,
}

/// All high-level errors that can happen when trying to register a local alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterLocalAliasError {
    /// The alias is already registered.
    AlreadyInUse,
    /// An other error occurred.
    Other,
}

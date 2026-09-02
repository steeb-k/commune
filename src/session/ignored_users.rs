use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use indexmap::IndexSet;
use ruma::OwnedUserId;
use tokio::task::AbortHandle;
use tracing::error;

use super::Session;
use crate::{core_bridge::ObjectWatcher, spawn_tokio};

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::IgnoredUsers)]
    pub struct IgnoredUsers {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        pub session: glib::WeakRef<Session>,
        /// The ignored users, mirrored from the core.
        pub list: RefCell<IndexSet<OwnedUserId>>,
        /// The task following the core's list.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IgnoredUsers {
        const NAME: &'static str = "IgnoredUsers";
        type Type = super::IgnoredUsers;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for IgnoredUsers {
        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl ListModelImpl for IgnoredUsers {
        fn item_type(&self) -> glib::Type {
            gtk::StringObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get_index(position as usize)
                .map(|user_id| gtk::StringObject::new(user_id.as_str()).upcast())
        }
    }

    impl IgnoredUsers {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            self.session.set(session);

            self.watch_core();
            self.obj().notify_session();
        }

        /// Follow the core's list.
        fn watch_core(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }

            let Some(session) = self.session.upgrade() else {
                return;
            };
            let core = session.core().ignored_users();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe(), |obj: &super::IgnoredUsers, list| {
                    obj.imp().update_list(list.into_iter().collect());
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.update_list(core.snapshot().into_iter().collect());
        }

        /// Update the list with the given new list.
        fn update_list(&self, new_list: IndexSet<OwnedUserId>) {
            if *self.list.borrow() == new_list {
                return;
            }

            let old_len = self.n_items();
            let new_len = new_list.len() as u32;

            let mut pos = 0;
            {
                let old_list = self.list.borrow();

                for old_item in old_list.iter() {
                    let Some(new_item) = new_list.get_index(pos as usize) else {
                        break;
                    };

                    if old_item != new_item {
                        break;
                    }

                    pos += 1;
                }
            }

            if old_len == new_len && pos == new_len {
                // Nothing changed.
                return;
            }

            self.list.replace(new_list);

            self.obj().items_changed(
                pos,
                old_len.saturating_sub(pos),
                new_len.saturating_sub(pos),
            );
        }
    }
}

glib::wrapper! {
    /// The list of ignored users of a `Session`.
    ///
    /// The list itself is the core's; this presents it.
    pub struct IgnoredUsers(ObjectSubclass<imp::IgnoredUsers>)
        @implements gio::ListModel;
}

impl IgnoredUsers {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Whether this list contains the given user ID.
    pub fn contains(&self, user_id: &OwnedUserId) -> bool {
        self.imp().list.borrow().contains(user_id)
    }

    /// Add the user with the given ID to the list.
    pub async fn add(&self, user_id: &OwnedUserId) -> Result<(), ()> {
        let Some(session) = self.session() else {
            return Err(());
        };

        let core = session.core().ignored_users().clone();
        let user_id = user_id.clone();
        let handle = spawn_tokio!(async move { core.add(&user_id).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not add to the ignored users: {error}");
            })
    }

    /// Remove the user with the given ID from the list.
    pub async fn remove(&self, user_id: &OwnedUserId) -> Result<(), ()> {
        let Some(session) = self.session() else {
            return Err(());
        };

        let core = session.core().ignored_users().clone();
        let user_id = user_id.clone();
        let handle = spawn_tokio!(async move { core.remove(&user_id).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not remove from the ignored users: {error}");
            })
    }
}

impl Default for IgnoredUsers {
    fn default() -> Self {
        Self::new()
    }
}

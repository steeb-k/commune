use commune_core::session::{RemoteUserEntry, RemoteUserState};
use gtk::{glib, prelude::*, subclass::prelude::*};
use tokio::task::AbortHandle;

use crate::{
    components::PillSource,
    core_bridge::ObjectWatcher,
    prelude::*,
    session::{Session, User},
    utils::LoadingState,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RemoteUser)]
    pub struct RemoteUser {
        // The loading state of the profile.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// The task following the core's entry for this user.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RemoteUser {
        const NAME: &'static str = "RemoteUser";
        type Type = super::RemoteUser;
        type ParentType = User;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RemoteUser {
        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl PillSourceImpl for RemoteUser {
        fn identifier(&self) -> String {
            self.obj().upcast_ref::<User>().user_id_string()
        }
    }

    impl RemoteUser {
        /// Follow the core's entry for this user.
        pub(super) fn watch(&self, entry: &RemoteUserEntry) {
            let handle = ObjectWatcher::new(&*self.obj())
                .follow(entry.subscribe(), |obj: &super::RemoteUser, state| {
                    obj.imp().update_state(&state);
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.update_state(&entry.state());
        }

        /// Mirror what the core knows about this user.
        fn update_state(&self, state: &RemoteUserState) {
            if let Some(profile) = &state.profile {
                let obj = self.obj();
                let user = obj.upcast_ref::<User>();
                user.set_name(profile.display_name.clone());
                user.set_avatar_url(profile.avatar_url.clone());
            }

            self.set_loading_state(state.loading_state.into());
        }

        /// Set the loading state.
        fn set_loading_state(&self, loading_state: LoadingState) {
            if self.loading_state.get() == loading_state {
                return;
            }

            self.loading_state.set(loading_state);
            self.obj().notify_loading_state();
        }
    }
}

glib::wrapper! {
    /// A User that can only be updated by making remote calls, i.e. it won't be updated via sync.
    ///
    /// The profile, and the asking for it, are the core's; this presents them.
    pub struct RemoteUser(ObjectSubclass<imp::RemoteUser>) @extends PillSource, User;
}

impl RemoteUser {
    /// Construct a new `RemoteUser` presenting the given entry of the core's
    /// cache, which asks for the profile and asks again when it is stale.
    pub(super) fn new(session: &Session, entry: &RemoteUserEntry) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        obj.upcast_ref::<User>()
            .imp()
            .set_user_id(entry.user_id().to_owned());
        obj.imp().watch(entry);

        obj
    }
}

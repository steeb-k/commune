use std::time::{Duration, Instant};

use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::OwnedUserId;
use tracing::error;

use crate::{
    components::PillSource,
    prelude::*,
    session::model::{Session, User},
    spawn, spawn_tokio,
    utils::{AbortableHandle, LoadingState},
};

/// The time after which the profile of a user is assumed to be stale.
///
/// This matches 1 hour.
const PROFILE_VALIDITY_DURATION: Duration = Duration::from_secs(60 * 60);

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RemoteUser)]
    pub struct RemoteUser {
        // The loading state of the profile.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        // The time of the last request.
        last_request_time: Cell<Option<Instant>>,
        request_abort_handle: AbortableHandle,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RemoteUser {
        const NAME: &'static str = "RemoteUser";
        type Type = super::RemoteUser;
        type ParentType = User;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RemoteUser {}

    impl PillSourceImpl for RemoteUser {
        fn identifier(&self) -> String {
            self.obj().upcast_ref::<User>().user_id_string()
        }
    }

    impl RemoteUser {
        /// Set the loading state.
        pub(super) fn set_loading_state(&self, loading_state: LoadingState) {
            if self.loading_state.get() == loading_state {
                return;
            }

            self.loading_state.set(loading_state);

            if loading_state == LoadingState::Error {
                // Reset the request time so we try it again the next time.
                self.last_request_time.take();
            }

            self.obj().notify_loading_state();
        }

        /// Whether the profile of the user is considered to be stale.
        pub(super) fn is_profile_stale(&self) -> bool {
            self.last_request_time
                .get()
                .is_none_or(|last_time| last_time.elapsed() > PROFILE_VALIDITY_DURATION)
        }

        /// Update the last request time to now.
        pub(super) fn update_last_request_time(&self) {
            self.last_request_time.set(Some(Instant::now()));
        }

        /// Request the profile of this user.
        pub(super) async fn load_profile(&self) {
            let obj = self.obj();

            self.set_loading_state(LoadingState::Loading);

            let user_id = obj.user_id();

            let client = obj.session().client();
            let user_id_clone = user_id.clone();
            let handle = spawn_tokio!(async move {
                client.account().fetch_user_profile_of(&user_id_clone).await
            });

            let Some(result) = self.request_abort_handle.await_task(handle).await else {
                // The task was aborted, which means that the object was dropped.
                return;
            };

            let profile = match result {
                Ok(profile) => profile,
                Err(error) => {
                    error!("Could not load profile for user `{user_id}`: {error}");
                    self.set_loading_state(LoadingState::Error);
                    return;
                }
            };

            obj.set_name(profile.displayname);
            obj.set_avatar_url(profile.avatar_url);
            self.set_loading_state(LoadingState::Ready);
        }
    }
}

glib::wrapper! {
    /// A User that can only be updated by making remote calls, i.e. it won't be updated via sync.
    pub struct RemoteUser(ObjectSubclass<imp::RemoteUser>) @extends PillSource, User;
}

impl RemoteUser {
    pub(super) fn new(session: &Session, user_id: OwnedUserId) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        obj.upcast_ref::<User>().imp().set_user_id(user_id);
        obj.load_profile_if_stale();

        obj
    }

    /// Request this user's profile from the homeserver if it is considered to
    /// be stale.
    pub(super) fn load_profile_if_stale(&self) {
        let imp = self.imp();

        if !imp.is_profile_stale() {
            // The data is still valid, nothing to do.
            return;
        }

        // Set the request time right away, to prevent several requests at the same
        // time.
        imp.update_last_request_time();

        spawn!(clone!(
            #[weak]
            imp,
            async move {
                imp.load_profile().await;
            }
        ));
    }
}

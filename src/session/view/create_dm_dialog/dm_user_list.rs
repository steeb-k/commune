use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::{api::client::user_directory::search_users, UserId};
use tracing::error;

use super::DmUser;
use crate::{prelude::*, session::model::Session, spawn, spawn_tokio};

#[derive(Debug, Default, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "ContentDmUserListState")]
pub enum DmUserListState {
    #[default]
    Initial = 0,
    Loading = 1,
    NoMatching = 2,
    Matching = 3,
    Error = 4,
}

mod imp {
    use std::cell::{Cell, RefCell};

    use futures_util::future::AbortHandle;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::DmUserList)]
    pub struct DmUserList {
        pub list: RefCell<Vec<DmUser>>,
        /// The current session.
        #[property(get, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// The state of the list.
        #[property(get, builder(DmUserListState::default()))]
        pub state: Cell<DmUserListState>,
        /// The search term.
        #[property(get, set = Self::set_search_term, explicit_notify, nullable)]
        pub search_term: RefCell<Option<String>>,
        pub abort_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DmUserList {
        const NAME: &'static str = "DmUserList";
        type Type = super::DmUserList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for DmUserList {}

    impl ListModelImpl for DmUserList {
        fn item_type(&self) -> glib::Type {
            DmUser::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get(position as usize)
                .cloned()
                .and_upcast()
        }
    }

    impl DmUserList {
        /// Set the search term.
        fn set_search_term(&self, search_term: Option<String>) {
            let search_term = search_term.filter(|s| !s.is_empty());

            if search_term == *self.search_term.borrow() {
                return;
            }
            let obj = self.obj();

            self.search_term.replace(search_term);

            spawn!(clone!(
                #[weak]
                obj,
                async move {
                    obj.search_users().await;
                }
            ));

            obj.notify_search_term();
        }
    }
}

glib::wrapper! {
    /// List of users matching the `search term`.
    pub struct DmUserList(ObjectSubclass<imp::DmUserList>)
        @implements gio::ListModel;
}

impl DmUserList {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Set the state of the list.
    fn set_state(&self, state: DmUserListState) {
        let imp = self.imp();

        if state == self.state() {
            return;
        }

        imp.state.set(state);
        self.notify_state();
    }

    fn set_list(&self, users: Vec<DmUser>) {
        let added = users.len();

        let prev_users = self.imp().list.replace(users);

        self.items_changed(0, prev_users.len() as u32, added as u32);
    }

    fn clear_list(&self) {
        self.set_list(Vec::new());
    }

    async fn search_users(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let client = session.client();
        let Some(search_term) = self.search_term() else {
            self.set_state(DmUserListState::Initial);
            return;
        };

        self.set_state(DmUserListState::Loading);
        self.clear_list();

        let search_term_clone = search_term.clone();
        let handle = spawn_tokio!(async move { client.search_users(&search_term_clone, 20).await });

        let (future, handle) = futures_util::future::abortable(handle);

        if let Some(abort_handle) = self.imp().abort_handle.replace(Some(handle)) {
            abort_handle.abort();
        }

        let response = if let Ok(result) = future.await {
            result.unwrap()
        } else {
            return;
        };

        if Some(&search_term) != self.search_term().as_ref() {
            return;
        }

        match response {
            Ok(mut response) => {
                let mut add_custom = false;
                // If the search term looks like a UserId and is not already in the response,
                // insert it.
                if let Ok(user_id) = UserId::parse(&search_term) {
                    if !response.results.iter().any(|item| item.user_id == user_id) {
                        let user = search_users::v3::User::new(user_id);
                        response.results.insert(0, user);
                        add_custom = true;
                    }
                }

                let mut users: Vec<DmUser> = vec![];
                for item in response.results {
                    let user = DmUser::new(
                        &session,
                        item.user_id,
                        item.display_name.as_deref(),
                        item.avatar_url,
                    );

                    // If it is the "custom user" from the search term, fetch the avatar
                    // and display name
                    if add_custom && *user.user_id() == search_term {
                        spawn!(clone!(
                            #[weak]
                            user,
                            async move {
                                user.load_profile().await;
                            }
                        ));
                    }

                    users.push(user);
                }

                let state = if users.is_empty() {
                    DmUserListState::NoMatching
                } else {
                    DmUserListState::Matching
                };
                self.set_state(state);
                self.set_list(users);
            }
            Err(error) => {
                error!("Could not load matching users: {error}");
                self.set_state(DmUserListState::Error);
                self.clear_list();
            }
        }
    }
}

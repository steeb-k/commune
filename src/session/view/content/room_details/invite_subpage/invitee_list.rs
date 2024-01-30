use gettextrs::gettext;
use gtk::{
    gio, glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::{
    ruma::{
        api::client::{profile::get_profile, user_directory::search_users},
        OwnedUserId, UserId,
    },
    HttpError,
};
use tracing::error;

use super::Invitee;
use crate::{
    prelude::*,
    session::model::{Membership, Room},
    spawn, spawn_tokio,
};

#[derive(Debug, Default, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "ContentInviteeListState")]
pub enum InviteeListState {
    #[default]
    Initial = 0,
    Loading = 1,
    NoMatching = 2,
    Matching = 3,
    Error = 4,
}

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        collections::HashMap,
        marker::PhantomData,
    };

    use futures_util::future::AbortHandle;
    use glib::subclass::Signal;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::InviteeList)]
    pub struct InviteeList {
        pub list: RefCell<Vec<Invitee>>,
        /// The room this invitee list refers to.
        #[property(get, construct_only)]
        pub room: OnceCell<Room>,
        /// The state of the list.
        #[property(get, builder(InviteeListState::default()))]
        pub state: Cell<InviteeListState>,
        /// The search term.
        #[property(get, set = Self::set_search_term, explicit_notify)]
        pub search_term: RefCell<Option<String>>,
        pub invitee_list: RefCell<HashMap<OwnedUserId, Invitee>>,
        pub abort_handle: RefCell<Option<AbortHandle>>,
        /// Whether some users are selected.
        #[property(get = Self::has_selected)]
        pub has_selected: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InviteeList {
        const NAME: &'static str = "InviteeList";
        type Type = super::InviteeList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for InviteeList {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![
                    Signal::builder("invitee-added")
                        .param_types([Invitee::static_type()])
                        .build(),
                    Signal::builder("invitee-removed")
                        .param_types([Invitee::static_type()])
                        .build(),
                ]
            });
            SIGNALS.as_ref()
        }
    }

    impl ListModelImpl for InviteeList {
        fn item_type(&self) -> glib::Type {
            Invitee::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get(position as usize)
                .map(glib::object::Cast::upcast_ref::<glib::Object>)
                .cloned()
        }
    }

    impl InviteeList {
        /// Set the search term.
        fn set_search_term(&self, search_term: Option<String>) {
            let search_term = search_term.filter(|s| !s.is_empty());

            if search_term == *self.search_term.borrow() {
                return;
            }
            let obj = self.obj();

            self.search_term.replace(search_term);

            obj.search_users();
            obj.notify_search_term();
        }

        /// Whether some users are selected.
        fn has_selected(&self) -> bool {
            !self.invitee_list.borrow().is_empty()
        }
    }
}

glib::wrapper! {
    /// List of users matching the `search term`.
    pub struct InviteeList(ObjectSubclass<imp::InviteeList>)
        @implements gio::ListModel;
}

impl InviteeList {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// Set the state of the list.
    fn set_state(&self, state: InviteeListState) {
        let imp = self.imp();

        if state == self.state() {
            return;
        }

        imp.state.set(state);
        self.notify_state();
    }

    fn set_list(&self, users: Vec<Invitee>) {
        let added = users.len();

        let prev_users = self.imp().list.replace(users);

        self.items_changed(0, prev_users.len() as u32, added as u32);
    }

    fn clear_list(&self) {
        self.set_list(Vec::new());
    }

    fn finish_search(
        &self,
        search_term: String,
        response: Result<search_users::v3::Response, HttpError>,
    ) {
        let Some(session) = self.room().session() else {
            return;
        };
        // We should have a strong reference to the list in the main page so we can use
        // `get_or_create_members()`.
        let member_list = self.room().get_or_create_members();

        if Some(&search_term) != self.search_term().as_ref() {
            return;
        }

        match response {
            Ok(mut response) => {
                // If the search term looks like an UserId and is not already in the response,
                // insert it.
                if let Ok(user_id) = UserId::parse(&search_term) {
                    if !response.results.iter().any(|item| item.user_id == user_id) {
                        let user = search_users::v3::User::new(user_id);
                        response.results.insert(0, user);
                    }
                }

                let users: Vec<Invitee> = response
                    .results
                    .into_iter()
                    .map(|item| {
                        let user = match self.get_invitee(&item.user_id) {
                            Some(user) => {
                                // The avatar or the display name may have changed in the meantime
                                user.set_avatar_url(item.avatar_url);
                                user.set_name(item.display_name);

                                user
                            }
                            None => {
                                let user = Invitee::new(
                                    &session,
                                    item.user_id.clone(),
                                    item.display_name.as_deref(),
                                    item.avatar_url,
                                );
                                user.connect_invited_notify(
                                    clone!(@weak self as obj => move |user| {
                                        if user.invited() && user.invite_exception().is_none() {
                                            obj.add_invitee(user.clone());
                                        } else {
                                            obj.remove_invitee(user.user_id())
                                        }
                                    }),
                                );
                                // If it is the "custom user" from the search term, fetch the avatar
                                // and display name
                                let user_id = user.user_id().clone();
                                if user_id == search_term {
                                    let client = session.client();
                                    let handle = spawn_tokio!(async move {
                                        let request = get_profile::v3::Request::new(user_id);
                                        client.send(request, None).await
                                    });
                                    spawn!(clone!(@weak user => async move {
                                        let response = handle.await.unwrap();
                                        let (display_name, avatar_url) = match response {
                                            Ok(response) => {
                                                (response.displayname, response.avatar_url)
                                            },
                                            Err(_) => {
                                                return;
                                            }
                                        };
                                        // If the display name and or the avatar were returned, the Invitee gets updated.
                                        if display_name.is_some() {
                                            user.set_name(display_name);
                                        }
                                        if avatar_url.is_some() {
                                            user.set_avatar_url(avatar_url);
                                        }
                                    }));
                                }

                                user
                            }
                        };
                        // 'Disable' users that can't be invited
                        match member_list.get_membership(&item.user_id) {
                            Membership::Join => user.set_invite_exception(Some(gettext("Member"))),
                            Membership::Ban => user.set_invite_exception(Some(gettext("Banned"))),
                            Membership::Invite => {
                                user.set_invite_exception(Some(gettext("Invited")))
                            }
                            _ => {}
                        };
                        user
                    })
                    .collect();
                match users.is_empty() {
                    true => self.set_state(InviteeListState::NoMatching),
                    false => self.set_state(InviteeListState::Matching),
                }
                self.set_list(users);
            }
            Err(error) => {
                error!("Couldn’t load matching users: {error}");
                self.set_state(InviteeListState::Error);
                self.clear_list();
            }
        }
    }

    fn search_users(&self) {
        let Some(session) = self.room().session() else {
            return;
        };
        let client = session.client();
        let search_term = if let Some(search_term) = self.search_term() {
            search_term
        } else {
            // Do nothing for no search term except when currently loading
            if self.state() == InviteeListState::Loading {
                self.set_state(InviteeListState::Initial);
            }
            return;
        };

        self.set_state(InviteeListState::Loading);
        self.clear_list();

        let search_term_clone = search_term.clone();
        let handle = spawn_tokio!(async move {
            let request = search_users::v3::Request::new(search_term_clone);
            client.send(request, None).await
        });

        let (future, handle) = futures_util::future::abortable(handle);

        if let Some(abort_handle) = self.imp().abort_handle.replace(Some(handle)) {
            abort_handle.abort();
        }

        spawn!(clone!(@weak self as obj => async move {
            if let Ok(result) = future.await {
                obj.finish_search(search_term, result.unwrap());
            }
        }));
    }

    fn get_invitee(&self, user_id: &UserId) -> Option<Invitee> {
        self.imp().invitee_list.borrow().get(user_id).cloned()
    }

    pub fn add_invitee(&self, user: Invitee) {
        user.set_invited(true);
        self.imp()
            .invitee_list
            .borrow_mut()
            .insert(user.user_id().clone(), user.clone());
        self.emit_by_name::<()>("invitee-added", &[&user]);
        self.notify_has_selected();
    }

    pub fn invitees(&self) -> Vec<Invitee> {
        self.imp()
            .invitee_list
            .borrow()
            .values()
            .map(Clone::clone)
            .collect()
    }

    pub fn remove_invitee(&self, user_id: &UserId) {
        let removed = self.imp().invitee_list.borrow_mut().remove(user_id);
        if let Some(user) = removed {
            user.set_invited(false);
            self.emit_by_name::<()>("invitee-removed", &[&user]);
            self.notify_has_selected();
        }
    }

    pub fn connect_invitee_added<F: Fn(&Self, &Invitee) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "invitee-added",
            true,
            closure_local!(move |obj: Self, invitee: Invitee| {
                f(&obj, &invitee);
            }),
        )
    }

    pub fn connect_invitee_removed<F: Fn(&Self, &Invitee) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "invitee-removed",
            true,
            closure_local!(move |obj: Self, invitee: Invitee| {
                f(&obj, &invitee);
            }),
        )
    }
}

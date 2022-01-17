use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use log::error;
use matrix_sdk::ruma::{api::client::r0::user_directory::search_users, identifiers::UserId};
use matrix_sdk::HttpError;
use std::sync::Arc;

use crate::session::user::UserExt;
use crate::{session::Room, spawn, spawn_tokio};

use super::Invitee;

#[derive(Debug, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "ContentInviteeListState")]
pub enum InviteeListState {
    Initial = 0,
    Loading = 1,
    NoMatching = 2,
    Matching = 3,
    Error = 4,
}

impl Default for InviteeListState {
    fn default() -> Self {
        Self::Initial
    }
}

mod imp {
    use futures::future::AbortHandle;
    use glib::subclass::Signal;
    use once_cell::{sync::Lazy, unsync::OnceCell};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    use super::*;

    #[derive(Debug, Default)]
    pub struct InviteeList {
        pub list: RefCell<Vec<Invitee>>,
        pub room: OnceCell<Room>,
        pub state: Cell<InviteeListState>,
        pub search_term: RefCell<Option<String>>,
        pub invitee_list: RefCell<HashMap<Arc<UserId>, Invitee>>,
        pub abort_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InviteeList {
        const NAME: &'static str = "InviteeList";
        type Type = super::InviteeList;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for InviteeList {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "room",
                        "Room",
                        "The room this invitee list refers to",
                        Room::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "search-term",
                        "Search Term",
                        "The search term",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_boolean(
                        "has-selected",
                        "Has Selected",
                        "Whether the user has selected some users",
                        false,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_enum(
                        "state",
                        "InviteeListState",
                        "The state of the list",
                        InviteeListState::static_type(),
                        InviteeListState::default() as i32,
                        glib::ParamFlags::READABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![
                    Signal::builder(
                        "invitee-added",
                        &[Invitee::static_type().into()],
                        <()>::static_type().into(),
                    )
                    .build(),
                    Signal::builder(
                        "invitee-removed",
                        &[Invitee::static_type().into()],
                        <()>::static_type().into(),
                    )
                    .build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "room" => self.room.set(value.get::<Room>().unwrap()).unwrap(),
                "search-term" => obj.set_search_term(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "room" => obj.room().to_value(),
                "search-term" => obj.search_term().to_value(),
                "has-selected" => obj.has_selected().to_value(),
                "state" => obj.state().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl ListModelImpl for InviteeList {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            Invitee::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.borrow().len() as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get(position as usize)
                .map(glib::object::Cast::upcast_ref::<glib::Object>)
                .cloned()
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
        glib::Object::new(&[("room", room)]).expect("Failed to create InviteeList")
    }

    pub fn room(&self) -> &Room {
        let priv_ = imp::InviteeList::from_instance(self);
        priv_.room.get().unwrap()
    }

    pub fn set_search_term(&self, search_term: Option<String>) {
        let priv_ = imp::InviteeList::from_instance(self);

        if search_term.as_ref() == priv_.search_term.borrow().as_ref() {
            return;
        }

        if search_term.as_ref().map_or(false, |s| s.is_empty()) {
            priv_.search_term.replace(None);
        } else {
            priv_.search_term.replace(search_term);
        }

        self.search_users();
        self.notify("search_term");
    }

    fn search_term(&self) -> Option<String> {
        let priv_ = imp::InviteeList::from_instance(self);
        priv_.search_term.borrow().clone()
    }

    fn set_state(&self, state: InviteeListState) {
        let priv_ = imp::InviteeList::from_instance(self);

        if state == self.state() {
            return;
        }

        priv_.state.set(state);
        self.notify("state");
    }

    pub fn state(&self) -> InviteeListState {
        let priv_ = imp::InviteeList::from_instance(self);
        priv_.state.get()
    }

    fn set_list(&self, users: Vec<Invitee>) {
        let priv_ = imp::InviteeList::from_instance(self);
        let added = users.len();

        let prev_users = priv_.list.replace(users);

        self.items_changed(0, prev_users.len() as u32, added as u32);
    }

    fn clear_list(&self) {
        self.set_list(Vec::new());
    }

    fn finish_search(
        &self,
        search_term: String,
        response: Result<search_users::Response, HttpError>,
    ) {
        let session = self.room().session();
        let member_list = self.room().members();

        if Some(search_term) != self.search_term() {
            return;
        }

        match response {
            Ok(response) if response.results.len() == 0 => {
                self.set_state(InviteeListState::NoMatching);
                self.clear_list();
            }
            Ok(response) => {
                let users: Vec<Invitee> = response
                    .results
                    .into_iter()
                    .filter_map(|item| {
                        // Skip over users that are already in the room
                        if member_list.contains(&item.user_id) {
                            self.remove_invitee(item.user_id.into());
                            None
                        } else if let Some(user) = self.get_invitee(&item.user_id) {
                            // The avatar or the display name may have changed in the mean time
                            user.set_avatar_url(item.avatar_url);
                            user.set_display_name(item.display_name);
                            Some(user)
                        } else {
                            let user = Invitee::new(
                                &session,
                                &item.user_id,
                                item.display_name.as_deref(),
                                item.avatar_url.as_deref(),
                            );

                            user.connect_notify_local(
                                Some("invited"),
                                clone!(@weak self as obj => move |user, _| {
                                    if user.is_invited() {
                                        obj.add_invitee(user.clone());
                                    } else {
                                        obj.remove_invitee(user.user_id())
                                    }
                                }),
                            );

                            Some(user)
                        }
                    })
                    .collect();

                self.set_list(users);
                self.set_state(InviteeListState::Matching);
            }
            Err(error) => {
                error!("Couldn't load matching users: {}", error);
                self.set_state(InviteeListState::Error);
                self.clear_list();
            }
        }
    }

    fn search_users(&self) {
        let priv_ = imp::InviteeList::from_instance(self);
        let client = self.room().session().client();
        let search_term = if let Some(search_term) = self.search_term() {
            search_term
        } else {
            // Do nothing for no search term execpt when currently loading
            if self.state() == InviteeListState::Loading {
                self.set_state(InviteeListState::Initial);
            }
            return;
        };

        self.set_state(InviteeListState::Loading);
        self.clear_list();

        let search_term_clone = search_term.clone();
        let handle = spawn_tokio!(async move {
            let request = search_users::Request::new(&search_term_clone);
            client.send(request, None).await
        });

        let (future, handle) = futures::future::abortable(handle);

        if let Some(abort_handle) = priv_.abort_handle.replace(Some(handle)) {
            abort_handle.abort();
        }

        spawn!(clone!(@weak self as obj => async move {
            match future.await {
                Ok(result) => obj.finish_search(search_term, result.unwrap()),
                Err(_) => {},
            }
        }));
    }

    fn get_invitee(&self, user_id: &UserId) -> Option<Invitee> {
        let priv_ = imp::InviteeList::from_instance(self);
        priv_.invitee_list.borrow().get(user_id).cloned()
    }

    pub fn add_invitee(&self, user: Invitee) {
        let priv_ = imp::InviteeList::from_instance(self);
        user.set_invited(true);
        priv_
            .invitee_list
            .borrow_mut()
            .insert(user.user_id(), user.clone());
        self.emit_by_name("invitee-added", &[&user]).unwrap();
        self.notify("has-selected");
    }

    pub fn invitees(&self) -> Vec<Invitee> {
        let priv_ = imp::InviteeList::from_instance(self);
        priv_
            .invitee_list
            .borrow()
            .values()
            .map(Clone::clone)
            .collect()
    }

    fn remove_invitee(&self, user_id: Arc<UserId>) {
        let priv_ = imp::InviteeList::from_instance(self);
        let removed = priv_.invitee_list.borrow_mut().remove(&user_id);
        if let Some(user) = removed {
            user.set_invited(false);
            self.emit_by_name("invitee-removed", &[&user]).unwrap();
            self.notify("has-selected");
        }
    }

    pub fn has_selected(&self) -> bool {
        let priv_ = imp::InviteeList::from_instance(self);
        !priv_.invitee_list.borrow().is_empty()
    }

    pub fn connect_invitee_added<F: Fn(&Self, &Invitee) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("invitee-added", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            let invitee = values[1].get::<Invitee>().unwrap();
            f(&obj, &invitee);
            None
        })
        .unwrap()
    }

    pub fn connect_invitee_removed<F: Fn(&Self, &Invitee) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("invitee-removed", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            let invitee = values[1].get::<Invitee>().unwrap();
            f(&obj, &invitee);
            None
        })
        .unwrap()
    }
}

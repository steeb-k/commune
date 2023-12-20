use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::{
    api::client::directory::get_public_rooms_filtered::v3::{
        Request as PublicRoomsRequest, Response as PublicRoomsResponse,
    },
    assign,
    directory::{Filter, RoomNetwork},
    uint, ServerName,
};
use ruma::directory::RoomTypeFilter;
use tracing::error;

use super::{PublicRoom, Server};
use crate::{session::model::Session, spawn, spawn_tokio};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PublicRoomList)]
    pub struct PublicRoomList {
        pub list: RefCell<Vec<PublicRoom>>,
        pub search_term: RefCell<Option<String>>,
        pub network: RefCell<Option<String>>,
        pub server: RefCell<Option<String>>,
        pub next_batch: RefCell<Option<String>>,
        pub request_sent: Cell<bool>,
        pub total_room_count_estimate: Cell<Option<u64>>,
        /// The current session.
        #[property(get, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// Whether the list is loading.
        #[property(get = Self::loading)]
        pub loading: PhantomData<bool>,
        /// Whether the list is empty.
        #[property(get = Self::empty)]
        pub empty: PhantomData<bool>,
        /// Whether all results for the current search were loaded.
        #[property(get = Self::complete)]
        pub complete: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PublicRoomList {
        const NAME: &'static str = "PublicRoomList";
        type Type = super::PublicRoomList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for PublicRoomList {}

    impl ListModelImpl for PublicRoomList {
        fn item_type(&self) -> glib::Type {
            PublicRoom::static_type()
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

    impl PublicRoomList {
        /// Whether the list is loading.
        fn loading(&self) -> bool {
            self.request_sent.get() && self.list.borrow().is_empty()
        }

        /// Whether the list is empty.
        fn empty(&self) -> bool {
            !self.request_sent.get() && self.list.borrow().is_empty()
        }

        /// Whether all results for the current search were loaded.
        fn complete(&self) -> bool {
            self.next_batch.borrow().is_none()
        }
    }
}

glib::wrapper! {
    /// A list of rooms in a homeserver's public directory.
    pub struct PublicRoomList(ObjectSubclass<imp::PublicRoomList>)
        @implements gio::ListModel;
}

impl PublicRoomList {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Whether a request is in progress.
    fn request_sent(&self) -> bool {
        self.imp().request_sent.get()
    }

    /// Set whether a request is in progress.
    fn set_request_sent(&self, request_sent: bool) {
        self.imp().request_sent.set(request_sent);

        self.notify_loading();
        self.notify_empty();
        self.notify_complete();
    }

    pub fn init(&self) {
        // Initialize the list if it's not loading nor loaded.
        if !self.request_sent() && self.imp().list.borrow().is_empty() {
            self.load_public_rooms(true);
        }
    }

    /// Search the given term on the given server.
    pub fn search(&self, search_term: Option<String>, server: Server) {
        let imp = self.imp();
        let network = Some(server.network());
        let server = server.server();

        if *imp.search_term.borrow() == search_term
            && *imp.server.borrow() == server
            && *imp.network.borrow() == network
        {
            return;
        }

        imp.search_term.replace(search_term);
        imp.server.replace(server);
        imp.network.replace(network);
        self.load_public_rooms(true);
    }

    fn handle_public_rooms_response(&self, response: PublicRoomsResponse) {
        let imp = self.imp();
        let session = self.session().unwrap();
        let room_list = session.room_list();

        imp.next_batch.replace(response.next_batch.to_owned());
        imp.total_room_count_estimate
            .replace(response.total_room_count_estimate.map(Into::into));

        let (position, removed, added) = {
            let mut list = imp.list.borrow_mut();
            let position = list.len();
            let added = response.chunk.len();
            let server = imp.server.borrow().clone().unwrap_or_default();
            let mut new_rooms = response
                .chunk
                .into_iter()
                .map(|matrix_room| {
                    let room = PublicRoom::new(&room_list, &server);
                    room.set_matrix_public_room(matrix_room);
                    room
                })
                .collect();

            let empty_row = list
                .pop()
                .unwrap_or_else(|| PublicRoom::new(&room_list, &server));
            list.append(&mut new_rooms);

            if !self.complete() {
                list.push(empty_row);
                if position == 0 {
                    (position, 0, added + 1)
                } else {
                    (position - 1, 0, added)
                }
            } else if position == 0 {
                (position, 0, added)
            } else {
                (position - 1, 1, added)
            }
        };

        if added > 0 {
            self.items_changed(position as u32, removed as u32, added as u32);
        }
        self.set_request_sent(false);
    }

    /// Whether this is the response for the latest request that was sent.
    fn is_valid_response(
        &self,
        search_term: Option<String>,
        server: Option<String>,
        network: Option<String>,
    ) -> bool {
        let imp = self.imp();
        *imp.search_term.borrow() == search_term
            && *imp.server.borrow() == server
            && *imp.network.borrow() == network
    }

    pub fn load_public_rooms(&self, clear: bool) {
        let imp = self.imp();

        if self.request_sent() && !clear {
            return;
        }

        if clear {
            // Clear the previous list
            let removed = imp.list.borrow().len();
            imp.list.borrow_mut().clear();
            let _ = imp.next_batch.take();
            self.items_changed(0, removed as u32, 0);
        }

        self.set_request_sent(true);

        let next_batch = imp.next_batch.borrow().clone();

        if next_batch.is_none() && !clear {
            return;
        }

        let client = self.session().unwrap().client();
        let search_term = imp.search_term.borrow().clone();
        let server = imp.server.borrow().clone();
        let network = imp.network.borrow().clone();
        let current_search_term = search_term.clone();
        let current_server = server.clone();
        let current_network = network.clone();

        let handle = spawn_tokio!(async move {
            let room_network = match network.as_deref() {
                Some("matrix") => RoomNetwork::Matrix,
                Some("all") => RoomNetwork::All,
                Some(custom) => RoomNetwork::ThirdParty(custom.to_owned()),
                _ => RoomNetwork::default(),
            };
            let server = server.and_then(|server| ServerName::parse(server).ok());

            let request = assign!(PublicRoomsRequest::new(), {
                limit: Some(uint!(20)),
                since: next_batch,
                room_network,
                server,
                filter: assign!(
                    Filter::new(),
                    { generic_search_term: search_term, room_types: vec![RoomTypeFilter::Default] }
                ),
            });
            client.public_rooms_filtered(request).await
        });

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                // If the search term changed we ignore the response
                if obj.is_valid_response(current_search_term, current_server, current_network) {
                    match handle.await.unwrap() {
                     Ok(response) => obj.handle_public_rooms_response(response),
                     Err(error) => {
                        obj.set_request_sent(false);
                        error!("Error loading public rooms: {error}")
                     },
                    }
                }
            })
        );
    }
}

use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{
    api::client::directory::get_public_rooms_filtered,
    assign,
    directory::{Filter, RoomNetwork, RoomTypeFilter},
    OwnedServerName,
};
use tokio::task::AbortHandle;
use tracing::error;

use super::{ExploreServer, PublicRoom};
use crate::{session::model::Session, spawn, spawn_tokio, utils::LoadingState};

/// The maximum size of a batch of public rooms.
const PUBLIC_ROOMS_BATCH_SIZE: u32 = 20;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PublicRoomList)]
    pub struct PublicRoomList {
        /// The current session.
        #[property(get, construct_only)]
        session: glib::WeakRef<Session>,
        /// The list of rooms.
        list: RefCell<Vec<PublicRoom>>,
        /// The current search.
        search: RefCell<PublicRoomsSearch>,
        /// The next batch to continue the search, if any.
        next_batch: RefCell<Option<String>>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// The abort handle for the current request.
        abort_handle: RefCell<Option<AbortHandle>>,
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
                .cloned()
                .and_upcast()
        }
    }

    impl PublicRoomList {
        /// Set the current search.
        pub(super) fn set_search(&self, search: PublicRoomsSearch) {
            if *self.search.borrow() == search {
                return;
            }

            self.search.replace(search);

            // Trigger a new search.
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load(true).await;
                }
            ));
        }

        /// Set the loading state.
        fn set_loading_state(&self, state: LoadingState) {
            if self.loading_state.get() == state {
                return;
            }

            self.loading_state.set(state);
            self.obj().notify_loading_state();
        }

        /// Whether the list is empty.
        pub(super) fn is_empty(&self) -> bool {
            self.list.borrow().is_empty()
        }

        /// Whether we can load more rooms with the current search.
        pub(super) fn can_load_more(&self) -> bool {
            self.loading_state.get() != LoadingState::Loading && self.next_batch.borrow().is_some()
        }

        /// Load rooms.
        ///
        /// If `clear` is `true`, we start a new search and replace the list of
        /// rooms, otherwise we use the `next_batch` and add more rooms.
        pub(super) async fn load(&self, clear: bool) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            // Only make a request if we can load more items or we want to replace the
            // current list.
            if !clear && !self.can_load_more() {
                return;
            }

            if clear {
                // Clear the list.
                let removed = self.list.borrow().len();
                self.list.borrow_mut().clear();
                self.next_batch.take();

                // Abort any ongoing request.
                if let Some(handle) = self.abort_handle.take() {
                    handle.abort();
                }

                self.obj().items_changed(0, removed as u32, 0);
            }

            self.set_loading_state(LoadingState::Loading);

            let next_batch = self.next_batch.borrow().clone();
            let search = self.search.borrow().clone();
            let request = search.as_request(next_batch);

            let client = session.client();
            let handle = spawn_tokio!(async move { client.public_rooms_filtered(request).await });

            self.abort_handle.replace(Some(handle.abort_handle()));

            let Ok(result) = handle.await else {
                // The request was aborted.
                self.abort_handle.take();
                return;
            };

            self.abort_handle.take();

            if *self.search.borrow() != search {
                // This is not the current search anymore, ignore the response.
                return;
            }

            match result {
                Ok(response) => self.add_rooms(&search, response),
                Err(error) => {
                    self.set_loading_state(LoadingState::Error);
                    error!("Could not search public rooms: {error}");
                }
            }
        }

        /// Add the rooms from the given response to this list.
        fn add_rooms(
            &self,
            search: &PublicRoomsSearch,
            response: get_public_rooms_filtered::v3::Response,
        ) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let room_list = session.room_list();

            self.next_batch.replace(response.next_batch);

            let (position, added) = {
                let mut list = self.list.borrow_mut();
                let position = list.len();
                let added = response.chunk.len();

                list.extend(
                    response
                        .chunk
                        .into_iter()
                        .map(|data| PublicRoom::new(&room_list, search.server.clone(), data)),
                );

                (position, added)
            };

            if added > 0 {
                self.obj().items_changed(position as u32, 0, added as u32);
            }
            self.set_loading_state(LoadingState::Ready);
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

    /// Whether the list is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.imp().is_empty()
    }

    /// Search the given term on the given server.
    pub(crate) fn search(&self, search_term: Option<String>, server: &ExploreServer) {
        let search = PublicRoomsSearch {
            search_term,
            server: server.server().cloned(),
            third_party_network: server.third_party_network(),
        };
        self.imp().set_search(search);
    }

    /// Load more rooms.
    pub(crate) fn load_more(&self) {
        let imp = self.imp();

        if imp.can_load_more() {
            spawn!(clone!(
                #[weak]
                imp,
                async move { imp.load(false).await }
            ));
        }
    }
}

/// Data about a search in the public rooms directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PublicRoomsSearch {
    /// The term to search.
    search_term: Option<String>,
    /// The server to search.
    server: Option<OwnedServerName>,
    /// The network to search.
    third_party_network: Option<String>,
}

impl PublicRoomsSearch {
    /// Convert this `PublicRoomsSearch` to a request.
    fn as_request(&self, next_batch: Option<String>) -> get_public_rooms_filtered::v3::Request {
        let room_network = if let Some(third_party_network) = &self.third_party_network {
            RoomNetwork::ThirdParty(third_party_network.clone())
        } else {
            RoomNetwork::Matrix
        };

        assign!( get_public_rooms_filtered::v3::Request::new(), {
            limit: Some(PUBLIC_ROOMS_BATCH_SIZE.into()),
            since: next_batch,
            room_network,
            server: self.server.clone(),
            filter: assign!(
                Filter::new(),
                { generic_search_term: self.search_term.clone(), room_types: vec![RoomTypeFilter::Default] }
            ),
        })
    }
}

use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    room::MessagesOptions,
    ruma::{
        api::client::filter::{RoomEventFilter, UrlFilter},
        assign,
        events::MessageLikeEventType,
        uint,
    },
};
use tracing::error;

use super::HistoryViewerEvent;
use crate::{
    session::model::{Room, TimelineState},
    spawn_tokio,
};

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        sync::Arc,
    };

    use futures_util::lock::Mutex;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::HistoryViewerTimeline)]
    pub struct HistoryViewerTimeline {
        /// The room that this timeline belongs to.
        #[property(get, construct_only)]
        pub room: OnceCell<Room>,
        /// The state of this timeline.
        #[property(get, builder(TimelineState::default()))]
        pub state: Cell<TimelineState>,
        pub list: RefCell<Vec<HistoryViewerEvent>>,
        pub last_token: Arc<Mutex<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HistoryViewerTimeline {
        const NAME: &'static str = "HistoryViewerTimeline";
        type Type = super::HistoryViewerTimeline;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for HistoryViewerTimeline {}

    impl ListModelImpl for HistoryViewerTimeline {
        fn item_type(&self) -> glib::Type {
            HistoryViewerEvent::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let list = self.list.borrow();
            list.get(position as usize)
                .map(|o| o.clone().upcast::<glib::Object>())
        }
    }
}

glib::wrapper! {
    /// A room timeline for the history viewers.
    pub struct HistoryViewerTimeline(ObjectSubclass<imp::HistoryViewerTimeline>)
        @implements gio::ListModel;
}

impl HistoryViewerTimeline {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// Load more events in the timeline.
    ///
    /// Returns `true` if more events can be loaded.
    pub async fn load(&self) -> bool {
        let imp = self.imp();

        if matches!(
            self.state(),
            TimelineState::Loading | TimelineState::Complete
        ) {
            return false;
        }

        self.set_state(TimelineState::Loading);

        let room = self.room();
        let matrix_room = room.matrix_room().clone();
        let last_token = imp.last_token.clone();
        let is_encrypted = room.encrypted();
        let handle: tokio::task::JoinHandle<matrix_sdk::Result<_>> = spawn_tokio!(async move {
            let last_token = last_token.lock().await;

            // If the room is encrypted, the messages content cannot be filtered with URLs
            let filter = if is_encrypted {
                let filter_types = vec![
                    MessageLikeEventType::RoomEncrypted.to_string(),
                    MessageLikeEventType::RoomMessage.to_string(),
                ];
                assign!(RoomEventFilter::default(), {
                    types: Some(filter_types),
                })
            } else {
                let filter_types = vec![MessageLikeEventType::RoomMessage.to_string()];
                assign!(RoomEventFilter::default(), {
                    types: Some(filter_types),
                    url_filter: Some(UrlFilter::EventsWithUrl),
                })
            };
            let options = assign!(MessagesOptions::backward().from(&**last_token), {
                limit: uint!(20),
                filter,
            });

            matrix_room.messages(options).await
        });

        match handle.await.unwrap() {
            Ok(events) => match events.end {
                Some(end_token) => {
                    *imp.last_token.lock().await = end_token;

                    let events: Vec<HistoryViewerEvent> = events
                        .chunk
                        .into_iter()
                        .filter_map(|event| HistoryViewerEvent::try_new(&room, event))
                        .collect();

                    self.append(events);

                    self.set_state(TimelineState::Ready);
                    true
                }
                None => {
                    self.set_state(TimelineState::Complete);
                    false
                }
            },
            Err(error) => {
                error!("Failed to load history viewer timeline events: {error}");
                self.set_state(TimelineState::Error);
                false
            }
        }
    }

    fn append(&self, batch: Vec<HistoryViewerEvent>) {
        let imp = self.imp();

        if batch.is_empty() {
            return;
        }

        let added = batch.len();
        let index = {
            let mut list = imp.list.borrow_mut();
            let index = list.len();

            // Extend the size of the list so that rust doesn't need to reallocate memory
            // multiple times
            list.reserve(batch.len());

            for event in batch {
                list.push(event.upcast());
            }

            index
        };

        self.items_changed(index as u32, 0, added as u32);
    }

    fn set_state(&self, state: TimelineState) {
        if state == self.state() {
            return;
        }

        self.imp().state.set(state);
        self.notify_state();
    }
}

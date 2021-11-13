use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use log::error;
use matrix_sdk::{
    ruma::{
        api::client::r0::message::get_message_events::Direction,
        events::{AnySyncRoomEvent, AnySyncStateEvent},
        identifiers::EventId,
    },
    uuid::Uuid,
};

use crate::session::room::{Event, Item, ItemType, Room};
use crate::{spawn, spawn_tokio};

mod imp {
    use super::*;
    use once_cell::{sync::Lazy, unsync::OnceCell};
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, VecDeque};

    #[derive(Debug, Default)]
    pub struct Timeline {
        pub room: OnceCell<Room>,
        /// A store to keep track of related events that aren't known
        pub relates_to_events: RefCell<HashMap<EventId, Vec<EventId>>>,
        /// All events shown in the room history
        pub list: RefCell<VecDeque<Item>>,
        /// A Hashmap linking `EventId` to corresponding `Event`
        pub event_map: RefCell<HashMap<EventId, Event>>,
        /// Maps the temporary `EventId` of the pending Event to the real `EventId`
        pub pending_events: RefCell<HashMap<String, EventId>>,
        pub loading: Cell<bool>,
        pub complete: Cell<bool>,
        pub oldest_event: RefCell<Option<EventId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Timeline {
        const NAME: &'static str = "Timeline";
        type Type = super::Timeline;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for Timeline {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "room",
                        "Room",
                        "The Room containing this timeline",
                        Room::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_boolean(
                        "loading",
                        "Loading",
                        "Whether a response is loaded or not",
                        false,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_boolean(
                        "empty",
                        "Empty",
                        "Whether the timeline is empty",
                        false,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_boolean(
                        "complete",
                        "Complete",
                        "Whether the full timeline is loaded",
                        false,
                        glib::ParamFlags::READABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "room" => {
                    let room = value.get::<Room>().unwrap();
                    obj.set_room(room);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "room" => self.room.get().unwrap().to_value(),
                "loading" => obj.loading().to_value(),
                "empty" => obj.is_empty().to_value(),
                "complete" => obj.is_complete().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl ListModelImpl for Timeline {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            Item::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.borrow().len() as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            let list = self.list.borrow();

            list.get(position as usize)
                .map(|o| o.clone().upcast::<glib::Object>())
        }
    }
}

glib::wrapper! {
    /// List of all loaded Events in a room. Implements ListModel.
    ///
    /// There is no strict message ordering enforced by the Timeline; events
    /// will be appended/prepended to existing events in the order they are
    /// received by the server.
    ///
    /// This struct additionally keeps track of pending events that have yet to
    /// get an event ID assigned from the server.
    pub struct Timeline(ObjectSubclass<imp::Timeline>)
        @implements gio::ListModel;
}

// TODO:
// - [ ] Add and handle AnyEphemeralRoomEvent this includes read recipes
// - [ ] Add new message divider
impl Timeline {
    pub fn new(room: &Room) -> Self {
        glib::Object::new(&[("room", &room)]).expect("Failed to create Timeline")
    }

    fn items_changed(&self, position: u32, removed: u32, added: u32) {
        let priv_ = imp::Timeline::from_instance(self);

        let last_new_message_date;

        // Insert date divider, this needs to happen before updating the position and headers
        let added = {
            let position = position as usize;
            let added = added as usize;
            let mut list = priv_.list.borrow_mut();

            let mut previous_timestamp = if position > 0 {
                list.get(position - 1)
                    .and_then(|item| item.event_timestamp())
            } else {
                None
            };
            let mut divider: Vec<(usize, Item)> = vec![];
            let mut index = position;
            for current in list.range(position..position + added) {
                if let Some(current_timestamp) = current.event_timestamp() {
                    if Some(current_timestamp.ymd()) != previous_timestamp.as_ref().map(|t| t.ymd())
                    {
                        divider.push((index, Item::for_day_divider(current_timestamp.clone())));
                        previous_timestamp = Some(current_timestamp);
                    }
                }
                index += 1;
            }

            let divider_len = divider.len();
            last_new_message_date = divider.last().and_then(|item| match item.1.type_() {
                ItemType::DayDivider(date) => Some(date.clone()),
                _ => None,
            });
            for (added, (position, date)) in divider.into_iter().enumerate() {
                list.insert(position + added, date);
            }

            (added + divider_len) as u32
        };

        // Remove first day divider if a new one is added earlier with the same day
        let removed = {
            let mut list = priv_.list.borrow_mut();
            if let Some(ItemType::DayDivider(date)) = list
                .get(position as usize + added as usize)
                .map(|item| item.type_())
            {
                if Some(date.ymd()) == last_new_message_date.as_ref().map(|date| date.ymd()) {
                    list.remove(position as usize + added as usize);
                    removed + 1
                } else {
                    removed
                }
            } else {
                removed
            }
        };

        // Update the header for events that are allowed to hide the header
        {
            let position = position as usize;
            let added = added as usize;
            let list = priv_.list.borrow();

            let mut previous_sender = if position > 0 {
                list.get(position - 1)
                    .filter(|event| event.can_hide_header())
                    .and_then(|event| event.matrix_sender())
            } else {
                None
            };

            for current in list.range(position..position + added) {
                let current_sender = current.matrix_sender();

                if !current.can_hide_header() {
                    current.set_show_header(false);
                    previous_sender = None;
                } else if current_sender != previous_sender {
                    current.set_show_header(true);
                    previous_sender = current_sender;
                } else {
                    current.set_show_header(false);
                }
            }

            // Update the events after the new events
            for next in list.range((position + added)..) {
                // After an event with non hiddable header the visibility for headers will be correct
                if !next.can_hide_header() {
                    break;
                }

                // Once the sender changes we can be sure that the visibility for headers will be correct
                if next.matrix_sender() != previous_sender {
                    next.set_show_header(true);
                    break;
                }

                // The `next` has the same sender as the `current`, therefore we don't show the
                // header and we need to check the event after `next`
                next.set_show_header(false);
            }
        }

        // Update relates_to
        {
            let list = priv_.list.borrow();
            let mut relates_to_events = priv_.relates_to_events.borrow_mut();

            for event in list
                .range(position as usize..(position + added) as usize)
                .filter_map(|item| item.event())
            {
                if let Some(relates_to_event_id) = event.related_matrix_event() {
                    if let Some(relates_to_event) = self.event_by_id(&relates_to_event_id) {
                        // FIXME: group events and set them all at once, to reduce the emission of notify
                        relates_to_event.add_relates_to(vec![event.to_owned()]);
                    } else {
                        // Store the new event if the `related_to` event isn't known, we will update the `relates_to` once
                        // the `related_to` event is is added to the list
                        let relates_to_event =
                            relates_to_events.entry(relates_to_event_id).or_default();
                        relates_to_event.push(event.matrix_event_id().to_owned());
                    }
                }

                if let Some(relates_to) = relates_to_events.remove(&event.matrix_event_id()) {
                    event.add_relates_to(
                        relates_to
                            .into_iter()
                            .map(|event_id| {
                                self.event_by_id(&event_id)
                                    .expect("Previously known event has disappeared")
                            })
                            .collect(),
                    );
                }
            }
        }

        self.notify("empty");

        self.upcast_ref::<gio::ListModel>()
            .items_changed(position, removed, added);
    }

    fn add_hidden_event(&self, event: Event) {
        let priv_ = imp::Timeline::from_instance(self);

        let mut relates_to_events = priv_.relates_to_events.borrow_mut();

        if let Some(relates_to_event_id) = event.related_matrix_event() {
            if let Some(relates_to_event) = self.event_by_id(&relates_to_event_id) {
                // FIXME: group events and set them all at once, to reduce the emission of notify
                relates_to_event.add_relates_to(vec![event.to_owned()]);
            } else {
                // Store the new event if the `related_to` event isn't known, we will update the `relates_to` once
                // the `related_to` event is is added to the list
                let relates_to_event = relates_to_events.entry(relates_to_event_id).or_default();
                relates_to_event.push(event.matrix_event_id());
            }
        }

        if let Some(relates_to) = relates_to_events.remove(&event.matrix_event_id()) {
            event.add_relates_to(
                relates_to
                    .into_iter()
                    .map(|event_id| {
                        self.event_by_id(&event_id)
                            .expect("Previously known event has disappeared")
                    })
                    .collect(),
            );
        }
    }

    /// Append the new events
    // TODO: This should be lazy, for inspiration see: https://blogs.gnome.org/ebassi/documentation/lazy-loading/
    pub fn append(&self, batch: Vec<Event>) {
        let priv_ = imp::Timeline::from_instance(self);

        if batch.is_empty() {
            return;
        }
        let mut added = batch.len();

        let index = {
            let index = {
                let mut list = priv_.list.borrow_mut();
                // Extend the size of the list so that rust doesn't need to reallocate memory multiple times
                list.reserve(batch.len());

                if list.is_empty() {
                    priv_
                        .oldest_event
                        .replace(batch.first().as_ref().map(|event| event.matrix_event_id()));
                }

                list.len()
            };

            let mut pending_events = priv_.pending_events.borrow_mut();

            for event in batch.into_iter() {
                let event_id = event.matrix_event_id();

                if let Some(pending_id) = event
                    .matrix_transaction_id()
                    .and_then(|txn_id| pending_events.remove(&txn_id))
                {
                    let mut event_map = priv_.event_map.borrow_mut();

                    if let Some(pending_event) = event_map.remove(&pending_id) {
                        pending_event.set_matrix_pure_event(event.matrix_pure_event());
                        event_map.insert(event_id, pending_event);
                    };
                    added -= 1;
                } else {
                    priv_
                        .event_map
                        .borrow_mut()
                        .insert(event_id.to_owned(), event.clone());
                    if event.is_hidden_event() {
                        self.add_hidden_event(event);
                        added -= 1;
                    } else {
                        priv_.list.borrow_mut().push_back(Item::for_event(event));
                    }
                }
            }

            index
        };

        self.items_changed(index as u32, 0, added as u32);
    }

    /// Append an event that wasn't yet fully sent and received via a sync
    pub fn append_pending(&self, txn_id: Uuid, event: Event) {
        let priv_ = imp::Timeline::from_instance(self);

        priv_
            .event_map
            .borrow_mut()
            .insert(event.matrix_event_id(), event.clone());

        priv_
            .pending_events
            .borrow_mut()
            .insert(txn_id.to_string(), event.matrix_event_id());

        let index = {
            let mut list = priv_.list.borrow_mut();
            let index = list.len();

            if event.is_hidden_event() {
                self.add_hidden_event(event);
                None
            } else {
                list.push_back(Item::for_event(event));
                Some(index)
            }
        };

        if let Some(index) = index {
            self.items_changed(index as u32, 0, 1);
        }
    }

    /// Returns the event with the given id
    pub fn event_by_id(&self, event_id: &EventId) -> Option<Event> {
        // TODO: if the referenced event isn't known to us we will need to request it
        // from the sdk or the matrix homeserver
        let priv_ = imp::Timeline::from_instance(self);
        priv_.event_map.borrow().get(event_id).cloned()
    }

    /// Prepends a batch of events
    // TODO: This should be lazy, see: https://blogs.gnome.org/ebassi/documentation/lazy-loading/
    pub fn prepend(&self, batch: Vec<Event>) {
        let priv_ = imp::Timeline::from_instance(self);
        let mut added = batch.len();

        priv_
            .oldest_event
            .replace(batch.last().as_ref().map(|event| event.matrix_event_id()));

        {
            // Extend the size of the list so that rust doesn't need to reallocate memory multiple times
            priv_.list.borrow_mut().reserve(added);

            for event in batch {
                priv_
                    .event_map
                    .borrow_mut()
                    .insert(event.matrix_event_id(), event.clone());

                if event.is_hidden_event() {
                    self.add_hidden_event(event);
                    added -= 1;
                } else {
                    priv_.list.borrow_mut().push_front(Item::for_event(event));
                }
            }
        }

        self.items_changed(0, 0, added as u32);
    }

    fn set_room(&self, room: Room) {
        let priv_ = imp::Timeline::from_instance(self);
        priv_.room.set(room).unwrap();
    }

    pub fn room(&self) -> &Room {
        let priv_ = imp::Timeline::from_instance(self);
        priv_.room.get().unwrap()
    }

    fn set_loading(&self, loading: bool) {
        let priv_ = imp::Timeline::from_instance(self);

        if loading == priv_.loading.get() {
            return;
        }

        priv_.loading.set(loading);

        self.notify("loading");
    }

    fn set_complete(&self, complete: bool) {
        let priv_ = imp::Timeline::from_instance(self);

        if complete == priv_.complete.get() {
            return;
        }

        priv_.complete.set(complete);
        self.notify("complete");
    }

    // Wether the timeline is full loaded
    pub fn is_complete(&self) -> bool {
        let priv_ = imp::Timeline::from_instance(self);
        priv_.complete.get()
    }

    pub fn loading(&self) -> bool {
        let priv_ = imp::Timeline::from_instance(self);
        priv_.loading.get()
    }

    pub fn is_empty(&self) -> bool {
        let priv_ = imp::Timeline::from_instance(self);
        priv_.list.borrow().is_empty() || (priv_.list.borrow().len() == 1 && self.loading())
    }

    fn oldest_event(&self) -> Option<EventId> {
        let priv_ = imp::Timeline::from_instance(self);
        priv_.oldest_event.borrow().clone()
    }
    fn add_loading_spinner(&self) {
        let priv_ = imp::Timeline::from_instance(self);
        priv_
            .list
            .borrow_mut()
            .push_front(Item::for_loading_spinner());
        self.upcast_ref::<gio::ListModel>().items_changed(0, 0, 1);
    }

    fn remove_loading_spinner(&self) {
        let priv_ = imp::Timeline::from_instance(self);
        priv_.list.borrow_mut().pop_front();
        self.upcast_ref::<gio::ListModel>().items_changed(0, 1, 0);
    }

    pub fn load_previous_events(&self) {
        if self.loading() || self.is_complete() {
            return;
        }

        self.set_loading(true);
        self.add_loading_spinner();

        let matrix_room = self.room().matrix_room();
        let last_event = self.oldest_event();
        let contains_last_event = last_event.is_some();

        let handle = spawn_tokio!(async move {
            matrix_room
                .messages(last_event.as_ref(), None, 20, Direction::Backward)
                .await
        });

        spawn!(
            glib::PRIORITY_LOW,
            clone!(@weak self as obj => async move {
                obj.remove_loading_spinner();

                // FIXME: If the request fails it's automatically restarted because the added events (none), didn't fill the screen.
                // We should block the loading for some time before retrying
                match handle.await.unwrap() {
                       Ok(Some(events)) => {
                            let events: Vec<Event> = if contains_last_event {
                                            events
                                           .into_iter()
                                           .skip(1)
                                           .map(|event| Event::new(event, obj.room())).collect()
                            } else {
                                            events
                                           .into_iter()
                                           .map(|event| Event::new(event, obj.room())).collect()
                            };
                            obj.set_complete(events.iter().any(|event| matches!(event.matrix_event(), Some(AnySyncRoomEvent::State(AnySyncStateEvent::RoomCreate(_))))));
                            obj.prepend(events)
                       },
                       Ok(None) => {
                           error!("The start event wasn't found in the timeline for room {}.", obj.room().room_id());
                       },
                       Err(error) => error!("Couldn't load previous events for room {}: {}", error, obj.room().room_id()),
               }
               obj.set_loading(false);
            })
        );
    }
}

mod event;
mod highlight_flags;
mod item;
mod member;
mod power_levels;
mod room_type;
mod timeline;

pub use self::event::Event;
pub use self::highlight_flags::HighlightFlags;
pub use self::item::Item;
pub use self::item::ItemType;
pub use self::member::Member;
pub use self::power_levels::{PowerLevels, RoomAction};
pub use self::room_type::RoomType;
pub use self::timeline::Timeline;

use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use log::{debug, error, info, warn};
use matrix_sdk::{
    deserialized_responses::{JoinedRoom, LeftRoom},
    room::Room as MatrixRoom,
    ruma::{
        api::client::r0::sync::sync_events::InvitedRoom,
        events::{
            room::{
                member::{MemberEventContent, MembershipState},
                message::{
                    EmoteMessageEventContent, MessageEventContent, MessageType,
                    TextMessageEventContent,
                },
                name::NameEventContent,
                power_levels::PowerLevelsEventContent,
                topic::TopicEventContent,
            },
            tag::TagName,
            AnyRoomAccountDataEvent, AnyStateEventContent, AnyStrippedStateEvent,
            AnySyncMessageEvent, AnySyncRoomEvent, AnySyncStateEvent, EventType, SyncMessageEvent,
            SyncStateEvent, Unsigned,
        },
        identifiers::{EventId, RoomId, UserId},
        serde::Raw,
        MilliSecondsSinceUnixEpoch,
    },
    uuid::Uuid,
    RoomMember,
};
use serde_json::value::RawValue;
use std::cell::RefCell;
use std::convert::{TryFrom, TryInto};
use std::path::PathBuf;

use crate::components::{LabelWithWidgets, Pill};
use crate::prelude::*;
use crate::session::avatar::update_room_avatar_from_file;
use crate::session::{Avatar, Session};
use crate::utils::do_async;
use crate::Error;
use crate::RUNTIME;

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use once_cell::sync::{Lazy, OnceCell};
    use std::cell::Cell;
    use std::collections::HashMap;

    #[derive(Debug, Default)]
    pub struct Room {
        pub room_id: OnceCell<RoomId>,
        pub matrix_room: RefCell<Option<MatrixRoom>>,
        pub session: OnceCell<Session>,
        pub name: RefCell<Option<String>>,
        pub avatar: OnceCell<Avatar>,
        pub category: Cell<RoomType>,
        pub timeline: OnceCell<Timeline>,
        pub room_members: RefCell<HashMap<UserId, Member>>,
        /// The user who sent the invite to this room. This is only set when this room is an invitiation.
        pub inviter: RefCell<Option<Member>>,
        pub members_loaded: Cell<bool>,
        pub power_levels: RefCell<PowerLevels>,
        pub latest_change: RefCell<Option<glib::DateTime>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Room {
        const NAME: &'static str = "Room";
        type Type = super::Room;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for Room {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_string(
                        "room-id",
                        "Room id",
                        "The room id of this Room",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_object(
                        "session",
                        "Session",
                        "The session",
                        Session::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name of this room",
                        None,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_object(
                        "inviter",
                        "Inviter",
                        "The user who sent the invite to this room, this is only set when this room represents an invite",
                        Member::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "avatar",
                        "Avatar",
                        "The Avatar of this room",
                        Avatar::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "timeline",
                        "Timeline",
                        "The timeline of this room",
                        Timeline::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_flags(
                        "highlight",
                        "Highlight",
                        "How this room is highlighted",
                        HighlightFlags::static_type(),
                        HighlightFlags::default().bits(),
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_uint64(
                        "notification-count",
                        "Notification count",
                        "The notification count of this room",
                        std::u64::MIN,
                        std::u64::MAX,
                        0,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_enum(
                        "category",
                        "Category",
                        "The category of this room",
                        RoomType::static_type(),
                        RoomType::default() as i32,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_string(
                        "topic",
                        "Topic",
                        "The topic of this room",
                        None,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_boxed(
                        "latest-change",
                        "Latest Change",
                        "Latest origin_server_ts of all loaded invents",
                        glib::DateTime::static_type(),
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
                "session" => self.session.set(value.get().unwrap()).unwrap(),
                "display-name" => {
                    let room_name = value.get().unwrap();
                    obj.store_room_name(room_name)
                }
                "category" => {
                    let category = value.get().unwrap();
                    obj.set_category(category);
                }
                "room-id" => self
                    .room_id
                    .set(RoomId::try_from(value.get::<&str>().unwrap()).unwrap())
                    .unwrap(),
                "topic" => {
                    let topic = value.get().unwrap();
                    obj.store_topic(topic);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let matrix_room = self.matrix_room.borrow();
            let matrix_room = matrix_room.as_ref().unwrap();
            match pspec.name() {
                "room-id" => obj.room_id().as_str().to_value(),
                "session" => obj.session().to_value(),
                "inviter" => obj.inviter().to_value(),
                "display-name" => obj.display_name().to_value(),
                "avatar" => obj.avatar().to_value(),
                "timeline" => self.timeline.get().unwrap().to_value(),
                "category" => obj.category().to_value(),
                "highlight" => obj.highlight().to_value(),
                "topic" => obj.topic().to_value(),
                "notification-count" => {
                    let highlight = matrix_room.unread_notification_counts().highlight_count;
                    let notification = matrix_room.unread_notification_counts().notification_count;

                    if highlight > 0 {
                        highlight
                    } else {
                        notification
                    }
                    .to_value()
                }
                "latest-change" => obj.latest_change().to_value(),
                _ => unimplemented!(),
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder("order-changed", &[], <()>::static_type().into()).build()]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            obj.set_matrix_room(obj.session().client().get_room(obj.room_id()).unwrap());
            self.timeline.set(Timeline::new(obj)).unwrap();
            self.avatar
                .set(Avatar::new(obj.session(), obj.matrix_room().avatar_url()))
                .unwrap();

            obj.load_power_levels();

            obj.bind_property("display-name", obj.avatar(), "display-name")
                .flags(glib::BindingFlags::SYNC_CREATE)
                .build()
                .unwrap();
        }
    }
}

glib::wrapper! {
    /// GObject representation of a Matrix room.
    ///
    /// Handles populating the Timeline.
    pub struct Room(ObjectSubclass<imp::Room>);
}

impl Room {
    pub fn new(session: &Session, room_id: &RoomId) -> Self {
        glib::Object::new(&[("session", session), ("room-id", &room_id.to_string())])
            .expect("Failed to create Room")
    }

    pub fn session(&self) -> &Session {
        let priv_ = imp::Room::from_instance(self);
        priv_.session.get().unwrap()
    }

    pub fn room_id(&self) -> &RoomId {
        let priv_ = imp::Room::from_instance(self);
        priv_.room_id.get().unwrap()
    }

    fn matrix_room(&self) -> MatrixRoom {
        let priv_ = imp::Room::from_instance(self);
        priv_.matrix_room.borrow().as_ref().unwrap().clone()
    }

    /// Set the new sdk room struct represented by this `Room`
    fn set_matrix_room(&self, matrix_room: MatrixRoom) {
        let priv_ = imp::Room::from_instance(self);

        // Check if the previous type was different
        if let Some(ref old_matrix_room) = *priv_.matrix_room.borrow() {
            let changed = match old_matrix_room {
                MatrixRoom::Joined(_) => !matches!(matrix_room, MatrixRoom::Joined(_)),
                MatrixRoom::Left(_) => !matches!(matrix_room, MatrixRoom::Left(_)),
                MatrixRoom::Invited(_) => !matches!(matrix_room, MatrixRoom::Invited(_)),
            };
            if changed {
                debug!("The matrix room struct for `Room` changed");
            } else {
                return;
            }
        }

        priv_.matrix_room.replace(Some(matrix_room));

        self.load_display_name();
        self.load_category();
    }

    pub fn category(&self) -> RoomType {
        let priv_ = imp::Room::from_instance(self);
        priv_.category.get()
    }

    fn set_category_internal(&self, category: RoomType) {
        let priv_ = imp::Room::from_instance(self);

        if self.category() == category {
            return;
        }

        priv_.category.set(category);
        self.notify("category");
        self.emit_by_name("order-changed", &[]).unwrap();
    }

    /// Set the category of this room.
    ///
    /// This makes the necessary to propagate the category to the homeserver.
    /// Note: Rooms can't be moved to the invite category.
    pub fn set_category(&self, category: RoomType) {
        let matrix_room = self.matrix_room();
        let previous_category = self.category();

        if previous_category == category {
            return;
        }

        if category == RoomType::Invited {
            warn!("Rooms can’t be moved to the invite Category");
            return;
        }

        do_async(
            glib::PRIORITY_DEFAULT_IDLE,
            async move {
                match matrix_room {
                    MatrixRoom::Invited(room) => {
                        match category {
                            RoomType::Invited => Ok(()),
                            RoomType::Favorite => {
                                room.accept_invitation().await
                                // TODO: set favorite tag
                            }
                            RoomType::Normal => room.accept_invitation().await,
                            RoomType::LowPriority => {
                                room.accept_invitation().await
                                // TODO: set low priority tag
                            }
                            RoomType::Left => room.reject_invitation().await,
                        }
                    }
                    MatrixRoom::Joined(room) => {
                        match category {
                            RoomType::Invited => Ok(()),
                            RoomType::Favorite => {
                                // TODO: set favorite tag
                                Ok(())
                            }
                            RoomType::Normal => {
                                // TODO: remove tags
                                Ok(())
                            }
                            RoomType::LowPriority => {
                                // TODO: set low priority tag
                                Ok(())
                            }
                            RoomType::Left => room.leave().await,
                        }
                    }
                    MatrixRoom::Left(room) => {
                        match category {
                            RoomType::Invited => Ok(()),
                            RoomType::Favorite => {
                                room.join().await
                                // TODO: set favorite tag
                            }
                            RoomType::Normal => {
                                room.join().await
                                // TODO: remove tags
                            }
                            RoomType::LowPriority => {
                                room.join().await
                                // TODO: set low priority tag
                            }
                            RoomType::Left => Ok(()),
                        }
                    }
                }
            },
            clone!(@weak self as obj => move |result| async move {
                match result {
                        Ok(_) => {},
                        Err(error) => {
                                error!("Couldn’t set the room category: {}", error);
                                let error = Error::new(
                                        error,
                                        clone!(@weak obj => @default-return None, move |_| {
                                                let error_message = gettext!(
                                                    "Failed to move <widget> from {} to {}.",
                                                    previous_category.to_string(),
                                                    category.to_string()
                                                );
                                                let room_pill = Pill::new();
                                                room_pill.set_room(Some(obj.clone()));
                                                let label = LabelWithWidgets::new(&error_message, vec![room_pill]);

                                                Some(label.upcast())
                                        }),
                                );

                                obj.session().append_error(&error);

                                // Load the previous category
                                obj.load_category();
                        },
                };

            }),
        );

        self.set_category_internal(category);
    }

    pub fn load_category(&self) {
        let matrix_room = self.matrix_room();

        match matrix_room {
            MatrixRoom::Joined(_) => {
                do_async(
                    glib::PRIORITY_DEFAULT_IDLE,
                    async move { matrix_room.tags().await },
                    clone!(@weak self as obj => move |tags_result| async move {
                        let mut category = RoomType::Normal;

                        if let Ok(Some(tags)) = tags_result {
                            if tags.get(&TagName::Favorite).is_some() {
                                category = RoomType::Favorite;
                            } else if tags.get(&TagName::LowPriority).is_some() {
                                category = RoomType::LowPriority;
                            }
                        }

                        obj.set_category_internal(category);
                    }),
                );
            }
            MatrixRoom::Invited(_) => self.set_category_internal(RoomType::Invited),
            MatrixRoom::Left(_) => self.set_category_internal(RoomType::Left),
        };
    }

    pub fn timeline(&self) -> &Timeline {
        let priv_ = imp::Room::from_instance(self);
        priv_.timeline.get().unwrap()
    }

    fn notify_notification_count(&self) {
        self.notify("highlight");
        self.notify("notification-count");
    }

    pub fn highlight(&self) -> HighlightFlags {
        let priv_ = imp::Room::from_instance(self);
        let count = priv_
            .matrix_room
            .borrow()
            .as_ref()
            .unwrap()
            .unread_notification_counts()
            .highlight_count;

        // TODO: how do we know when to set the row to be bold
        if count > 0 {
            HighlightFlags::HIGHLIGHT
        } else {
            HighlightFlags::NONE
        }
    }

    pub fn display_name(&self) -> String {
        let priv_ = imp::Room::from_instance(self);
        let display_name = priv_.name.borrow().clone();
        display_name.unwrap_or_else(|| gettext("Unknown"))
    }

    fn set_display_name(&self, display_name: Option<String>) {
        let priv_ = imp::Room::from_instance(self);

        if Some(self.display_name()) == display_name {
            return;
        }

        priv_.name.replace(display_name);
        self.notify("display-name");
    }

    fn load_display_name(&self) {
        let matrix_room = self.matrix_room();
        do_async(
            glib::PRIORITY_DEFAULT_IDLE,
            async move { matrix_room.display_name().await },
            clone!(@weak self as obj => move |display_name| async move {
                // FIXME: We should retry to if the request failed
                match display_name {
                        Ok(display_name) => obj.set_display_name(Some(display_name)),
                        Err(error) => error!("Couldn’t fetch display name: {}", error),
                };
            }),
        );
    }

    /// Updates the Matrix room with the given name.
    pub fn store_room_name(&self, room_name: String) {
        if self.display_name() == room_name {
            return;
        }

        let joined_room = match self.matrix_room() {
            MatrixRoom::Joined(joined_room) => joined_room,
            _ => {
                error!("Room name can’t be changed when not a member.");
                return;
            }
        };
        let room_name = match room_name.try_into() {
            Ok(room_name) => room_name,
            Err(e) => {
                error!("Invalid room name: {}", e);
                return;
            }
        };
        let name_content = NameEventContent::new(Some(room_name));

        do_async(
            glib::PRIORITY_DEFAULT_IDLE,
            async move {
                let content = AnyStateEventContent::RoomName(name_content);
                joined_room.send_state_event(content, "").await
            },
            clone!(@weak self as obj => move |room_name| async move {
                match room_name {
                    Ok(_room_name) => info!("Successfully updated room name"),
                    Err(error) => error!("Couldn’t update room name: {}", error),
                };
            }),
        );
    }

    pub fn avatar(&self) -> &Avatar {
        let priv_ = imp::Room::from_instance(self);
        priv_.avatar.get().unwrap()
    }

    pub fn topic(&self) -> Option<String> {
        self.matrix_room()
            .topic()
            .filter(|topic| !topic.is_empty() && topic.find(|c: char| !c.is_whitespace()).is_some())
    }

    /// Updates the Matrix room with the given topic.
    pub fn store_topic(&self, topic: String) {
        if self.topic().as_ref() == Some(&topic) {
            return;
        }

        let joined_room = match self.matrix_room() {
            MatrixRoom::Joined(joined_room) => joined_room,
            _ => {
                error!("Room topic can’t be changed when not a member.");
                return;
            }
        };

        do_async(
            glib::PRIORITY_DEFAULT_IDLE,
            async move {
                let content = AnyStateEventContent::RoomTopic(TopicEventContent::new(topic));
                joined_room.send_state_event(content, "").await
            },
            clone!(@weak self as obj => move |topic| async move {
                match topic {
                    Ok(_topic) => info!("Successfully updated room topic"),
                    Err(error) => error!("Couldn’t update topic: {}", error),
                };
            }),
        );
    }

    pub fn power_levels(&self) -> PowerLevels {
        let priv_ = imp::Room::from_instance(self);
        priv_.power_levels.borrow().clone()
    }

    pub fn inviter(&self) -> Option<Member> {
        let priv_ = imp::Room::from_instance(self);
        priv_.inviter.borrow().clone()
    }

    /// Returns the room member `User` object
    ///
    /// The returned `User` is specific to this room
    pub fn member_by_id(&self, user_id: &UserId) -> Member {
        let priv_ = imp::Room::from_instance(self);
        let mut room_members = priv_.room_members.borrow_mut();

        room_members
            .entry(user_id.clone())
            .or_insert_with(|| Member::new(self, user_id))
            .clone()
    }

    /// Handle stripped state events.
    ///
    /// Events passed to this function aren't added to the timeline.
    pub fn handle_invite_events(&self, events: Vec<AnyStrippedStateEvent>) {
        let priv_ = imp::Room::from_instance(self);
        let invite_event = events
            .iter()
            .find(|event| {
                if let AnyStrippedStateEvent::RoomMember(event) = event {
                    event.content.membership == MembershipState::Invite
                        && event.state_key == self.session().user().user_id().as_str()
                } else {
                    false
                }
            })
            .unwrap();

        let inviter_id = invite_event.sender();

        let inviter_event = events.iter().find(|event| {
            if let AnyStrippedStateEvent::RoomMember(event) = event {
                &event.sender == inviter_id
            } else {
                false
            }
        });

        let inviter = Member::new(self, inviter_id);
        if let Some(AnyStrippedStateEvent::RoomMember(event)) = inviter_event {
            inviter.update_from_member_event(event);
        }

        priv_.inviter.replace(Some(inviter));
        self.notify("inviter");
    }

    /// Add new events to the timeline
    pub fn append_events(&self, batch: Vec<Event>) {
        let priv_ = imp::Room::from_instance(self);

        //FIXME: notify only when the count has changed
        self.notify_notification_count();

        let mut latest_change = self.latest_change();
        for event in batch.iter().flat_map(Event::matrix_event) {
            match &event {
                AnySyncRoomEvent::State(AnySyncStateEvent::RoomMember(event)) => {
                    self.update_member_for_member_event(event)
                }
                AnySyncRoomEvent::State(AnySyncStateEvent::RoomAvatar(event)) => {
                    self.avatar().set_url(event.content.url.to_owned());
                }
                AnySyncRoomEvent::State(AnySyncStateEvent::RoomName(_)) => {
                    // FIXME: this doesn't take into account changes in the calculated name
                    self.load_display_name()
                }
                AnySyncRoomEvent::State(AnySyncStateEvent::RoomTopic(_)) => {
                    self.notify("topic");
                }
                AnySyncRoomEvent::State(AnySyncStateEvent::RoomPowerLevels(event)) => {
                    self.power_levels().update_from_event(event.clone());
                }
                _ => {}
            }
            let event_ts = glib::DateTime::from_unix_millis_utc(event.origin_server_ts());
            latest_change = latest_change.max(event_ts.ok());
        }

        priv_.timeline.get().unwrap().append(batch);
        priv_.latest_change.replace(latest_change);
        self.notify("latest-change");
        self.emit_by_name("order-changed", &[]).unwrap();
    }

    /// Returns the point in time this room received its latest event.
    pub fn latest_change(&self) -> Option<glib::DateTime> {
        let priv_ = imp::Room::from_instance(self);
        priv_.latest_change.borrow().clone()
    }

    /// Add an initial set of members needed to display room events
    ///
    /// The `Timeline` makes sure to update the members when a member state event arrives
    fn add_members(&self, members: Vec<RoomMember>) {
        let priv_ = imp::Room::from_instance(self);
        let mut room_members = priv_.room_members.borrow_mut();
        for member in members {
            let user_id = member.user_id();
            let user = room_members
                .entry(user_id.clone())
                .or_insert_with(|| Member::new(self, user_id));
            user.update_from_room_member(&member);
        }
    }

    /// Updates a room member based on the room member state event
    fn update_member_for_member_event(&self, event: &SyncStateEvent<MemberEventContent>) {
        let priv_ = imp::Room::from_instance(self);
        let mut room_members = priv_.room_members.borrow_mut();
        let user_id = &event.sender;
        let user = room_members
            .entry(user_id.clone())
            .or_insert_with(|| Member::new(self, user_id));
        user.update_from_member_event(event);
    }

    pub fn load_members(&self) {
        let priv_ = imp::Room::from_instance(self);
        if priv_.members_loaded.get() {
            return;
        }

        priv_.members_loaded.set(true);
        let matrix_room = self.matrix_room();
        do_async(
            glib::PRIORITY_LOW,
            async move { matrix_room.active_members().await },
            clone!(@weak self as obj => move |members| async move {
                // FIXME: We should retry to load the room members if the request failed
                let priv_ = imp::Room::from_instance(&obj);
                match members {
                        Ok(members) => obj.add_members(members),
                        Err(error) => {
                            priv_.members_loaded.set(false);
                            error!("Couldn’t load room members: {}", error)
                        },
                };
            }),
        );
    }

    pub fn load_previous_events(&self) {
        warn!("Loading previous events is not yet implemented");
        /*
        let matrix_room = priv_.matrix_room.get().unwrap().clone();
        do_async(
            async move { matrix_room.messages().await },
            clone!(@weak self as obj => move |events| async move {
                // FIXME: We should retry to load the room members if the request failed
                match events {
                        Ok(events) => obj.prepend(events),
                        Err(error) => error!("Couldn’t load room members: {}", error),
                };
            }),
        );
        */
    }

    fn load_power_levels(&self) {
        let matrix_room = self.matrix_room();
        do_async(
            glib::PRIORITY_DEFAULT_IDLE,
            async move {
                let state_event = match matrix_room
                    .get_state_event(EventType::RoomPowerLevels, "")
                    .await
                {
                    Ok(state_event) => state_event,
                    Err(e) => {
                        error!("Initial load of room power levels failed: {}", e);
                        return None;
                    }
                };

                state_event
                    .and_then(|e| e.deserialize().ok())
                    .and_then(|e| {
                        if let AnySyncStateEvent::RoomPowerLevels(e) = e {
                            Some(e)
                        } else {
                            None
                        }
                    })
            },
            clone!(@weak self as obj => move |event: Option<SyncStateEvent<PowerLevelsEventContent>>| async move {
                if let Some(event) = event {
                    obj.power_levels().update_from_event(event);
                }
            }),
        );
    }

    pub fn send_text_message(&self, body: &str, markdown_enabled: bool) {
        let content = if let Some(body) = body.strip_prefix("/me ") {
            let emote = if markdown_enabled {
                EmoteMessageEventContent::markdown(body)
            } else {
                EmoteMessageEventContent::plain(body)
            };
            MessageEventContent::new(MessageType::Emote(emote))
        } else {
            let text = if markdown_enabled {
                TextMessageEventContent::markdown(body)
            } else {
                TextMessageEventContent::plain(body)
            };
            MessageEventContent::new(MessageType::Text(text))
        };

        let txn_id = Uuid::new_v4();

        let pending_event = AnySyncMessageEvent::RoomMessage(SyncMessageEvent {
            content,
            event_id: EventId::try_from(format!("${}:fractal.gnome.org", txn_id)).unwrap(),
            sender: self.session().user().user_id().clone(),
            origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
            unsigned: Unsigned::default(),
        });

        self.send_message(txn_id, pending_event);
    }

    pub fn send_message(&self, txn_id: Uuid, event: AnySyncMessageEvent) {
        let priv_ = imp::Room::from_instance(self);
        let content = event.content();

        if let MatrixRoom::Joined(matrix_room) = self.matrix_room() {
            let pending_id = event.event_id().clone();
            let json = serde_json::to_string(&AnySyncRoomEvent::Message(event)).unwrap();
            let raw_event: Raw<AnySyncRoomEvent> =
                Raw::from_json(RawValue::from_string(json).unwrap());
            let event = Event::new(raw_event.into(), self);
            priv_.timeline.get().unwrap().append_pending(event);

            do_async(
                glib::PRIORITY_DEFAULT_IDLE,
                async move { matrix_room.send(content, Some(txn_id)).await },
                clone!(@weak self as obj => move |result| async move {
                    // FIXME: We should retry the request if it fails
                    match result {
                            Ok(result) => obj.timeline().set_event_id_for_pending(pending_id, result.event_id),
                            Err(error) => error!("Couldn’t send message: {}", error),
                    };
                }),
            );
        }
    }

    /// Creates an expression that is true when the user is allowed the given action.
    pub fn new_allowed_expr(&self, room_action: RoomAction) -> gtk::Expression {
        let user_id = self.session().user().user_id();
        let member = self.member_by_id(user_id);
        self.power_levels().new_allowed_expr(&member, room_action)
    }

    /// Uploads the given file to the server and makes it the room avatar.
    ///
    /// Removes the avatar if no filename is given.
    pub fn store_avatar(&self, filename: Option<PathBuf>) {
        let matrix_room = self.matrix_room();
        let client = self.session().client().clone();

        do_async(
            glib::PRIORITY_DEFAULT_IDLE,
            async move { update_room_avatar_from_file(&client, &matrix_room, filename.as_ref()).await },
            clone!(@weak self as this => move |avatar_uri| async move {
                match avatar_uri {
                    Ok(_avatar_uri) => info!("Sucessfully updated room avatar"),
                    Err(error) => error!("Couldn’t update room avatar: {}", error),
                };
            }),
        );
    }

    pub async fn accept_invite(&self) -> Result<(), Error> {
        let matrix_room = self.matrix_room();

        if let MatrixRoom::Invited(matrix_room) = matrix_room {
            let (sender, receiver) = futures::channel::oneshot::channel();
            RUNTIME.spawn(async move { sender.send(matrix_room.accept_invitation().await) });
            match receiver.await.unwrap() {
                Ok(result) => Ok(result),
                Err(error) => {
                    error!("Accepting invitation failed: {}", error);
                    let error = Error::new(
                        error,
                        clone!(@strong self as room => move |_| {
                                let error_message = gettext("Failed to accept invitation for <widget>. Try again later.");
                                let room_pill = Pill::new();
                                room_pill.set_room(Some(room.clone()));
                                let error_label = LabelWithWidgets::new(&error_message, vec![room_pill]);
                                Some(error_label.upcast())
                        }),
                    );
                    self.session().append_error(&error);
                    Err(error)
                }
            }
        } else {
            error!("Can’t accept invite, because this room isn’t an invited room");
            Ok(())
        }
    }

    pub async fn reject_invite(&self) -> Result<(), Error> {
        let matrix_room = self.matrix_room();

        if let MatrixRoom::Invited(matrix_room) = matrix_room {
            let (sender, receiver) = futures::channel::oneshot::channel();
            RUNTIME.spawn(async move { sender.send(matrix_room.reject_invitation().await) });
            match receiver.await.unwrap() {
                Ok(result) => Ok(result),
                Err(error) => {
                    error!("Rejecting invitation failed: {}", error);
                    let error = Error::new(
                        error,
                        clone!(@strong self as room => move |_| {
                                let error_message = gettext("Failed to reject invitation for <widget>. Try again later.");
                                let room_pill = Pill::new();
                                room_pill.set_room(Some(room.clone()));
                                let error_label = LabelWithWidgets::new(&error_message, vec![room_pill]);
                                Some(error_label.upcast())
                        }),
                    );
                    self.session().append_error(&error);
                    Err(error)
                }
            }
        } else {
            error!("Can’t reject invite, because this room isn’t an invited room");
            Ok(())
        }
    }

    pub fn handle_left_response(&self, response_room: LeftRoom) {
        self.set_matrix_room(self.session().client().get_room(self.room_id()).unwrap());

        self.append_events(
            response_room
                .timeline
                .events
                .into_iter()
                .map(|event| Event::new(event, self))
                .collect(),
        );
    }

    pub fn handle_joined_response(&self, response_room: JoinedRoom) {
        self.set_matrix_room(self.session().client().get_room(self.room_id()).unwrap());

        if response_room
            .account_data
            .events
            .iter()
            .any(|e| matches!(e.deserialize(), Ok(AnyRoomAccountDataEvent::Tag(_))))
        {
            self.load_category();
        }

        self.append_events(
            response_room
                .timeline
                .events
                .into_iter()
                .map(|event| Event::new(event, self))
                .collect(),
        );
    }

    pub fn handle_invited_response(&self, response_room: InvitedRoom) {
        self.set_matrix_room(self.session().client().get_room(self.room_id()).unwrap());

        self.handle_invite_events(
            response_room
                .invite_state
                .events
                .into_iter()
                .filter_map(|event| {
                    if let Ok(event) = event.deserialize() {
                        Some(event)
                    } else {
                        error!("Couldn’t deserialize event: {:?}", event);
                        None
                    }
                })
                .collect(),
        )
    }

    pub fn connect_order_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("order-changed", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            f(&obj);
            None
        })
        .unwrap()
    }
}

trait GlibDateTime {
    /// Creates a glib::DateTime from the given unix time.
    fn from_unix_millis_utc(
        unix_time: &MilliSecondsSinceUnixEpoch,
    ) -> Result<glib::DateTime, glib::BoolError> {
        let millis: f64 = unix_time.get().into();
        let unix_epoch = glib::DateTime::from_unix_utc(0)?;
        unix_epoch.add_seconds(millis / 1000.0)
    }
}
impl GlibDateTime for glib::DateTime {}

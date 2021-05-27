use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use log::{debug, error, warn};
use matrix_sdk::{
    api::r0::sync::sync_events::InvitedRoom,
    deserialized_responses::{JoinedRoom, LeftRoom},
    events::{
        room::{
            member::{MemberEventContent, MembershipState},
            message::{
                EmoteMessageEventContent, MessageEventContent, MessageType, TextMessageEventContent,
            },
        },
        tag::TagName,
        AnyMessageEvent, AnyRoomAccountDataEvent, AnyRoomEvent, AnyStateEvent,
        AnyStrippedStateEvent, AnySyncRoomEvent, MessageEvent, StateEvent, Unsigned,
    },
    identifiers::{EventId, RoomId, UserId},
    room::Room as MatrixRoom,
    uuid::Uuid,
    MilliSecondsSinceUnixEpoch, RoomMember,
};
use std::cell::RefCell;

use crate::components::{LabelWithWidgets, UserPill};
use crate::event_from_sync_event;
use crate::session::{
    categories::CategoryType,
    room::{HighlightFlags, Timeline},
    User,
};
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
        pub matrix_room: RefCell<Option<MatrixRoom>>,
        pub user: OnceCell<User>,
        pub name: RefCell<Option<String>>,
        pub avatar: RefCell<Option<gio::LoadableIcon>>,
        pub category: Cell<CategoryType>,
        pub timeline: OnceCell<Timeline>,
        pub room_members: RefCell<HashMap<UserId, User>>,
        /// The user of this room
        pub user_id: OnceCell<UserId>,
        /// The user who send the invite to this room. This is only set when this room is an invitiation.
        pub inviter: RefCell<Option<User>>,
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
                    glib::ParamSpec::new_boxed(
                        "matrix-room",
                        "Matrix room",
                        "The underlaying matrix room.",
                        BoxedMatrixRoom::static_type(),
                        glib::ParamFlags::WRITABLE | glib::ParamFlags::CONSTRUCT,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name of this room",
                        None,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "user",
                        "User",
                        "The user of the session that owns this room",
                        User::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_object(
                        "inviter",
                        "Inviter",
                        "The user who send the invite to this room, this is only set when this room rapresents an invite",
                        User::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "avatar",
                        "Avatar",
                        "The url of the avatar of this room",
                        gio::LoadableIcon::static_type(),
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
                        CategoryType::static_type(),
                        CategoryType::default() as i32,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_string(
                        "topic",
                        "Topic",
                        "The topic of this room",
                        None,
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
                "matrix-room" => {
                    let matrix_room = value.get::<BoxedMatrixRoom>().unwrap();
                    obj.set_matrix_room(matrix_room.0);
                }
                "user" => {
                    let user = value.get().unwrap();
                    self.user.set(user).unwrap();
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let matrix_room = self.matrix_room.borrow();
            let matrix_room = matrix_room.as_ref().unwrap();
            match pspec.name() {
                "user" => obj.user().to_value(),
                "inviter" => obj.inviter().to_value(),
                "display-name" => obj.display_name().to_value(),
                "avatar" => self.avatar.borrow().to_value(),
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
                _ => unimplemented!(),
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder(
                    "error",
                    &[Error::static_type().into()],
                    <()>::static_type().into(),
                )
                .build()]
            });
            SIGNALS.as_ref()
        }
    }
}

glib::wrapper! {
    pub struct Room(ObjectSubclass<imp::Room>);
}

#[derive(Clone, Debug, glib::GBoxed)]
#[gboxed(type_name = "BoxedMatrixRoom")]
struct BoxedMatrixRoom(MatrixRoom);

impl Room {
    pub fn new(room: MatrixRoom, user: &User) -> Self {
        glib::Object::new(&[("matrix-room", &BoxedMatrixRoom(room)), ("user", user)])
            .expect("Failed to create Room")
    }

    pub fn matrix_room_id(&self) -> RoomId {
        let priv_ = imp::Room::from_instance(self);
        priv_
            .matrix_room
            .borrow()
            .as_ref()
            .unwrap()
            .room_id()
            .clone()
    }

    fn matrix_room(&self) -> MatrixRoom {
        let priv_ = imp::Room::from_instance(self);
        priv_.matrix_room.borrow().as_ref().unwrap().clone()
    }

    /// Set the new sdk room struct represented by this `Room`
    pub fn set_matrix_room(&self, matrix_room: MatrixRoom) {
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
        // We create the timeline once
        priv_.timeline.get_or_init(|| Timeline::new(self));

        self.load_members();
        self.load_display_name();
        self.load_category();
    }

    pub fn user(&self) -> &User {
        let priv_ = imp::Room::from_instance(self);
        priv_.user.get().unwrap()
    }

    pub fn category(&self) -> CategoryType {
        let priv_ = imp::Room::from_instance(self);
        priv_.category.get()
    }

    // TODO: makes this method public and propagate the category to the homeserver via the sdk
    fn set_category(&self, category: CategoryType) {
        let priv_ = imp::Room::from_instance(self);
        if self.category() == category {
            return;
        }

        priv_.category.set(category);
        self.notify("category");
    }

    pub fn load_category(&self) {
        let matrix_room = self.matrix_room();

        match matrix_room {
            MatrixRoom::Joined(_) => {
                do_async(
                    glib::PRIORITY_DEFAULT_IDLE,
                    async move { matrix_room.tags().await },
                    clone!(@weak self as obj => move |tags_result| async move {
                        let mut category = CategoryType::Normal;

                        if let Ok(Some(tags)) = tags_result {
                            if tags.get(&TagName::Favorite).is_some() {
                                category = CategoryType::Favorite;
                            } else if tags.get(&TagName::LowPriority).is_some() {
                                category = CategoryType::LowPriority;
                            }
                        }

                        obj.set_category(category);
                    }),
                );
            }
            MatrixRoom::Invited(_) => self.set_category(CategoryType::Invited),
            MatrixRoom::Left(_) => self.set_category(CategoryType::Left),
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
        let priv_ = imp::Room::from_instance(&self);
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
        let priv_ = imp::Room::from_instance(&self);
        priv_.name.borrow().to_owned().unwrap_or(gettext("Unknown"))
    }

    fn set_display_name(&self, display_name: Option<String>) {
        let priv_ = imp::Room::from_instance(&self);

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
                        Err(error) => error!("Couldn't fetch display name: {}", error),
                };
            }),
        );
    }

    pub fn topic(&self) -> Option<String> {
        self.matrix_room()
            .topic()
            .filter(|topic| !topic.is_empty() && topic.find(|c: char| !c.is_whitespace()).is_some())
    }

    pub fn inviter(&self) -> Option<User> {
        let priv_ = imp::Room::from_instance(&self);
        priv_.inviter.borrow().clone()
    }

    /// Returns the room member `User` object
    ///
    /// The returned `User` is specific to this room
    pub fn member_by_id(&self, user_id: &UserId) -> User {
        let priv_ = imp::Room::from_instance(self);
        let mut room_members = priv_.room_members.borrow_mut();

        room_members
            .entry(user_id.clone())
            .or_insert(User::new(&user_id))
            .clone()
    }

    /// Handle stripped state events.
    ///
    /// Events passed to this function arn't added to the timeline.
    pub fn handle_invite_events(&self, events: Vec<AnyStrippedStateEvent>) {
        let priv_ = imp::Room::from_instance(self);
        let invite_event = events
            .iter()
            .find(|event| {
                if let AnyStrippedStateEvent::RoomMember(event) = event {
                    event.content.membership == MembershipState::Invite
                        && event.state_key == self.user().user_id().as_str()
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

        let inviter = User::new(inviter_id);
        if let Some(AnyStrippedStateEvent::RoomMember(event)) = inviter_event {
            inviter.update_from_stripped_member_event(event);
        }

        priv_.inviter.replace(Some(inviter));
        self.notify("inviter");
    }

    /// Add new events to the timeline
    pub fn append_events(&self, batch: Vec<AnyRoomEvent>) {
        let priv_ = imp::Room::from_instance(self);

        //FIXME: notify only when the count has changed
        self.notify_notification_count();

        for event in batch.iter() {
            match event {
                AnyRoomEvent::State(AnyStateEvent::RoomMember(ref event)) => {
                    self.update_member_for_member_event(event)
                }
                AnyRoomEvent::State(AnyStateEvent::RoomName(_)) => {
                    // FIXME: this doesn't take in account changes in the calculated name
                    self.load_display_name()
                }
                AnyRoomEvent::State(AnyStateEvent::RoomTopic(_)) => {
                    self.notify("topic");
                }
                _ => {}
            }
        }

        priv_.timeline.get().unwrap().append(batch);
    }

    /// Add an initial set of members needed to diplay room events
    ///
    /// The `Timeline` makes sure to update the members when a member state event arrives
    fn add_members(&self, members: Vec<RoomMember>) {
        let priv_ = imp::Room::from_instance(self);
        let mut room_members = priv_.room_members.borrow_mut();
        for member in members {
            let user_id = member.user_id();
            let user = room_members
                .entry(user_id.clone())
                .or_insert(User::new(user_id));
            user.update_from_room_member(&member);
        }
    }

    /// Updates a room member based on the room member state event
    fn update_member_for_member_event(&self, event: &StateEvent<MemberEventContent>) {
        let priv_ = imp::Room::from_instance(self);
        let mut room_members = priv_.room_members.borrow_mut();
        let user_id = &event.sender;
        let user = room_members
            .entry(user_id.clone())
            .or_insert(User::new(user_id));
        user.update_from_member_event(event);
    }

    fn load_members(&self) {
        let matrix_room = self.matrix_room();
        do_async(
            glib::PRIORITY_LOW,
            async move { matrix_room.active_members().await },
            clone!(@weak self as obj => move |members| async move {
                // FIXME: We should retry to load the room members if the request failed
                match members {
                        Ok(members) => obj.add_members(members),
                        Err(error) => error!("Couldn't load room members: {}", error),
                };
            }),
        );
    }

    pub fn load_previous_events(&self) {
        warn!("Loading previous evetns is not yet implemented");
        /*
        let matrix_room = priv_.matrix_room.get().unwrap().clone();
        do_async(
            async move { matrix_room.messages().await },
            clone!(@weak self as obj => move |events| async move {
                // FIXME: We should retry to load the room members if the request failed
                match events {
                        Ok(events) => obj.prepend(events),
                        Err(error) => error!("Couldn't load room members: {}", error),
                };
            }),
        );
        */
    }

    pub fn send_text_message(&self, body: &str, markdown_enabled: bool) {
        use std::convert::TryFrom;
        if let MatrixRoom::Joined(matrix_room) = self.matrix_room() {
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

            let pending_event = AnyMessageEvent::RoomMessage(MessageEvent {
                content,
                event_id: EventId::try_from(format!("${}:fractal.gnome.org", txn_id)).unwrap(),
                sender: self.user().user_id().clone(),
                origin_server_ts: MilliSecondsSinceUnixEpoch::now(),
                room_id: matrix_room.room_id().clone(),
                unsigned: Unsigned::default(),
            });

            self.send_message(txn_id, pending_event);
        }
    }

    pub fn send_message(&self, txn_id: Uuid, event: AnyMessageEvent) {
        let priv_ = imp::Room::from_instance(self);
        let content = event.content();

        if let MatrixRoom::Joined(matrix_room) = self.matrix_room() {
            let pending_id = event.event_id().clone();
            priv_
                .timeline
                .get()
                .unwrap()
                .append_pending(AnyRoomEvent::Message(event));

            do_async(
                glib::PRIORITY_DEFAULT_IDLE,
                async move { matrix_room.send(content, Some(txn_id)).await },
                clone!(@weak self as obj => move |result| async move {
                    // FIXME: We should retry the request if it fails
                    match result {
                            Ok(result) => {
                                    let priv_ = imp::Room::from_instance(&obj);
                                    priv_.timeline.get().unwrap().set_event_id_for_pending(pending_id, result.event_id)
                            },
                            Err(error) => error!("Couldn't send message: {}", error),
                    };
                }),
            );
        }
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
                                let room_pill = UserPill::new();
                                room_pill.set_room(Some(room.clone()));
                                let error_label = LabelWithWidgets::new(&error_message, vec![room_pill]);
                                Some(error_label.upcast())
                        }),
                    );
                    self.emit_by_name("error", &[&error]).unwrap();
                    Err(error)
                }
            }
        } else {
            error!("Can't accept invite, because this room isn't an invited room");
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
                                let room_pill = UserPill::new();
                                room_pill.set_room(Some(room.clone()));
                                let error_label = LabelWithWidgets::new(&error_message, vec![room_pill]);
                                Some(error_label.upcast())
                        }),
                    );
                    self.emit_by_name("error", &[&error]).unwrap();
                    Err(error)
                }
            }
        } else {
            error!("Can't reject invite, because this room isn't an invited room");
            Ok(())
        }
    }

    pub fn handle_left_response(&self, response_room: LeftRoom, matrix_room: MatrixRoom) {
        self.set_matrix_room(matrix_room);

        let room_id = self.matrix_room_id();

        self.append_events(
            response_room
                .timeline
                .events
                .into_iter()
                .filter_map(|event| {
                    if let Ok(event) = event.event.deserialize() {
                        Some(event)
                    } else {
                        error!("Couldn't deserialize event: {:?}", event);
                        None
                    }
                })
                .map(|event| event_from_sync_event!(event, room_id))
                .collect(),
        );
    }

    pub fn handle_joined_response(&self, response_room: JoinedRoom, matrix_room: MatrixRoom) {
        self.set_matrix_room(matrix_room);

        if response_room
            .account_data
            .events
            .iter()
            .any(|e| matches!(e.deserialize(), Ok(AnyRoomAccountDataEvent::Tag(_))))
        {
            self.load_category();
        }

        let room_id = self.matrix_room_id();

        self.append_events(
            response_room
                .timeline
                .events
                .into_iter()
                .filter_map(|event| {
                    if let Ok(event) = event.event.deserialize() {
                        Some(event)
                    } else {
                        error!("Couldn't deserialize event: {:?}", event);
                        None
                    }
                })
                .map(|event| event_from_sync_event!(event, room_id))
                .collect(),
        );
    }

    pub fn handle_invited_response(&self, response_room: InvitedRoom, matrix_room: MatrixRoom) {
        self.set_matrix_room(matrix_room);

        self.handle_invite_events(
            response_room
                .invite_state
                .events
                .into_iter()
                .filter_map(|event| {
                    if let Ok(event) = event.deserialize() {
                        Some(event)
                    } else {
                        error!("Couldn't deserialize event: {:?}", event);
                        None
                    }
                })
                .collect(),
        )
    }

    pub fn connect_error<F: Fn(&Self, Error) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("error", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            let error = values[1].get::<Error>().unwrap();

            f(&obj, error);

            None
        })
        .unwrap()
    }
}

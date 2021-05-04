use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use log::{error, warn};
use matrix_sdk::{
    events::{room::member::MemberEventContent, AnyRoomEvent, AnyStateEvent, StateEvent},
    identifiers::UserId,
    room::Room as MatrixRoom,
    RoomMember,
};

use crate::session::{
    categories::CategoryType,
    room::{HighlightFlags, Timeline},
    User,
};
use crate::utils::do_async;

mod imp {
    use super::*;
    use once_cell::sync::{Lazy, OnceCell};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    #[derive(Debug, Default)]
    pub struct Room {
        pub matrix_room: OnceCell<MatrixRoom>,
        pub user: OnceCell<User>,
        pub name: RefCell<Option<String>>,
        pub avatar: RefCell<Option<gio::LoadableIcon>>,
        pub category: Cell<CategoryType>,
        pub timeline: OnceCell<Timeline>,
        pub room_members: RefCell<HashMap<UserId, User>>,
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
                        glib::ParamFlags::WRITABLE | glib::ParamFlags::CONSTRUCT_ONLY,
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
            let matrix_room = self.matrix_room.get().unwrap();
            match pspec.name() {
                "user" => obj.user().to_value(),
                "display-name" => obj.display_name().to_value(),
                "avatar" => self.avatar.borrow().to_value(),
                "timeline" => self.timeline.get().unwrap().to_value(),
                "category" => obj.category().to_value(),
                "highlight" => obj.highlight().to_value(),
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

    pub fn matrix_room(&self) -> &MatrixRoom {
        let priv_ = imp::Room::from_instance(self);
        priv_.matrix_room.get().unwrap()
    }

    fn set_matrix_room(&self, matrix_room: MatrixRoom) {
        let priv_ = imp::Room::from_instance(self);

        let category = match matrix_room {
            MatrixRoom::Joined(_) => CategoryType::Normal,
            MatrixRoom::Invited(_) => CategoryType::Invited,
            MatrixRoom::Left(_) => CategoryType::Left,
        };

        priv_.matrix_room.set(matrix_room).unwrap();
        priv_.timeline.set(Timeline::new(self)).unwrap();

        // We only need to load the room members once, because updates we will receive via state events
        self.load_members();
        self.load_display_name();
        // TODO: change category when room type changes
        self.set_category(category);
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
            .get()
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
        let priv_ = imp::Room::from_instance(&self);
        let matrix_room = priv_.matrix_room.get().unwrap().clone();
        do_async(
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
        let priv_ = imp::Room::from_instance(self);

        let matrix_room = priv_.matrix_room.get().unwrap().clone();
        do_async(
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
}

use gtk::{
    gio,
    glib::{self, clone},
    prelude::*,
    subclass::prelude::*,
};
use indexmap::{map::Entry, IndexMap};
use matrix_sdk::{
    ruma::{
        events::{room::member::RoomMemberEventContent, OriginalSyncStateEvent},
        OwnedUserId, UserId,
    },
    RoomMemberships,
};
use tracing::error;

use super::{Event, Member, Membership, Room};
use crate::{spawn, spawn_tokio, utils::LoadingState};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::object::WeakRef;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default)]
    pub struct MemberList {
        /// The list of known members.
        pub members: RefCell<IndexMap<OwnedUserId, Member>>,
        /// The room these members belong to.
        pub room: WeakRef<Room>,
        /// The loading state of the list.
        pub state: Cell<LoadingState>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberList {
        const NAME: &'static str = "MemberList";
        type Type = super::MemberList;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for MemberList {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<Room>("room")
                        .construct_only()
                        .build(),
                    glib::ParamSpecEnum::builder::<LoadingState>("state")
                        .read_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "room" => self.obj().set_room(&value.get().ok().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "room" => obj.room().to_value(),
                "state" => obj.state().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl ListModelImpl for MemberList {
        fn item_type(&self) -> glib::Type {
            Member::static_type()
        }

        fn n_items(&self) -> u32 {
            self.members.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let members = self.members.borrow();

            members
                .get_index(position as usize)
                .map(|(_user_id, member)| member.clone().upcast())
        }
    }
}

glib::wrapper! {
    /// List of all Members in a room. Implements ListModel.
    ///
    /// Members are sorted in "insertion order", not anything useful.
    pub struct MemberList(ObjectSubclass<imp::MemberList>)
        @implements gio::ListModel;
}

impl MemberList {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// The room containing these members.
    pub fn room(&self) -> Room {
        self.imp().room.upgrade().unwrap()
    }

    fn set_room(&self, room: &Room) {
        self.imp().room.set(Some(room));
        self.notify("room");

        spawn!(
            glib::Priority::LOW,
            clone!(@weak self as obj => async move {
                obj.load().await;
            })
        );
    }

    /// The state of this list.
    pub fn state(&self) -> LoadingState {
        self.imp().state.get()
    }

    /// Set whether this list is being loaded.
    fn set_state(&self, state: LoadingState) {
        if self.state() == state {
            return;
        }

        self.imp().state.set(state);
        self.notify("state");
    }

    pub fn reload(&self) {
        self.set_state(LoadingState::Initial);

        spawn!(clone!(@weak self as obj => async move {
            obj.load().await;
        }));
    }

    /// Load this list.
    async fn load(&self) {
        if matches!(self.state(), LoadingState::Loading | LoadingState::Ready) {
            return;
        }

        self.set_state(LoadingState::Loading);

        let room = self.room();
        let matrix_room = room.matrix_room();

        // First load what we have locally.
        let matrix_room_clone = matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let mut memberships = RoomMemberships::all();
            memberships.remove(RoomMemberships::LEAVE);

            matrix_room_clone.members_no_sync(memberships).await
        });

        match handle.await.unwrap() {
            Ok(members) => {
                self.update_from_room_members(&members);

                if matrix_room.are_members_synced() {
                    // Nothing more to do, we can stop here.
                    self.set_state(LoadingState::Ready);
                    return;
                }
            }
            Err(error) => {
                error!("Failed to load room members from store: {error}");
            }
        }

        // We don't have everything locally, request the rest from the server.
        let handle = spawn_tokio!(async move {
            let mut memberships = RoomMemberships::all();
            memberships.remove(RoomMemberships::LEAVE);

            matrix_room.members(memberships).await
        });

        // FIXME: We should retry to load the room members if the request failed
        match handle.await.unwrap() {
            Ok(members) => {
                // Add all members needed to display room events.
                self.update_from_room_members(&members);
                self.set_state(LoadingState::Ready);
            }
            Err(error) => {
                self.set_state(LoadingState::Error);
                error!(%error, "Failed to load room members from server");
            }
        }
    }

    /// Updates members with the given RoomMember values.
    ///
    /// If some of the values do not correspond to existing members, new members
    /// are created.
    fn update_from_room_members(&self, new_members: &[matrix_sdk::room::RoomMember]) {
        let imp = self.imp();
        let mut members = imp.members.borrow_mut();
        let prev_len = members.len();
        for member in new_members {
            if let Entry::Vacant(entry) = members.entry(member.user_id().into()) {
                entry.insert(Member::new(&self.room(), member.user_id()));
            }
        }
        let num_members_added = members.len().saturating_sub(prev_len);

        // We can't have the mut borrow active when members are updated or items_changed
        // is emitted because that will probably cause reads of the members
        // field.
        std::mem::drop(members);

        {
            for room_member in new_members {
                let member = imp.members.borrow().get(room_member.user_id()).cloned();
                if let Some(member) = member {
                    member.update_from_room_member(room_member);
                }
            }

            // Restore the members activity according to the known timeline events.
            for item in self.room().timeline().items().iter::<glib::Object>().rev() {
                let Ok(item) = item else {
                    // The iterator is broken, stop.
                    break;
                };
                let Ok(event) = item.downcast::<Event>() else {
                    continue;
                };
                if !event.counts_as_unread() {
                    continue;
                }

                let member = imp.members.borrow().get(&event.sender_id()).cloned();
                if let Some(member) = member {
                    member.set_latest_activity(event.origin_server_ts_u64());
                }
            }
        }

        if num_members_added > 0 {
            // IndexMap preserves insertion order, so all the new items will be at the end.
            self.items_changed(prev_len as u32, 0, num_members_added as u32);
        }
    }

    /// Returns the member with the given ID.
    ///
    /// Creates a new member first if there is no member with the given ID.
    pub fn get_or_create(&self, user_id: OwnedUserId) -> Member {
        let mut members = self.imp().members.borrow_mut();
        let mut was_member_added = false;
        let prev_len = members.len();
        let member = members
            .entry(user_id)
            .or_insert_with_key(|user_id| {
                was_member_added = true;
                Member::new(&self.room(), user_id)
            })
            .clone();

        // We can't have the borrow active when items_changed is emitted because that
        // will probably cause reads of the members field.
        std::mem::drop(members);
        if was_member_added {
            // IndexMap preserves insertion order so the new member will be at the end.
            self.items_changed(prev_len as u32, 0, 1);
        }

        member
    }

    /// Updates a room member based on the room member state event.
    ///
    /// Creates a new member first if there is no member matching the given
    /// event.
    pub(super) fn update_member_for_member_event(
        &self,
        event: &OriginalSyncStateEvent<RoomMemberEventContent>,
    ) {
        self.get_or_create(event.state_key.to_owned())
            .update_from_member_event(event);
    }

    /// Returns the Membership of a given UserId.
    ///
    /// If the user has no Membership, Membership::Leave will be returned
    pub fn get_membership(&self, user_id: &UserId) -> Membership {
        self.imp()
            .members
            .borrow()
            .get(user_id)
            .map_or_else(|| Membership::Leave, |member| member.membership())
    }
}

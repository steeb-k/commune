use gtk::{
    gio,
    glib::{self, clone},
    prelude::*,
    subclass::prelude::*,
};
use indexmap::{map::Entry, IndexMap};
use matrix_sdk::RoomMemberships;
use ruma::{events::room::power_levels::RoomPowerLevels, OwnedUserId, UserId};
use tracing::error;

use super::{Event, Member, Membership, Room};
use crate::{prelude::*, spawn, spawn_tokio, utils::LoadingState};

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::MemberList)]
    pub struct MemberList {
        /// The list of known members.
        pub members: RefCell<IndexMap<OwnedUserId, Member>>,
        /// The room these members belong to.
        #[property(get, set = Self::set_room, construct_only)]
        pub room: glib::WeakRef<Room>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        pub state: Cell<LoadingState>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberList {
        const NAME: &'static str = "MemberList";
        type Type = super::MemberList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for MemberList {}

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

    impl MemberList {
        /// Set the room these members belong to.
        fn set_room(&self, room: Room) {
            let obj = self.obj();

            let own_member = room.own_member();
            self.members
                .borrow_mut()
                .insert(own_member.user_id().clone(), own_member);

            if let Some(member) = room.direct_member() {
                self.members
                    .borrow_mut()
                    .insert(member.user_id().clone(), member);
            }

            self.room.set(Some(&room));
            obj.notify_room();

            spawn!(
                glib::Priority::LOW,
                clone!(@weak obj => async move {
                    obj.load().await;
                })
            );
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
        glib::Object::builder::<Self>()
            .property("room", room)
            .build()
    }

    /// Set whether this list is being loaded.
    fn set_state(&self, state: LoadingState) {
        if self.state() == state {
            return;
        }

        self.imp().state.set(state);
        self.notify_state();
    }

    pub fn reload(&self) {
        self.set_state(LoadingState::Initial);

        spawn!(clone!(@weak self as obj => async move {
            obj.load().await;
        }));
    }

    /// Load this list.
    async fn load(&self) {
        let Some(room) = self.room() else {
            return;
        };
        if matches!(self.state(), LoadingState::Loading | LoadingState::Ready) {
            return;
        }

        self.set_state(LoadingState::Loading);

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
                error!("Could not load room members from store: {error}");
            }
        }

        // We don't have everything locally, request the rest from the server.
        let matrix_room = matrix_room.clone();
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
                error!(%error, "Could not load room members from server");
            }
        }
    }

    /// Updates members with the given RoomMember values.
    ///
    /// If some of the values do not correspond to existing members, new members
    /// are created.
    fn update_from_room_members(&self, new_members: &[matrix_sdk::room::RoomMember]) {
        let Some(room) = self.room() else {
            return;
        };
        let imp = self.imp();
        let mut members = imp.members.borrow_mut();
        let prev_len = members.len();
        for member in new_members {
            if let Entry::Vacant(entry) = members.entry(member.user_id().into()) {
                entry.insert(Member::new(&room, member.user_id().to_owned()));
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
            for item in room.timeline().items().iter::<glib::Object>().rev() {
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

    /// Returns the member with the given ID, if it exists in the list.
    pub fn get(&self, user_id: &UserId) -> Option<Member> {
        self.imp().members.borrow().get(user_id).cloned()
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
                Member::new(&self.room().unwrap(), user_id.clone())
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

    /// Update a room member with the SDK's data.
    ///
    /// Creates a new member first if there is no member matching the given
    /// event.
    pub(super) fn update_member(&self, user_id: OwnedUserId) {
        self.get_or_create(user_id).update();
    }

    /// Updates the room members' power level.
    pub(super) fn update_power_levels(&self, power_levels: &RoomPowerLevels) {
        // We need to go through the whole list because we don't know who was
        // added/removed.
        for (user_id, member) in &*self.imp().members.borrow() {
            member.set_power_level(power_levels.for_user(user_id).into());
        }
    }

    /// Returns the Membership of a given UserId.
    ///
    /// If the user has no Membership, Membership::Leave will be returned
    pub fn get_membership(&self, user_id: &UserId) -> Membership {
        self.imp()
            .members
            .borrow()
            .get(user_id)
            .map_or(Membership::Leave, |member| member.membership())
    }
}

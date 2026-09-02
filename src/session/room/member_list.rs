use std::collections::HashMap;

use commune_core::{
    VectorDiff,
    session::{Member as CoreMember, MemberList as CoreMemberList},
};
use gtk::{gio, glib, glib::closure, prelude::*, subclass::prelude::*};
use indexmap::IndexMap;
use ruma::{OwnedUserId, UserId};
use tokio::task::AbortHandle;

use super::{Event, Member, Membership, Room};
use crate::{
    core_bridge::{ObjectWatcher, list_model::apply_diff},
    prelude::*,
    utils::LoadingState,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::MemberList)]
    pub struct MemberList {
        /// The known members, in the core's order.
        ///
        /// Also the wrapper cache: the same `Member` for the same user
        /// across every diff.
        pub(super) members: RefCell<IndexMap<OwnedUserId, Member>>,
        /// The room these members belong to.
        #[property(get, set = Self::set_room, construct_only)]
        room: glib::WeakRef<Room>,
        /// The lists of members filtered by membership.
        membership_lists: RefCell<HashMap<MembershipListKind, gio::ListModel>>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        state: Cell<LoadingState>,
        /// The task following the core's list.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberList {
        const NAME: &'static str = "MemberList";
        type Type = super::MemberList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for MemberList {
        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
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
            self.members
                .borrow()
                .get_index(position as usize)
                .map(|(_user_id, member)| member.clone().upcast())
        }
    }

    impl MemberList {
        /// Set the room these members belong to, and follow its core list.
        fn set_room(&self, room: &Room) {
            self.room.set(Some(room));
            self.obj().notify_room();

            let core = room.core().member_list();
            let (members, stream) = core.subscribe();

            // The core's list holds our own member from the start; the
            // room's own object stands in for it, as before.
            let wrapped = members
                .into_iter()
                .map(|member| self.wrap(&member))
                .collect::<Vec<_>>();
            let added = wrapped.len();
            self.members.borrow_mut().extend(wrapped);
            self.restore_latest_activity();
            if added > 0 {
                self.obj().items_changed(0, 0, added as u32);
            }

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(stream, |obj: &super::MemberList, diffs| {
                    obj.imp().apply_diffs(diffs);
                })
                .follow(core.subscribe_state(), |obj: &super::MemberList, state| {
                    obj.imp().set_state(state.into());
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            self.set_state(core.state().into());
        }

        /// The core's list.
        pub(super) fn core(&self) -> Option<CoreMemberList> {
            self.room.upgrade().map(|room| room.core().member_list())
        }

        /// The wrapper for the given core member, brought up to date with
        /// it: the one this list has, the room's own or direct member, or
        /// a new one.
        fn wrap(&self, member: &CoreMember) -> (OwnedUserId, Member) {
            let user_id = member.user_id.clone();

            let existing = self.members.borrow().get(&user_id).cloned();
            let wrapper = existing.unwrap_or_else(|| {
                let room = self.room.upgrade().expect("a member list outlives no room");

                if *room.own_member().user_id() == user_id {
                    room.own_member()
                } else if let Some(direct) = room
                    .direct_member()
                    .filter(|direct| *direct.user_id() == user_id)
                {
                    direct
                } else {
                    Member::new(&room, user_id.clone())
                }
            });

            wrapper.update_from_snapshot(member);
            (user_id, wrapper)
        }

        /// Apply a batch of changes from the core's list.
        fn apply_diffs(&self, diffs: Vec<VectorDiff<CoreMember>>) {
            let mut added = 0;

            for diff in diffs {
                // Wrap first, outside the borrow: a wrapper's constructor
                // may look the list up.
                let diff = diff.map(|member| self.wrap(&member));
                let changes = apply_diff(&mut self.members.borrow_mut(), diff);

                let obj = self.obj();
                for change in changes {
                    obj.items_changed(change.position, change.removed, change.added);
                    added += change.added;
                }
            }

            if added > 0 {
                self.restore_latest_activity();
            }
        }

        /// Restore the members' activity according to the known live
        /// timeline events.
        fn restore_latest_activity(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            for item in room.live_timeline().items().iter::<glib::Object>().rev() {
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

                let member = self.members.borrow().get(&event.sender_id()).cloned();
                if let Some(member) = member {
                    member.set_latest_activity(u64::from(event.origin_server_ts().get()));
                }
            }
        }

        /// Get the list filtered by membership for the given kind.
        pub(super) fn membership_list(&self, kind: MembershipListKind) -> gio::ListModel {
            if let Some(list) = self.membership_lists.borrow().get(&kind) {
                return list.clone();
            }

            // Construct the list if it doesn't exist.
            let list = kind.filtered_list_model(self.obj().upcast_ref());
            self.membership_lists
                .borrow_mut()
                .insert(kind, list.clone());
            list
        }

        /// Set the loading state of the list, from the core.
        pub(super) fn set_state(&self, state: LoadingState) {
            if self.state.get() == state {
                return;
            }

            self.state.set(state);
            self.obj().notify_state();
        }
    }
}

glib::wrapper! {
    /// List of all Members in a room. Implements ListModel.
    ///
    /// Members are sorted in "insertion order", not anything useful.
    ///
    /// The list itself is the core's; this presents it, one `Member` per
    /// core member, in the core's order.
    pub struct MemberList(ObjectSubclass<imp::MemberList>)
        @implements gio::ListModel;
}

impl MemberList {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder::<Self>()
            .property("room", room)
            .build()
    }

    /// Reload this list.
    pub(crate) fn reload(&self) {
        if let Some(core) = self.imp().core() {
            core.reload();
        }
    }

    /// Returns the member with the given ID, if it exists in the list.
    pub(crate) fn get(&self, user_id: &UserId) -> Option<Member> {
        self.imp().members.borrow().get(user_id).cloned()
    }

    /// Returns the member with the given ID.
    ///
    /// Creates a new member first if there is no member with the given ID:
    /// the core's list gets a placeholder it fills in from the store, and
    /// this list gets the wrapper at the same index right away.
    pub(crate) fn get_or_create(&self, user_id: OwnedUserId) -> Member {
        if let Some(member) = self.get(&user_id) {
            return member;
        }

        let imp = self.imp();
        let room = self.room().expect("room exists");
        let member = Member::new(&room, user_id.clone());

        let index = imp
            .core()
            .map_or_else(|| imp.members.borrow().len(), |core| core.ensure(&user_id));

        let position = {
            let mut members = imp.members.borrow_mut();
            let position = index.min(members.len());
            members.shift_insert(position, user_id, member.clone());
            position
        };

        // We can't have the borrow active when items_changed is emitted because that
        // will probably cause reads of the members field.
        self.items_changed(position as u32, 0, 1);

        member
    }

    /// Get the list filtered by membership for the given kind.
    pub(crate) fn membership_list(&self, kind: MembershipListKind) -> gio::ListModel {
        self.imp().membership_list(kind)
    }
}

/// The kind of membership used to filter a list of room members.
///
/// This is a subset of [`Membership`].
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum, glib::Variant)]
#[enum_type(name = "MembershipListKind")]
pub enum MembershipListKind {
    /// The user is currently in the room.
    #[default]
    Join,
    /// The user was invited to the room.
    Invite,
    /// The user was banned from the room.
    Ban,
    /// The user knocked on the room.
    Knock,
}

impl MembershipListKind {
    /// Build a `GListModel` that filters the given list model containing
    /// [`Member`]s with this kind, and add it to the given map.
    fn filtered_list_model(self, members: &gio::ListModel) -> gio::ListModel {
        let membership = Membership::from(self);
        let membership_eq_expr = Member::this_expression("membership").chain_closure::<bool>(
            closure!(|_: Option<glib::Object>, this_membership: Membership| {
                this_membership == membership
            }),
        );

        gtk::FilterListModel::builder()
            .model(members)
            .filter(&gtk::BoolFilter::new(Some(&membership_eq_expr)))
            .watch_items(true)
            .build()
            .upcast()
    }

    /// The tag to use for pages that present this kind.
    pub(crate) const fn tag(self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::Invite => "invite",
            Self::Ban => "ban",
            Self::Knock => "knock",
        }
    }

    /// The name of the icon that represents this kind.
    pub(crate) const fn icon_name(self) -> &'static str {
        match self {
            Self::Join | Self::Knock => "users-symbolic",
            Self::Invite => "user-add-symbolic",
            Self::Ban => "safety-symbolic",
        }
    }
}

impl From<MembershipListKind> for Membership {
    fn from(value: MembershipListKind) -> Self {
        match value {
            MembershipListKind::Join => Self::Join,
            MembershipListKind::Invite => Self::Invite,
            MembershipListKind::Ban => Self::Ban,
            MembershipListKind::Knock => Self::Knock,
        }
    }
}

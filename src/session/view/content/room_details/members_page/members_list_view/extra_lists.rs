use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use super::MembershipSubpageItem;
use crate::{
    components::LoadingRow,
    session::model::MemberList,
    utils::{BoundObjectWeakRef, LoadingState},
};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ExtraLists)]
    pub struct ExtraLists {
        /// The list of all members.
        #[property(get, set = Self::set_members, construct_only)]
        pub members: BoundObjectWeakRef<MemberList>,
        pub state: RefCell<Option<LoadingRow>>,
        /// The item for the list of invited members.
        #[property(get, construct_only)]
        pub invited: OnceCell<MembershipSubpageItem>,
        /// The item for the list of banned members.
        #[property(get, construct_only)]
        pub banned: OnceCell<MembershipSubpageItem>,
        pub invited_is_empty: Cell<bool>,
        pub banned_is_empty: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ExtraLists {
        const NAME: &'static str = "ContentMembersExtraLists";
        type Type = super::ExtraLists;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for ExtraLists {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let invited_members = obj.invited().model();
            let banned_members = obj.banned().model();

            invited_members.connect_items_changed(clone!(
                #[weak]
                obj,
                move |_, _, _, _| {
                    obj.update_invited();
                }
            ));

            banned_members.connect_items_changed(clone!(
                #[weak]
                obj,
                move |_, _, _, _| {
                    obj.update_banned();
                }
            ));

            self.invited_is_empty.set(invited_members.n_items() == 0);
            self.banned_is_empty.set(banned_members.n_items() == 0);
        }
    }

    impl ListModelImpl for ExtraLists {
        fn item_type(&self) -> glib::Type {
            glib::Object::static_type()
        }

        fn n_items(&self) -> u32 {
            self.state.borrow().is_some() as u32
                + (!self.invited_is_empty.get()) as u32
                + (!self.banned_is_empty.get()) as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let mut position = position;

            if let Some(state_row) = &*self.state.borrow() {
                if position == 0 {
                    return Some(state_row.clone().upcast());
                }

                position -= 1;
            }

            if !self.invited_is_empty.get() {
                if position == 0 {
                    return self.invited.get().cloned().and_upcast();
                }

                position -= 1;
            }

            if !self.banned_is_empty.get() && position == 0 {
                return self.banned.get().cloned().and_upcast();
            }

            None
        }
    }

    impl ExtraLists {
        /// Set the list of all members.
        fn set_members(&self, members: MemberList) {
            let obj = self.obj();

            self.members.disconnect_signals();

            let signal_handler_ids = vec![members.connect_state_notify(clone!(
                #[weak]
                obj,
                move |members| {
                    obj.update_loading_state(members.state());
                }
            ))];
            obj.update_loading_state(members.state());

            self.members.set(&members, signal_handler_ids);
            obj.notify_members();
        }
    }
}

glib::wrapper! {
    /// The list of extra items in the list of room members.
    pub struct ExtraLists(ObjectSubclass<imp::ExtraLists>)
        @implements gio::ListModel;
}

impl ExtraLists {
    pub fn new(
        members: &MemberList,
        invited: &MembershipSubpageItem,
        banned: &MembershipSubpageItem,
    ) -> Self {
        glib::Object::builder()
            .property("members", members)
            .property("invited", invited)
            .property("banned", banned)
            .build()
    }

    /// Update this list for the given loading state.
    fn update_loading_state(&self, state: LoadingState) {
        let imp = self.imp();

        if state == LoadingState::Ready {
            if imp.state.take().is_some() {
                self.items_changed(0, 1, 0);
            }

            return;
        }

        let mut added = false;
        {
            let mut state_row_borrow = imp.state.borrow_mut();
            let state_row = state_row_borrow.get_or_insert_with(|| {
                added = true;
                LoadingRow::new()
            });

            let error = (state == LoadingState::Error)
                .then(|| gettext("Could not load the full list of room members"));

            state_row.set_error(error.as_deref());
        }

        if added {
            self.items_changed(0, 0, 1);
        }
    }

    fn update_invited(&self) {
        let imp = self.imp();

        let was_empty = imp.invited_is_empty.get();
        let is_empty = self.invited().model().n_items() == 0;

        if was_empty == is_empty {
            // Nothing changed so don't do anything
            return;
        }

        imp.invited_is_empty.set(is_empty);
        let position = imp.state.borrow().is_some() as u32;

        // If it is not added, it is removed.
        let added = was_empty as u32;
        let removed = (!was_empty) as u32;

        self.items_changed(position, removed, added);
    }

    fn update_banned(&self) {
        let imp = self.imp();

        let was_empty = imp.banned_is_empty.get();
        let is_empty = self.banned().model().n_items() == 0;

        if was_empty == is_empty {
            // Nothing changed so don't do anything
            return;
        }

        imp.banned_is_empty.set(is_empty);

        let position = imp.state.borrow().is_some() as u32 + (!imp.invited_is_empty.get()) as u32;

        // If it is not added, it is removed.
        let added = was_empty as u32;
        let removed = (!was_empty) as u32;

        self.items_changed(position, removed, added);
    }
}

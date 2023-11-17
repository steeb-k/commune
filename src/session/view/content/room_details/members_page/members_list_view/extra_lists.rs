use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use super::MembershipSubpageItem;
use crate::{
    components::{LoadingRow, LoadingState},
    session::model::MemberList,
    utils::BoundObjectWeakRef,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use once_cell::{sync::Lazy, unsync::OnceCell};

    use super::*;

    #[derive(Debug, Default)]
    pub struct ExtraLists {
        pub members: BoundObjectWeakRef<MemberList>,
        pub state: RefCell<Option<LoadingRow>>,
        pub invited: OnceCell<MembershipSubpageItem>,
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

    impl ObjectImpl for ExtraLists {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<MemberList>("members")
                        .construct_only()
                        .build(),
                    glib::ParamSpecObject::builder::<MembershipSubpageItem>("invited")
                        .construct_only()
                        .build(),
                    glib::ParamSpecObject::builder::<MembershipSubpageItem>("banned")
                        .construct_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "members" => obj.set_members(value.get::<Option<MemberList>>().unwrap().as_ref()),
                "invited" => obj.set_invited(value.get().unwrap()),
                "banned" => obj.set_banned(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "members" => obj.members().to_value(),
                "invited" => obj.invited().to_value(),
                "banned" => obj.banned().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let invited_members = obj.invited().model();
            let banned_members = obj.banned().model();

            invited_members.connect_items_changed(clone!(@weak obj => move |_, _, _, _| {
                obj.update_invited();
            }));

            banned_members.connect_items_changed(clone!(@weak obj => move |_, _, _, _| {
                obj.update_banned();
            }));

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
}

glib::wrapper! {
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

    pub fn members(&self) -> Option<MemberList> {
        self.imp().members.obj()
    }

    fn set_members(&self, members: Option<&MemberList>) {
        let Some(members) = members else {
            // Ignore if there is no list.
            return;
        };

        let imp = self.imp();
        imp.members.disconnect_signals();

        let signal_handler_ids = vec![members.connect_notify_local(
            Some("state"),
            clone!(@weak self as obj => move |members, _| {
                obj.update_loading_state(members.state());
            }),
        )];
        self.update_loading_state(members.state());

        imp.members.set(members, signal_handler_ids);
        self.notify("members");
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

    /// The subpage item for invited members.
    pub fn invited(&self) -> &MembershipSubpageItem {
        self.imp().invited.get().unwrap()
    }

    /// Set the subpage item for invited members.
    fn set_invited(&self, item: MembershipSubpageItem) {
        self.imp().invited.set(item).unwrap();
    }

    /// The subpage for banned members.
    pub fn banned(&self) -> &MembershipSubpageItem {
        self.imp().banned.get().unwrap()
    }

    /// Set the subpage for banned members.
    fn set_banned(&self, item: MembershipSubpageItem) {
        self.imp().banned.set(item).unwrap();
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

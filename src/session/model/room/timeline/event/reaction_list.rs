use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use matrix_sdk_ui::timeline::ReactionsByKeyBySender;

use super::ReactionGroup;
use crate::session::model::User;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use indexmap::IndexMap;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ReactionList)]
    pub struct ReactionList {
        /// The user of the parent session.
        #[property(get, set)]
        user: OnceCell<User>,
        /// The list of reactions grouped by key.
        reactions: RefCell<IndexMap<String, ReactionGroup>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReactionList {
        const NAME: &'static str = "ReactionList";
        type Type = super::ReactionList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for ReactionList {}

    impl ListModelImpl for ReactionList {
        fn item_type(&self) -> glib::Type {
            ReactionGroup::static_type()
        }

        fn n_items(&self) -> u32 {
            self.reactions.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let reactions = self.reactions.borrow();

            reactions
                .get_index(position as usize)
                .map(|(_key, reaction_group)| reaction_group.clone().upcast())
        }
    }

    impl ReactionList {
        /// Update the reaction list with the given reactions.
        pub(super) fn update(&self, new_reactions: &ReactionsByKeyBySender) {
            let changed = {
                let old_reactions = self.reactions.borrow();

                old_reactions.len() != new_reactions.len()
                    || new_reactions
                        .keys()
                        .zip(old_reactions.keys())
                        .any(|(new_key, old_key)| new_key != old_key)
            };

            if changed {
                let mut reactions = self.reactions.borrow_mut();
                let user = self.user.get().expect("user is initialized");
                let prev_len = reactions.len();
                let new_len = new_reactions.len();

                *reactions = new_reactions
                    .iter()
                    .map(|(key, reactions)| {
                        let group = ReactionGroup::new(key, user);
                        group.update(reactions);
                        (key.clone(), group)
                    })
                    .collect();

                // We cannot have the borrow active when items_changed is emitted because that
                // will probably cause reads of the reactions field.
                std::mem::drop(reactions);

                self.obj().items_changed(0, prev_len as u32, new_len as u32);
            } else {
                let reactions = self.reactions.borrow();
                for (reactions, group) in new_reactions.values().zip(reactions.values()) {
                    group.update(reactions);
                }
            }
        }

        /// Get a reaction group by its key.
        ///
        /// Returns `None` if no action group was found with this key.
        pub(super) fn reaction_group_by_key(&self, key: &str) -> Option<ReactionGroup> {
            self.reactions.borrow().get(key).cloned()
        }
    }
}

glib::wrapper! {
    /// List of all `ReactionGroup`s for an event.
    ///
    /// Implements `GListModel`. `ReactionGroup`s are sorted in "insertion order".
    pub struct ReactionList(ObjectSubclass<imp::ReactionList>)
        @implements gio::ListModel;
}

impl ReactionList {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Update the reaction list with the given reactions.
    pub(crate) fn update(&self, new_reactions: &ReactionsByKeyBySender) {
        self.imp().update(new_reactions);
    }

    /// Get a reaction group by its key.
    ///
    /// Returns `None` if no action group was found with this key.
    pub(crate) fn reaction_group_by_key(&self, key: &str) -> Option<ReactionGroup> {
        self.imp().reaction_group_by_key(key)
    }
}

impl Default for ReactionList {
    fn default() -> Self {
        Self::new()
    }
}

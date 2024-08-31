use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use indexmap::IndexMap;
use matrix_sdk_ui::timeline::ReactionInfo;
use ruma::{MilliSecondsSinceUnixEpoch, OwnedUserId};

use crate::{prelude::*, session::model::User};

/// A map of user ID to reaction info.
type ReactionsMap = IndexMap<OwnedUserId, ReactionInfo>;

/// Data of a reaction in a reaction group.
#[derive(Clone, Debug)]
pub struct ReactionData {
    /// The sender of the reaction.
    pub sender_id: OwnedUserId,
    /// The timestamp of the reaction.
    pub timestamp: MilliSecondsSinceUnixEpoch,
}

mod imp {
    use std::{
        cell::{OnceCell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ReactionGroup)]
    pub struct ReactionGroup {
        /// The user of the parent session.
        #[property(get, construct_only)]
        pub user: OnceCell<User>,
        /// The key of the group.
        #[property(get, construct_only)]
        pub key: OnceCell<String>,
        /// The reactions in the group.
        pub reactions: RefCell<Option<ReactionsMap>>,
        /// The number of reactions in this group.
        #[property(get = Self::count)]
        pub count: PhantomData<u32>,
        /// Whether this group has a reaction from our own user.
        #[property(get = Self::has_user)]
        pub has_user: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReactionGroup {
        const NAME: &'static str = "ReactionGroup";
        type Type = super::ReactionGroup;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for ReactionGroup {}

    impl ListModelImpl for ReactionGroup {
        fn item_type(&self) -> glib::Type {
            glib::BoxedAnyObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.reactions
                .borrow()
                .as_ref()
                .map(|reactions| reactions.len())
                .unwrap_or_default() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.reactions
                .borrow()
                .as_ref()
                .and_then(|reactions| reactions.get_index(position as usize))
                .map(|(user_id, info)| {
                    glib::BoxedAnyObject::new(ReactionData {
                        sender_id: user_id.clone(),
                        timestamp: info.timestamp,
                    })
                    .upcast()
                })
        }
    }

    impl ReactionGroup {
        /// The number of reactions in this group.
        fn count(&self) -> u32 {
            self.n_items()
        }

        /// Whether this group has a reaction from our own user.
        fn has_user(&self) -> bool {
            let user_id = self.user.get().unwrap().user_id();
            self.reactions
                .borrow()
                .as_ref()
                .is_some_and(|reactions| reactions.contains_key(user_id))
        }
    }
}

glib::wrapper! {
    /// Reactions grouped by a given key. Implements `ListModel`.
    pub struct ReactionGroup(ObjectSubclass<imp::ReactionGroup>)
        @implements gio::ListModel;
}

impl ReactionGroup {
    pub fn new(key: &str, user: &User) -> Self {
        glib::Object::builder()
            .property("key", key)
            .property("user", user)
            .build()
    }

    /// Update this group with the given reactions.
    pub fn update(&self, new_reactions: &ReactionsMap) {
        let prev_has_user = self.has_user();
        let prev_count = self.count();
        let new_count = new_reactions.len() as u32;
        let reactions = &self.imp().reactions;

        let same_reactions = match reactions.borrow().as_ref() {
            Some(old_reactions) => {
                prev_count == new_count
                    && new_reactions.iter().zip(old_reactions.iter()).all(
                        |((old_sender_id, old_info), (new_sender_id, new_info))| {
                            old_sender_id == new_sender_id
                                && old_info.timestamp == new_info.timestamp
                        },
                    )
            }
            // There were no reactions before, now there are, so it is definitely not the same.
            None => false,
        };
        if same_reactions {
            return;
        }

        *reactions.borrow_mut() = Some(new_reactions.clone());

        self.items_changed(0, prev_count, new_count);

        if self.count() != prev_count {
            self.notify_count();
        }

        if self.has_user() != prev_has_user {
            self.notify_has_user();
        }
    }
}

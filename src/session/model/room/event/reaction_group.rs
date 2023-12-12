use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use matrix_sdk_ui::timeline::ReactionGroup as SdkReactionGroup;

use super::EventKey;
use crate::{prelude::*, session::model::User};

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
        pub reactions: RefCell<Option<SdkReactionGroup>>,
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
                .map(|reactions| reactions.senders().count())
                .unwrap_or_default() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.reactions.borrow().as_ref().and_then(|reactions| {
                reactions
                    .senders()
                    .nth(position as usize)
                    .map(|sd| glib::BoxedAnyObject::new(sd.clone()).upcast())
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
            let user_id = UserExt::user_id(self.user.get().unwrap());
            self.reactions
                .borrow()
                .as_ref()
                .filter(|reactions| reactions.by_sender(&user_id).next().is_some())
                .is_some()
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

    /// The event ID of the reaction in this group sent by the logged-in user,
    /// if any.
    pub fn user_reaction_event_key(&self) -> Option<EventKey> {
        let user_id = UserExt::user_id(&self.user());
        self.imp()
            .reactions
            .borrow()
            .as_ref()
            .and_then(|reactions| {
                reactions
                    .by_sender(&user_id)
                    .next()
                    .and_then(|timeline_key| match timeline_key {
                        (Some(txn_id), None) => Some(EventKey::TransactionId(txn_id.clone())),
                        (_, Some(event_id)) => Some(EventKey::EventId(event_id.clone())),
                        _ => None,
                    })
            })
    }

    /// Update this group with the given reactions.
    pub fn update(&self, new_reactions: SdkReactionGroup) {
        let prev_has_user = self.has_user();
        let prev_count = self.count();
        let new_count = new_reactions.senders().count() as u32;
        let reactions = &self.imp().reactions;

        let same_reactions = match reactions.borrow().as_ref() {
            Some(old_reactions) => {
                prev_count == new_count
                    && new_reactions.senders().zip(old_reactions.senders()).all(
                        |(old_sender, new_sender)| {
                            old_sender.sender_id == new_sender.sender_id
                                && old_sender.timestamp == new_sender.timestamp
                        },
                    )
            }
            // There were no reactions before, now there are, so it is definitely not the same.
            None => false,
        };
        if same_reactions {
            return;
        }

        *reactions.borrow_mut() = Some(new_reactions);

        self.items_changed(0, prev_count, new_count);

        if self.count() != prev_count {
            self.notify_count();
        }

        if self.has_user() != prev_has_user {
            self.notify_has_user();
        }
    }
}

use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::{MxcUri, UserId};

use crate::{
    prelude::*,
    session::model::{Session, User},
};

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Invitee)]
    pub struct Invitee {
        /// Whether this user is invited.
        #[property(get, set = Self::set_invited, explicit_notify)]
        pub invited: Cell<bool>,
        /// The anchor for this user in the text buffer.
        #[property(get, set = Self::set_anchor, explicit_notify, nullable)]
        pub anchor: RefCell<Option<gtk::TextChildAnchor>>,
        /// The reason the user can't be invited.
        #[property(get, set = Self::set_invite_exception, explicit_notify, nullable)]
        pub invite_exception: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Invitee {
        const NAME: &'static str = "Invitee";
        type Type = super::Invitee;
        type ParentType = User;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Invitee {}

    impl Invitee {
        /// Set whether this user is invited.
        fn set_invited(&self, invited: bool) {
            if self.invited.get() == invited {
                return;
            }

            self.invited.set(invited);
            self.obj().notify_invited();
        }

        /// Set the anchor for this user in the text buffer.
        fn set_anchor(&self, anchor: Option<gtk::TextChildAnchor>) {
            if *self.anchor.borrow() == anchor {
                return;
            }

            self.anchor.replace(anchor);
            self.obj().notify_anchor();
        }

        /// Set the reason the user can't be invited.
        fn set_invite_exception(&self, exception: Option<String>) {
            if exception == *self.invite_exception.borrow() {
                return;
            }

            self.invite_exception.replace(exception);
            self.obj().notify_invite_exception();
        }
    }
}

glib::wrapper! {
    /// A possible invitee.
    pub struct Invitee(ObjectSubclass<imp::Invitee>) @extends User;
}

impl Invitee {
    pub fn new(
        session: &Session,
        user_id: &UserId,
        display_name: Option<&str>,
        avatar_url: Option<&MxcUri>,
    ) -> Self {
        let obj: Self = glib::Object::builder()
            .property("session", session)
            .property("user-id", user_id.as_str())
            .property("display-name", display_name)
            .build();
        // FIXME: we should make the avatar_url settable as property
        obj.set_avatar_url(avatar_url.map(std::borrow::ToOwned::to_owned));
        obj
    }

    /// Take the anchor for this user in the text buffer.
    ///
    /// The anchor will be `None` after calling this method.
    pub fn take_anchor(&self) -> Option<gtk::TextChildAnchor> {
        let anchor = self.imp().anchor.take();
        self.notify_anchor();
        anchor
    }
}

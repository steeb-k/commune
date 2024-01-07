use gtk::{glib, prelude::*, subclass::prelude::*};

use super::UserSession;

/// This enum contains all possible types the user sessions list can hold.
#[derive(Debug, Clone, glib::Boxed)]
#[boxed_type(name = "UserSessionsListItemType")]
pub enum UserSessionsListItemType {
    UserSession(UserSession),
    Error(String),
    LoadingSpinner,
}

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::UserSessionsListItem)]
    pub struct UserSessionsListItem {
        /// The type of this item.
        #[property(get, construct_only)]
        pub item_type: OnceCell<UserSessionsListItemType>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserSessionsListItem {
        const NAME: &'static str = "UserSessionsListItem";
        type Type = super::UserSessionsListItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for UserSessionsListItem {}
}

glib::wrapper! {
    /// An item in the user sessions list.
    pub struct UserSessionsListItem(ObjectSubclass<imp::UserSessionsListItem>);
}

impl UserSessionsListItem {
    pub fn for_user_session(user_session: UserSession) -> Self {
        let item_type = UserSessionsListItemType::UserSession(user_session);
        glib::Object::builder()
            .property("item-type", &item_type)
            .build()
    }

    pub fn for_error(error: String) -> Self {
        let item_type = UserSessionsListItemType::Error(error);
        glib::Object::builder()
            .property("item-type", &item_type)
            .build()
    }

    pub fn for_loading_spinner() -> Self {
        let item_type = UserSessionsListItemType::LoadingSpinner;
        glib::Object::builder()
            .property("item-type", &item_type)
            .build()
    }
}

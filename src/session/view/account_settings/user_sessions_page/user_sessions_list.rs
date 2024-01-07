use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    encryption::identities::UserDevices as CryptoDevices,
    ruma::api::client::device::Device as MatrixDevice, Error,
};
use tracing::error;

use super::{UserSession, UserSessionsListItem};
use crate::{session::model::Session, spawn, spawn_tokio};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::UserSessionsList)]
    pub struct UserSessionsList {
        /// The list of user session list items.
        pub list: RefCell<Vec<UserSessionsListItem>>,
        /// The current session.
        #[property(get, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// The current user session.
        pub current_user_session_inner: RefCell<Option<UserSessionsListItem>>,
        /// The current user session, or a replacement list item if it is not
        /// found.
        #[property(get = Self::current_user_session)]
        current_user_session: PhantomData<UserSessionsListItem>,
        pub loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserSessionsList {
        const NAME: &'static str = "UserSessionsList";
        type Type = super::UserSessionsList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for UserSessionsList {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().load();
        }
    }

    impl ListModelImpl for UserSessionsList {
        fn item_type(&self) -> glib::Type {
            UserSessionsListItem::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get(position as usize)
                .map(glib::object::Cast::upcast_ref::<glib::Object>)
                .cloned()
        }
    }

    impl UserSessionsList {
        /// The current user session.
        fn current_user_session(&self) -> UserSessionsListItem {
            self.current_user_session_inner
                .borrow()
                .clone()
                .unwrap_or_else(|| {
                    if self.loading.get() {
                        UserSessionsListItem::for_loading_spinner()
                    } else {
                        UserSessionsListItem::for_error(gettext("Failed to load connected device."))
                    }
                })
        }
    }
}

glib::wrapper! {
    /// List of active user sessions for the logged-in user.
    pub struct UserSessionsList(ObjectSubclass<imp::UserSessionsList>)
        @implements gio::ListModel;
}

impl UserSessionsList {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    fn set_loading(&self, loading: bool) {
        let imp = self.imp();

        if loading == imp.loading.get() {
            return;
        }
        if loading {
            self.update_list(vec![UserSessionsListItem::for_loading_spinner()]);
        }
        imp.loading.set(loading);
        self.notify_current_user_session();
    }

    /// Set the current user session.
    fn set_current_user_session(&self, user_session: Option<UserSessionsListItem>) {
        self.imp().current_user_session_inner.replace(user_session);

        self.notify_current_user_session();
    }

    /// Update the list with the given user sessions.
    fn update_list(&self, user_sessions: Vec<UserSessionsListItem>) {
        let added = user_sessions.len();

        let prev_user_sessions = self.imp().list.replace(user_sessions);

        self.items_changed(0, prev_user_sessions.len() as u32, added as u32);
    }

    /// Process the user sessions received in the response.
    fn finish_loading(
        &self,
        response: Result<(Option<MatrixDevice>, Vec<MatrixDevice>, CryptoDevices), Error>,
    ) {
        let Some(session) = self.session() else {
            return;
        };

        match response {
            Ok((current_user_session, user_sessions, crypto_sessions)) => {
                let user_sessions = user_sessions
                    .into_iter()
                    .map(|user_session| {
                        let crypto_session = crypto_sessions.get(&user_session.device_id);
                        UserSessionsListItem::for_user_session(UserSession::new(
                            &session,
                            user_session,
                            crypto_session,
                        ))
                    })
                    .collect();

                self.update_list(user_sessions);

                self.set_current_user_session(current_user_session.map(|user_session| {
                    let crypto_session = crypto_sessions.get(&user_session.device_id);
                    UserSessionsListItem::for_user_session(UserSession::new(
                        &session,
                        user_session,
                        crypto_session,
                    ))
                }));
            }
            Err(error) => {
                error!("Couldn’t load user sessions list: {error}");
                self.update_list(vec![UserSessionsListItem::for_error(gettext(
                    "Failed to load the list of connected devices.",
                ))]);
            }
        }
        self.set_loading(false);
    }

    /// Load the list of user sessions.
    pub fn load(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let client = session.client();

        self.set_loading(true);

        let handle = spawn_tokio!(async move {
            let user_id = client.user_id().unwrap();
            let crypto_sessions = client.encryption().get_user_devices(user_id).await?;

            match client.devices().await {
                Ok(mut response) => {
                    response
                        .devices
                        .sort_unstable_by(|a, b| b.last_seen_ts.cmp(&a.last_seen_ts));

                    let current_user_session = if let Some(current_device_id) = client.device_id() {
                        if let Some(index) = response
                            .devices
                            .iter()
                            .position(|device| *device.device_id == current_device_id.as_ref())
                        {
                            Some(response.devices.remove(index))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    Ok((current_user_session, response.devices, crypto_sessions))
                }
                Err(error) => Err(Error::Http(error)),
            }
        });

        spawn!(clone!(@weak self as obj => async move {
            obj.finish_loading(handle.await.unwrap());
        }));
    }
}

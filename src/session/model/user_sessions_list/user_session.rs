use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::encryption::identities::Device as CryptoDevice;
use ruma::{api::client::device::Device as DeviceData, DeviceId, OwnedDeviceId};
use tracing::{debug, error};

use crate::{
    components::{AuthDialog, AuthError},
    prelude::*,
    session::model::Session,
};

/// The possible sources of the user data.
#[derive(Debug, Clone)]
pub enum UserSessionData {
    /// We only have the device ID.
    DeviceId(OwnedDeviceId),
    /// The data comes from the `/devices` API.
    DevicesApi(DeviceData),
    /// The data comes from the crypto store.
    Crypto(CryptoDevice),
    /// The data comes from both sources.
    Both {
        api: DeviceData,
        crypto: CryptoDevice,
    },
}

impl UserSessionData {
    /// The ID of the user session.
    pub fn device_id(&self) -> &DeviceId {
        match self {
            UserSessionData::DeviceId(device_id) => device_id,
            UserSessionData::DevicesApi(api) | UserSessionData::Both { api, .. } => &api.device_id,
            UserSessionData::Crypto(crypto) => crypto.device_id(),
        }
    }

    /// The `/devices` API data.
    pub fn api(&self) -> Option<&DeviceData> {
        match self {
            UserSessionData::DevicesApi(api) | UserSessionData::Both { api, .. } => Some(api),
            UserSessionData::DeviceId(_) | UserSessionData::Crypto(_) => None,
        }
    }

    /// The crypto API.
    pub fn crypto(&self) -> Option<&CryptoDevice> {
        match self {
            UserSessionData::Crypto(crypto) | UserSessionData::Both { crypto, .. } => Some(crypto),
            UserSessionData::DeviceId(_) | UserSessionData::DevicesApi(_) => None,
        }
    }
}

mod imp {
    use std::{
        cell::{Cell, OnceCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::UserSession)]
    pub struct UserSession {
        /// The current session.
        #[property(get, construct_only)]
        session: glib::WeakRef<Session>,
        /// The user session data.
        data: OnceCell<UserSessionData>,
        /// Whether this is the current user session.
        #[property(get)]
        is_current: Cell<bool>,
        /// The ID of the user session.
        #[property(get = Self::device_id)]
        device_id: PhantomData<String>,
        /// The display name of the user session.
        #[property(get = Self::display_name)]
        display_name: PhantomData<String>,
        /// The last IP address used by the user session.
        #[property(get = Self::last_seen_ip)]
        last_seen_ip: PhantomData<Option<String>>,
        /// The last time the user session was used.
        #[property(get = Self::last_seen_ts)]
        last_seen_ts: PhantomData<Option<glib::DateTime>>,
        /// Whether this user session is verified.
        #[property(get = Self::verified)]
        verified: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserSession {
        const NAME: &'static str = "UserSession";
        type Type = super::UserSession;
    }

    #[glib::derived_properties]
    impl ObjectImpl for UserSession {}

    impl UserSession {
        /// Set the user session data.
        pub(super) fn set_data(&self, data: UserSessionData) {
            if let Some(session) = self.session.upgrade() {
                let is_current = *session.device_id() == data.device_id();
                self.is_current.set(is_current);
            }

            self.data.set(data).expect("data is uninitialized");
        }

        /// The user session data.
        pub(super) fn data(&self) -> &UserSessionData {
            self.data.get().expect("data is initialized")
        }

        /// The ID of this user session.
        fn device_id(&self) -> String {
            self.data().device_id().to_string()
        }

        /// The display name of the device.
        fn display_name(&self) -> String {
            if let Some(display_name) = self.data().api().and_then(|d| d.display_name.clone()) {
                display_name
            } else {
                self.device_id()
            }
        }

        /// The last IP address used by the user session.
        fn last_seen_ip(&self) -> Option<String> {
            self.data().api()?.last_seen_ip.clone()
        }

        /// The last time the user session was used.
        fn last_seen_ts(&self) -> Option<glib::DateTime> {
            self.data().api()?.last_seen_ts.map(|last_seen_ts| {
                glib::DateTime::from_unix_utc(last_seen_ts.as_secs().into())
                    .and_then(|t| t.to_local())
                    .expect("constructing GDateTime works")
            })
        }

        /// Whether this device is verified.
        fn verified(&self) -> bool {
            self.data().crypto().is_some_and(CryptoDevice::is_verified)
        }
    }
}

glib::wrapper! {
    /// A user's session.
    pub struct UserSession(ObjectSubclass<imp::UserSession>);
}

impl UserSession {
    pub fn new(session: &Session, data: UserSessionData) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        obj.imp().set_data(data);

        obj
    }

    /// Deletes the `UserSession`.
    ///
    /// Requires a widget because it might show a dialog for UIAA.
    pub async fn delete(&self, parent: &impl IsA<gtk::Widget>) -> Result<(), AuthError> {
        let Some(session) = self.session() else {
            return Err(AuthError::NoSession);
        };
        let device_id = self.imp().data().device_id().to_owned();

        let dialog = AuthDialog::new(&session);

        let res = dialog
            .authenticate(parent, move |client, auth| {
                let device_id = device_id.clone();
                async move {
                    client
                        .delete_devices(&[device_id], auth)
                        .await
                        .map_err(Into::into)
                }
            })
            .await;

        match res {
            Ok(_) => Ok(()),
            Err(error) => {
                let device_id = self.device_id();
                if matches!(error, AuthError::UserCancelled) {
                    debug!("Deletion of user session {device_id} cancelled by user");
                } else {
                    error!("Could not delete user session {device_id}: {error:?}");
                }
                Err(error)
            }
        }
    }
}

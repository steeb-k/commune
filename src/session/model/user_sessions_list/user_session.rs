use gtk::{glib, glib::closure_local, prelude::*, subclass::prelude::*};
use matrix_sdk::encryption::identities::Device as CryptoDevice;
use ruma::{api::client::device::Device as DeviceData, DeviceId, OwnedDeviceId};
use tracing::{debug, error};

use crate::{
    components::{AuthDialog, AuthError},
    prelude::*,
    session::model::Session,
    utils::matrix::timestamp_to_date,
};

/// The possible sources of the user data.
#[derive(Debug, Clone)]
pub(super) enum UserSessionData {
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
    pub(super) fn device_id(&self) -> &DeviceId {
        match self {
            UserSessionData::DevicesApi(api) | UserSessionData::Both { api, .. } => &api.device_id,
            UserSessionData::Crypto(crypto) => crypto.device_id(),
        }
    }

    /// The `/devices` API data.
    fn api(&self) -> Option<&DeviceData> {
        match self {
            UserSessionData::DevicesApi(api) | UserSessionData::Both { api, .. } => Some(api),
            UserSessionData::Crypto(_) => None,
        }
    }

    /// The crypto API.
    fn crypto(&self) -> Option<&CryptoDevice> {
        match self {
            UserSessionData::Crypto(crypto) | UserSessionData::Both { crypto, .. } => Some(crypto),
            UserSessionData::DevicesApi(_) => None,
        }
    }
}

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::UserSession)]
    pub struct UserSession {
        /// The current session.
        #[property(get, construct_only)]
        session: glib::WeakRef<Session>,
        /// The ID of the user session.
        device_id: OnceCell<OwnedDeviceId>,
        /// The user session data.
        data: RefCell<Option<UserSessionData>>,
        /// Whether this is the current user session.
        #[property(get)]
        is_current: Cell<bool>,
        /// The ID of the user session, as a string.
        #[property(get = Self::device_id_string)]
        device_id_string: PhantomData<String>,
        /// The display name of the user session.
        #[property(get = Self::display_name)]
        display_name: PhantomData<String>,
        /// The last IP address used by the user session.
        #[property(get = Self::last_seen_ip)]
        last_seen_ip: PhantomData<Option<String>>,
        /// The last time the user session was used, as the number of
        /// milliseconds since Unix EPOCH.
        #[property(get = Self::last_seen_ts)]
        last_seen_ts: PhantomData<u64>,
        /// The last time the user session was used, as a `GDateTime`.
        #[property(get = Self::last_seen_datetime)]
        last_seen_datetime: PhantomData<Option<glib::DateTime>>,
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
    impl ObjectImpl for UserSession {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("disconnected").build()]);
            SIGNALS.as_ref()
        }
    }

    impl UserSession {
        /// She the ID of this user session.
        pub(super) fn set_device_id(&self, device_id: OwnedDeviceId) {
            let device_id = self.device_id.get_or_init(|| device_id);

            if let Some(session) = self.session.upgrade() {
                let is_current = session.device_id() == device_id;
                self.is_current.set(is_current);
            }
        }

        /// The ID of this user session.
        pub(super) fn device_id(&self) -> &OwnedDeviceId {
            self.device_id
                .get()
                .expect("device ID should be initialized")
        }

        /// Set the user session data.
        pub(super) fn set_data(&self, data: UserSessionData) {
            let old_display_name = self.display_name();
            let old_last_seen_ip = self.last_seen_ip();
            let old_last_seen_ts = self.last_seen_ts();
            let old_verified = self.verified();

            self.data.replace(Some(data));

            let obj = self.obj();
            if self.display_name() != old_display_name {
                obj.notify_display_name();
            }
            if self.last_seen_ip() != old_last_seen_ip {
                obj.notify_last_seen_ip();
            }
            if self.last_seen_ts() != old_last_seen_ts {
                obj.notify_last_seen_ts();
            }
            if self.verified() != old_verified {
                obj.notify_verified();
            }
        }

        /// The ID of this user session, as a string.
        fn device_id_string(&self) -> String {
            self.device_id().to_string()
        }

        /// The display name of the device.
        fn display_name(&self) -> String {
            if let Some(display_name) = self
                .data
                .borrow()
                .as_ref()
                .and_then(UserSessionData::api)
                .and_then(|d| d.display_name.clone())
            {
                display_name
            } else {
                self.device_id_string()
            }
        }

        /// The last IP address used by the user session.
        fn last_seen_ip(&self) -> Option<String> {
            self.data.borrow().as_ref()?.api()?.last_seen_ip.clone()
        }

        /// The last time the user session was used, as the number of
        /// milliseconds since Unix EPOCH.
        ///
        /// Defaults to `0` if the timestamp is unknown.
        fn last_seen_ts(&self) -> u64 {
            self.data
                .borrow()
                .as_ref()
                .and_then(UserSessionData::api)
                .and_then(|s| s.last_seen_ts)
                .map(|ts| ts.0.into())
                .unwrap_or_default()
        }

        /// The last time the user session was used, as a `GDateTime`.
        fn last_seen_datetime(&self) -> Option<glib::DateTime> {
            self.data
                .borrow()
                .as_ref()?
                .api()?
                .last_seen_ts
                .map(timestamp_to_date)
        }

        /// Whether this device is verified.
        fn verified(&self) -> bool {
            self.data
                .borrow()
                .as_ref()
                .and_then(UserSessionData::crypto)
                .is_some_and(CryptoDevice::is_verified)
        }
    }
}

glib::wrapper! {
    /// A user's session.
    pub struct UserSession(ObjectSubclass<imp::UserSession>);
}

impl UserSession {
    pub(super) fn new(session: &Session, device_id: OwnedDeviceId) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        obj.imp().set_device_id(device_id);

        obj
    }

    /// The ID of this user session.
    pub(crate) fn device_id(&self) -> &OwnedDeviceId {
        self.imp().device_id()
    }

    /// Set the user session data.
    pub(super) fn set_data(&self, data: UserSessionData) {
        self.imp().set_data(data);
    }

    /// Deletes the `UserSession`.
    ///
    /// Requires a widget because it might show a dialog for UIAA.
    pub(crate) async fn delete(&self, parent: &impl IsA<gtk::Widget>) -> Result<(), AuthError> {
        let Some(session) = self.session() else {
            return Err(AuthError::NoSession);
        };
        let device_id = self.imp().device_id().clone();

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
                let device_id = self.imp().device_id();

                if matches!(error, AuthError::UserCancelled) {
                    debug!("Deletion of user session {device_id} cancelled by user");
                } else {
                    error!("Could not delete user session {device_id}: {error:?}");
                }
                Err(error)
            }
        }
    }

    /// Signal that this session was disconnected.
    pub(super) fn emit_disconnected(&self) {
        self.emit_by_name::<()>("disconnected", &[]);
    }

    /// Connect to the signal emitted when this session is disconnected.
    pub fn connect_disconnected<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "disconnected",
            true,
            closure_local!(|obj: Self| {
                f(&obj);
            }),
        )
    }
}

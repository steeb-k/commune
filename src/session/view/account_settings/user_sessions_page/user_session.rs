use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::encryption::identities::Device as CryptoDevice;
use ruma::{
    api::client::device::{delete_device, Device as MatrixDevice},
    assign,
};

use crate::{
    components::{AuthDialog, AuthError},
    session::model::Session,
};

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::UserSession)]
    pub struct UserSession {
        /// The user session data.
        pub data: OnceCell<MatrixDevice>,
        /// The encryption API of the user session.
        pub crypto: OnceCell<CryptoDevice>,
        /// The current session.
        #[property(get, construct_only)]
        pub session: glib::WeakRef<Session>,
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
        /// The user session data.
        pub(super) fn data(&self) -> &MatrixDevice {
            self.data.get().unwrap()
        }

        /// The ID of this user session.
        fn device_id(&self) -> String {
            self.data().device_id.to_string()
        }

        /// The display name of the device.
        fn display_name(&self) -> String {
            if let Some(display_name) = self.data().display_name.clone() {
                display_name
            } else {
                self.device_id()
            }
        }

        /// The last IP address used by the user session.
        fn last_seen_ip(&self) -> Option<String> {
            // TODO: Would be nice to also show the location
            // See: https://gitlab.gnome.org/GNOME/fractal/-/issues/700
            self.data().last_seen_ip.clone()
        }

        /// The last time the user session was used.
        fn last_seen_ts(&self) -> Option<glib::DateTime> {
            self.data().last_seen_ts.map(|last_seen_ts| {
                glib::DateTime::from_unix_utc(last_seen_ts.as_secs().into())
                    .and_then(|t| t.to_local())
                    .unwrap()
            })
        }

        /// Whether this device is verified.
        fn verified(&self) -> bool {
            self.crypto.get().is_some_and(|d| d.is_verified())
        }
    }
}

glib::wrapper! {
    /// A user's session.
    pub struct UserSession(ObjectSubclass<imp::UserSession>);
}

impl UserSession {
    pub fn new(session: &Session, data: MatrixDevice, crypto: Option<CryptoDevice>) -> Self {
        let obj: Self = glib::Object::builder().property("session", session).build();

        obj.set_data(data, crypto);

        obj
    }

    /// Set the SDK data of this `UserSession`.
    fn set_data(&self, data: MatrixDevice, crypto: Option<CryptoDevice>) {
        let imp = self.imp();
        imp.data.set(data).unwrap();
        if let Some(crypto) = crypto {
            imp.crypto.set(crypto).unwrap();
        }
    }

    /// Deletes the `UserSession`.
    pub async fn delete(
        &self,
        transient_for: Option<&impl IsA<gtk::Window>>,
    ) -> Result<(), AuthError> {
        let Some(session) = self.session() else {
            return Err(AuthError::NoSession);
        };
        let device_id = self.imp().data().device_id.clone();

        let dialog = AuthDialog::new(transient_for, &session);

        dialog
            .authenticate(move |client, auth| {
                let device_id = device_id.clone();
                async move {
                    let request = assign!(delete_device::v3::Request::new(device_id), { auth });
                    client.send(request, None).await.map_err(Into::into)
                }
            })
            .await?;
        Ok(())
    }
}

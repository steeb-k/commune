use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    encryption::identities::Device as CryptoDevice,
    ruma::{
        api::client::device::{delete_device, Device as MatrixDevice},
        assign,
    },
};

use crate::{
    components::{AuthDialog, AuthError},
    session::model::Session,
};

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Device)]
    pub struct Device {
        /// The device data.
        pub device: OnceCell<MatrixDevice>,
        /// The encryption API of the device.
        pub crypto_device: OnceCell<CryptoDevice>,
        /// The current session.
        #[property(get, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// The ID of the device.
        #[property(get = Self::device_id)]
        device_id: PhantomData<String>,
        /// The display name of the device.
        #[property(get = Self::display_name)]
        display_name: PhantomData<String>,
        /// The last IP address the device used.
        #[property(get = Self::last_seen_ip)]
        last_seen_ip: PhantomData<Option<String>>,
        /// The last time the device was used.
        #[property(get = Self::last_seen_ts)]
        last_seen_ts: PhantomData<Option<glib::DateTime>>,
        /// Whether this device is verified.
        #[property(get = Self::verified)]
        verified: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Device {
        const NAME: &'static str = "Device";
        type Type = super::Device;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Device {}

    impl Device {
        /// The device data.
        fn device(&self) -> &MatrixDevice {
            self.device.get().unwrap()
        }

        /// The ID of this device.
        fn device_id(&self) -> String {
            self.device().device_id.to_string()
        }

        /// The display name of the device.
        fn display_name(&self) -> String {
            if let Some(display_name) = self.device().display_name.clone() {
                display_name
            } else {
                self.device_id()
            }
        }

        /// The last IP address the device used.
        fn last_seen_ip(&self) -> Option<String> {
            // TODO: Would be nice to also show the location
            // See: https://gitlab.gnome.org/GNOME/fractal/-/issues/700
            self.device().last_seen_ip.clone()
        }

        /// The last time the device was used.
        fn last_seen_ts(&self) -> Option<glib::DateTime> {
            self.device().last_seen_ts.map(|last_seen_ts| {
                glib::DateTime::from_unix_utc(last_seen_ts.as_secs().into())
                    .and_then(|t| t.to_local())
                    .unwrap()
            })
        }

        /// Whether this device is verified.
        fn verified(&self) -> bool {
            self.crypto_device
                .get()
                .is_some_and(|device| device.is_verified())
        }
    }
}

glib::wrapper! {
    /// `glib::Object` representation of a Device/Session of a User.
    pub struct Device(ObjectSubclass<imp::Device>);
}

impl Device {
    pub fn new(
        session: &Session,
        device: MatrixDevice,
        crypto_device: Option<CryptoDevice>,
    ) -> Self {
        let obj: Self = glib::Object::builder().property("session", session).build();

        obj.set_matrix_device(device, crypto_device);

        obj
    }

    /// Set the Matrix device of this `Device`.
    fn set_matrix_device(&self, device: MatrixDevice, crypto_device: Option<CryptoDevice>) {
        let imp = self.imp();
        imp.device.set(device).unwrap();
        if let Some(crypto_device) = crypto_device {
            imp.crypto_device.set(crypto_device).unwrap();
        }
    }

    /// Deletes the `Device`.
    pub async fn delete(
        &self,
        transient_for: Option<&impl IsA<gtk::Window>>,
    ) -> Result<(), AuthError> {
        let Some(session) = self.session() else {
            return Err(AuthError::NoSession);
        };
        let device_id = self.imp().device.get().unwrap().device_id.clone();

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

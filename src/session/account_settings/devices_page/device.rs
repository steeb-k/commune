use gtk::{glib, prelude::*, subclass::prelude::*};

use crate::components::AuthDialog;
use crate::session::Session;
use matrix_sdk::{
    encryption::identities::Device as CryptoDevice,
    ruma::{
        api::client::r0::device::{delete_device, Device as MatrixDevice},
        assign,
        identifiers::DeviceId,
    },
};

use log::error;

mod imp {
    use super::*;
    use glib::object::WeakRef;
    use once_cell::{sync::Lazy, unsync::OnceCell};

    #[derive(Debug, Default)]
    pub struct Device {
        pub device: OnceCell<MatrixDevice>,
        pub crypto_device: OnceCell<CryptoDevice>,
        pub session: OnceCell<WeakRef<Session>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Device {
        const NAME: &'static str = "Device";
        type Type = super::Device;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for Device {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "session",
                        "Session",
                        "The session",
                        Session::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "device-id",
                        "Device Id",
                        "The Id of this device",
                        None,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name of the device",
                        None,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_string(
                        "last-seen-ip",
                        "Last Seen Ip",
                        "The last ip the device used",
                        None,
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_pointer(
                        "last-seen-ts",
                        "Last Seen Ts",
                        "The last time the device was used",
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_pointer(
                        "verified",
                        "Verified",
                        "Whether this devices is verified",
                        glib::ParamFlags::READABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "session" => self
                    .session
                    .set(value.get::<Session>().unwrap().downgrade())
                    .unwrap(),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "session" => obj.session().to_value(),
                "display-name" => obj.display_name().to_value(),
                "device-id" => obj.device_id().as_str().to_value(),
                "last-seen-ip" => obj.last_seen_ip().to_value(),
                "last-seen-ts" => obj.last_seen_ts().to_value(),
                "verified" => obj.is_verified().to_value(),
                _ => unimplemented!(),
            }
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
        let obj: Self =
            glib::Object::new(&[("session", session)]).expect("Failed to create Device");

        obj.set_matrix_device(device, crypto_device);

        obj
    }

    pub fn session(&self) -> Session {
        let priv_ = imp::Device::from_instance(self);
        priv_.session.get().unwrap().upgrade().unwrap()
    }

    fn set_matrix_device(&self, device: MatrixDevice, crypto_device: Option<CryptoDevice>) {
        let priv_ = imp::Device::from_instance(self);
        priv_.device.set(device).unwrap();
        if let Some(crypto_device) = crypto_device {
            priv_.crypto_device.set(crypto_device).unwrap();
        }
    }

    pub fn device_id(&self) -> &DeviceId {
        let priv_ = imp::Device::from_instance(self);
        &priv_.device.get().unwrap().device_id
    }

    pub fn display_name(&self) -> &str {
        let priv_ = imp::Device::from_instance(self);
        if let Some(ref display_name) = priv_.device.get().unwrap().display_name {
            display_name
        } else {
            self.device_id().as_str()
        }
    }

    pub fn last_seen_ip(&self) -> Option<&str> {
        let priv_ = imp::Device::from_instance(self);
        // TODO: Would be nice to also show the location
        // See: https://gitlab.gnome.org/GNOME/fractal/-/issues/700
        priv_.device.get().unwrap().last_seen_ip.as_deref()
    }

    pub fn last_seen_ts(&self) -> Option<glib::DateTime> {
        let priv_ = imp::Device::from_instance(self);
        priv_
            .device
            .get()
            .unwrap()
            .last_seen_ts
            .map(|last_seen_ts| {
                glib::DateTime::from_unix_utc(last_seen_ts.as_secs().into())
                    .and_then(|t| t.to_local())
                    .unwrap()
            })
    }

    /// Delete the `Device`
    ///
    /// Returns `true` for success
    pub async fn delete(&self, transient_for: Option<&impl IsA<gtk::Window>>) -> bool {
        let session = self.session();
        let device_id = self.device_id().to_owned();

        let dialog = AuthDialog::new(transient_for, &session);

        let result = dialog
            .authenticate(move |client, auth_data| {
                let device_id = device_id.clone();
                async move {
                    if let Some(auth) = auth_data {
                        let auth = Some(auth.as_matrix_auth_data());
                        let request = assign!(delete_device::Request::new(&device_id), { auth });
                        client.send(request, None).await.map_err(Into::into)
                    } else {
                        let request = delete_device::Request::new(&device_id);
                        client.send(request, None).await.map_err(Into::into)
                    }
                }
            })
            .await;
        match result {
            Some(Ok(_)) => true,
            Some(Err(err)) => {
                // TODO: show error message to the user
                error!("Failed to delete device: {}", err);
                false
            }
            None => false,
        }
    }

    pub fn is_verified(&self) -> bool {
        let priv_ = imp::Device::from_instance(self);
        priv_
            .crypto_device
            .get()
            .map_or(false, |device| device.verified())
    }
}

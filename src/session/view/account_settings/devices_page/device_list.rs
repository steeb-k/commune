use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::{
    encryption::identities::UserDevices as CryptoDevices,
    ruma::api::client::device::Device as MatrixDevice, Error,
};
use tracing::error;

use super::{Device, DeviceListItem};
use crate::{session::model::Session, spawn, spawn_tokio};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::DeviceList)]
    pub struct DeviceList {
        /// The list of device list items.
        pub list: RefCell<Vec<DeviceListItem>>,
        /// The current session.
        #[property(get, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// The device of this session.
        pub current_device_inner: RefCell<Option<DeviceListItem>>,
        /// The device of this session, or a replacement list item if it is not
        /// found.
        #[property(get = Self::current_device)]
        current_device: PhantomData<DeviceListItem>,
        pub loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeviceList {
        const NAME: &'static str = "DeviceList";
        type Type = super::DeviceList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for DeviceList {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().load_devices();
        }
    }

    impl ListModelImpl for DeviceList {
        fn item_type(&self) -> glib::Type {
            DeviceListItem::static_type()
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

    impl DeviceList {
        /// The device of this session.
        fn current_device(&self) -> DeviceListItem {
            self.current_device_inner
                .borrow()
                .clone()
                .unwrap_or_else(|| {
                    if self.loading.get() {
                        DeviceListItem::for_loading_spinner()
                    } else {
                        DeviceListItem::for_error(gettext("Failed to load connected device."))
                    }
                })
        }
    }
}

glib::wrapper! {
    /// List of active devices for the logged in user.
    pub struct DeviceList(ObjectSubclass<imp::DeviceList>)
        @implements gio::ListModel;
}

impl DeviceList {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    fn set_loading(&self, loading: bool) {
        let imp = self.imp();

        if loading == imp.loading.get() {
            return;
        }
        if loading {
            self.update_list(vec![DeviceListItem::for_loading_spinner()]);
        }
        imp.loading.set(loading);
        self.notify_current_device();
    }

    /// Set the device of this session.
    fn set_current_device(&self, device: Option<DeviceListItem>) {
        self.imp().current_device_inner.replace(device);

        self.notify_current_device();
    }

    fn update_list(&self, devices: Vec<DeviceListItem>) {
        let added = devices.len();

        let prev_devices = self.imp().list.replace(devices);

        self.items_changed(0, prev_devices.len() as u32, added as u32);
    }

    fn finish_loading(
        &self,
        response: Result<(Option<MatrixDevice>, Vec<MatrixDevice>, CryptoDevices), Error>,
    ) {
        let Some(session) = self.session() else {
            return;
        };

        match response {
            Ok((current_device, devices, crypto_devices)) => {
                let devices = devices
                    .into_iter()
                    .map(|device| {
                        let crypto_device = crypto_devices.get(&device.device_id);
                        DeviceListItem::for_device(Device::new(&session, device, crypto_device))
                    })
                    .collect();

                self.update_list(devices);

                self.set_current_device(current_device.map(|device| {
                    let crypto_device = crypto_devices.get(&device.device_id);
                    DeviceListItem::for_device(Device::new(&session, device, crypto_device))
                }));
            }
            Err(error) => {
                error!("Couldn’t load device list: {error}");
                self.update_list(vec![DeviceListItem::for_error(gettext(
                    "Failed to load the list of connected devices.",
                ))]);
            }
        }
        self.set_loading(false);
    }

    pub fn load_devices(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let client = session.client();

        self.set_loading(true);

        let handle = spawn_tokio!(async move {
            let user_id = client.user_id().unwrap();
            let crypto_devices = client.encryption().get_user_devices(user_id).await?;

            match client.devices().await {
                Ok(mut response) => {
                    response
                        .devices
                        .sort_unstable_by(|a, b| b.last_seen_ts.cmp(&a.last_seen_ts));

                    let current_device = if let Some(current_device_id) = client.device_id() {
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

                    Ok((current_device, response.devices, crypto_devices))
                }
                Err(error) => Err(Error::Http(error)),
            }
        });

        spawn!(clone!(@weak self as obj => async move {
            obj.finish_loading(handle.await.unwrap());
        }));
    }
}

use gtk::{glib, prelude::*, subclass::prelude::*};

use super::Device;

/// This enum contains all possible types the device list can hold.
#[derive(Debug, Clone, glib::Boxed)]
#[boxed_type(name = "DeviceListItemType")]
pub enum DeviceListItemType {
    Device(Device),
    Error(String),
    LoadingSpinner,
}

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::DeviceListItem)]
    pub struct DeviceListItem {
        /// The type of this item.
        #[property(get, construct_only)]
        pub item_type: OnceCell<DeviceListItemType>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeviceListItem {
        const NAME: &'static str = "DeviceListItem";
        type Type = super::DeviceListItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for DeviceListItem {}
}

glib::wrapper! {
    /// An item in the device list.
    pub struct DeviceListItem(ObjectSubclass<imp::DeviceListItem>);
}

impl DeviceListItem {
    pub fn for_device(device: Device) -> Self {
        let item_type = DeviceListItemType::Device(device);
        glib::Object::builder()
            .property("item-type", &item_type)
            .build()
    }

    pub fn for_error(error: String) -> Self {
        let item_type = DeviceListItemType::Error(error);
        glib::Object::builder()
            .property("item-type", &item_type)
            .build()
    }

    pub fn for_loading_spinner() -> Self {
        let item_type = DeviceListItemType::LoadingSpinner;
        glib::Object::builder()
            .property("item-type", &item_type)
            .build()
    }
}

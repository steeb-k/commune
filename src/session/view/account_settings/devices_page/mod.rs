use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

mod device;
mod device_item;
mod device_list;
mod device_row;

use self::{
    device::Device,
    device_item::{DeviceListItem, DeviceListItemType},
    device_list::DeviceList,
    device_row::DeviceRow,
};
use crate::{components::LoadingRow, session::model::User};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/devices_page/mod.ui"
    )]
    #[properties(wrapper_type = super::DevicesPage)]
    pub struct DevicesPage {
        /// The logged-in user.
        #[property(get, set = Self::set_user, explicit_notify)]
        pub user: RefCell<Option<User>>,
        #[template_child]
        pub other_sessions_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub other_sessions: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub current_session: TemplateChild<gtk::ListBox>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DevicesPage {
        const NAME: &'static str = "DevicesPage";
        type Type = super::DevicesPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for DevicesPage {}

    impl WidgetImpl for DevicesPage {}
    impl PreferencesPageImpl for DevicesPage {}

    impl DevicesPage {
        /// Set the logged-in user.
        fn set_user(&self, user: Option<User>) {
            if *self.user.borrow() == user {
                return;
            }
            let obj = self.obj();

            if let Some(user) = &user {
                let device_list = DeviceList::new(&user.session());
                self.other_sessions.bind_model(
                    Some(&device_list),
                    clone!(@weak device_list => @default-panic, move |item| {
                        match item.downcast_ref::<DeviceListItem>().unwrap().item_type() {
                            DeviceListItemType::Device(device) => DeviceRow::new(&device, false).upcast(),
                            DeviceListItemType::Error(error) => {
                                let row = LoadingRow::new();
                                row.set_error(Some(error.clone()));
                                row.connect_retry(clone!(@weak device_list => move|_| {
                                    device_list.load_devices()
                                }));
                                row.upcast()
                            }
                            DeviceListItemType::LoadingSpinner => {
                                LoadingRow::new().upcast()
                            }
                        }
                    }),
                );

                device_list.connect_items_changed(
                    clone!(@weak obj => move |device_list, _, _, _| {
                        obj.set_other_sessions_visibility(device_list.n_items() > 0)
                    }),
                );

                obj.set_other_sessions_visibility(device_list.n_items() > 0);

                device_list.connect_current_device_notify(clone!(@weak obj => move |device_list| {
                    obj.set_current_device(device_list);
                }));

                obj.set_current_device(&device_list);
            } else {
                self.other_sessions.unbind_model();

                if let Some(child) = self.current_session.first_child() {
                    self.current_session.remove(&child);
                }
            }

            self.user.replace(user);
            obj.notify_user();
        }
    }
}

glib::wrapper! {
    /// User devices page.
    pub struct DevicesPage(ObjectSubclass<imp::DevicesPage>)
        @extends gtk::Widget, gtk::Window, adw::Window, adw::PreferencesWindow, @implements gtk::Accessible;
}

impl DevicesPage {
    pub fn new(user: &User) -> Self {
        glib::Object::builder().property("user", user).build()
    }

    fn set_other_sessions_visibility(&self, visible: bool) {
        self.imp().other_sessions_group.set_visible(visible);
    }

    fn set_current_device(&self, device_list: &DeviceList) {
        let imp = self.imp();
        if let Some(child) = imp.current_session.first_child() {
            imp.current_session.remove(&child);
        }
        let row: gtk::Widget = match device_list.current_device().item_type() {
            DeviceListItemType::Device(device) => DeviceRow::new(&device, true).upcast(),
            DeviceListItemType::Error(error) => {
                let row = LoadingRow::new();
                row.set_error(Some(error.clone()));
                row.connect_retry(clone!(@weak device_list => move|_| {
                    device_list.load_devices()
                }));
                row.upcast()
            }
            DeviceListItemType::LoadingSpinner => LoadingRow::new().upcast(),
        };
        imp.current_session.append(&row);
    }
}

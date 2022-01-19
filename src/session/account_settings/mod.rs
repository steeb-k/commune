use adw::subclass::prelude::*;
use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

mod devices_page;
use devices_page::DevicesPage;

use crate::session::User;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/account-settings.ui")]
    pub struct AccountSettings {
        pub user: RefCell<Option<User>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AccountSettings {
        const NAME: &'static str = "AccountSettings";
        type Type = super::AccountSettings;
        type ParentType = adw::PreferencesWindow;

        fn class_init(klass: &mut Self::Class) {
            DevicesPage::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AccountSettings {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecObject::new(
                    "user",
                    "User",
                    "The user of this account",
                    User::static_type(),
                    glib::ParamFlags::READWRITE,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "user" => obj.set_user(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "user" => obj.user().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for AccountSettings {}
    impl WindowImpl for AccountSettings {}
    impl AdwWindowImpl for AccountSettings {}
    impl PreferencesWindowImpl for AccountSettings {}
}

glib::wrapper! {
    /// Preference Window to display and update room details.
    pub struct AccountSettings(ObjectSubclass<imp::AccountSettings>)
        @extends gtk::Widget, gtk::Window, adw::Window, adw::PreferencesWindow, @implements gtk::Accessible;
}

impl AccountSettings {
    pub fn new(parent_window: Option<&impl IsA<gtk::Window>>, user: &User) -> Self {
        glib::Object::new(&[("transient-for", &parent_window), ("user", user)])
            .expect("Failed to create AccountSettings")
    }

    pub fn user(&self) -> Option<User> {
        let priv_ = imp::AccountSettings::from_instance(self);
        priv_.user.borrow().clone()
    }

    fn set_user(&self, user: Option<User>) {
        let priv_ = imp::AccountSettings::from_instance(self);

        if self.user() == user {
            return;
        }

        priv_.user.replace(user);
        self.notify("user");
    }
}

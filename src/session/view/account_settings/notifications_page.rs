use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

use crate::{
    components::Spinner, session::model::NotificationsSettings, spawn, toast,
    utils::BoundObjectWeakRef,
};

mod imp {
    use std::cell::Cell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/notifications_page.ui"
    )]
    #[properties(wrapper_type = super::NotificationsPage)]
    pub struct NotificationsPage {
        #[template_child]
        pub account_switch: TemplateChild<gtk::Switch>,
        #[template_child]
        pub session_row: TemplateChild<adw::SwitchRow>,
        /// The notifications settings of the current session.
        #[property(get, set = Self::set_notifications_settings, explicit_notify)]
        pub notifications_settings: BoundObjectWeakRef<NotificationsSettings>,
        /// Whether the account section is busy.
        #[property(get)]
        pub account_loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NotificationsPage {
        const NAME: &'static str = "NotificationsPage";
        type Type = super::NotificationsPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Spinner::static_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for NotificationsPage {}

    impl WidgetImpl for NotificationsPage {}
    impl PreferencesPageImpl for NotificationsPage {}

    impl NotificationsPage {
        /// Set the notifications settings of the current session.
        fn set_notifications_settings(
            &self,
            notifications_settings: Option<&NotificationsSettings>,
        ) {
            if self.notifications_settings.obj().as_ref() == notifications_settings {
                return;
            }
            let obj = self.obj();

            self.notifications_settings.disconnect_signals();

            if let Some(settings) = notifications_settings {
                let account_enabled_handler =
                    settings.connect_account_enabled_notify(clone!(@weak obj => move |_| {
                        obj.update_account();
                    }));
                let session_enabled_handler =
                    settings.connect_session_enabled_notify(clone!(@weak obj => move |_| {
                        obj.update_session();
                    }));

                self.notifications_settings.set(
                    settings,
                    vec![account_enabled_handler, session_enabled_handler],
                );
            }

            obj.update_account();
            obj.update_session();
            obj.notify_notifications_settings();
        }
    }
}

glib::wrapper! {
    /// Preferences page to edit global notification settings.
    pub struct NotificationsPage(ObjectSubclass<imp::NotificationsPage>)
        @extends gtk::Widget, adw::PreferencesPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl NotificationsPage {
    pub fn new(notifications_settings: &NotificationsSettings) -> Self {
        glib::Object::builder()
            .property("notifications-settings", notifications_settings)
            .build()
    }

    /// Update the section about the account.
    fn update_account(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        imp.account_switch.set_active(settings.account_enabled());
        imp.account_switch.set_sensitive(!self.account_loading());

        // Other sessions will be disabled or not.
        self.update_session();
    }

    /// Update the section about the session.
    fn update_session(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        imp.session_row.set_active(settings.session_enabled());
        imp.session_row.set_sensitive(settings.account_enabled());
    }

    fn set_account_loading(&self, loading: bool) {
        self.imp().account_loading.set(loading);
        self.notify_account_loading();
    }

    #[template_callback]
    fn account_switched(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        let enabled = imp.account_switch.is_active();
        if enabled == settings.account_enabled() {
            // Nothing to do.
            return;
        }

        imp.account_switch.set_sensitive(false);
        self.set_account_loading(true);

        spawn!(clone!(@weak self as obj, @weak settings => async move {
            if settings.set_account_enabled(enabled).await.is_err() {
                let msg = if enabled {
                    gettext("Could not enable account notifications")
                } else {
                    gettext("Could not disable account notifications")
                };
                toast!(obj, msg);
            }

            obj.set_account_loading(false);
            obj.update_account();
        }));
    }

    #[template_callback]
    fn session_switched(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        settings.set_session_enabled(imp.session_row.is_active());
    }
}

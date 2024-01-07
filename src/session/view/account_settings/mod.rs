use adw::{prelude::*, subclass::prelude::*};
use gtk::{
    glib,
    glib::{clone, FromVariant},
    CompositeTemplate,
};

mod general_page;
mod notifications_page;
mod security_page;
mod user_sessions_page;

use self::{
    general_page::GeneralPage, notifications_page::NotificationsPage, security_page::SecurityPage,
    user_sessions_page::UserSessionsPage,
};
use crate::{session::model::Session, utils::BoundObjectWeakRef};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/account_settings/mod.ui")]
    #[properties(wrapper_type = super::AccountSettings)]
    pub struct AccountSettings {
        /// The current session.
        #[property(get, set = Self::set_session, nullable)]
        pub session: BoundObjectWeakRef<Session>,
        pub session_handler: RefCell<Option<glib::SignalHandlerId>>,
        #[template_child]
        pub general_page: TemplateChild<GeneralPage>,
        #[template_child]
        pub security_page: TemplateChild<SecurityPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AccountSettings {
        const NAME: &'static str = "AccountSettings";
        type Type = super::AccountSettings;
        type ParentType = adw::PreferencesWindow;

        fn class_init(klass: &mut Self::Class) {
            UserSessionsPage::static_type();
            GeneralPage::static_type();
            NotificationsPage::static_type();
            SecurityPage::static_type();

            Self::bind_template(klass);

            klass.install_action("account-settings.close", None, |obj, _, _| {
                obj.close();
            });

            klass.install_action("account-settings.logout", None, |obj, _, _| {
                obj.imp().general_page.show_log_out_page();
            });

            klass.install_action("account-settings.export_keys", None, |obj, _, _| {
                obj.imp().security_page.show_export_keys_page();
            });

            klass.install_action("win.add-toast", Some("s"), |obj, _, message| {
                if let Some(message) = message.and_then(String::from_variant) {
                    let toast = adw::Toast::new(&message);
                    obj.add_toast(toast);
                }
            });

            klass.install_action("win.close-subpage", None, |obj, _, _| {
                obj.pop_subpage();
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AccountSettings {}

    impl WidgetImpl for AccountSettings {}
    impl WindowImpl for AccountSettings {}
    impl AdwWindowImpl for AccountSettings {}
    impl PreferencesWindowImpl for AccountSettings {}

    impl AccountSettings {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            if self.session.obj() == session {
                return;
            }
            let obj = self.obj();

            self.session.disconnect_signals();

            if let Some(session) = session {
                let logged_out_handler = session.connect_logged_out(clone!(@weak obj => move |_| {
                    obj.close();
                }));
                self.session.set(&session, vec![logged_out_handler]);
            }

            obj.notify_session();
        }
    }
}

glib::wrapper! {
    /// Preference Window to display and update room details.
    pub struct AccountSettings(ObjectSubclass<imp::AccountSettings>)
        @extends gtk::Widget, gtk::Window, adw::Window, adw::PreferencesWindow, @implements gtk::Accessible;
}

impl AccountSettings {
    pub fn new(parent_window: Option<&impl IsA<gtk::Window>>, session: &Session) -> Self {
        glib::Object::builder()
            .property("transient-for", parent_window)
            .property("session", session)
            .build()
    }
}

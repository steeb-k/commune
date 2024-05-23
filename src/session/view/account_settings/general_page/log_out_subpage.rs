use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, CompositeTemplate};

use crate::{
    components::LoadingButton,
    session::{
        model::{CryptoIdentityState, RecoveryState, Session, SessionVerificationState},
        view::AccountSettings,
    },
    toast,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/general_page/log_out_subpage.ui"
    )]
    #[properties(wrapper_type = super::LogOutSubpage)]
    pub struct LogOutSubpage {
        /// The current session.
        #[property(get, set = Self::set_session, nullable)]
        pub session: glib::WeakRef<Session>,
        #[template_child]
        pub warning_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub warning_description: TemplateChild<gtk::Label>,
        #[template_child]
        pub warning_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub logout_button: TemplateChild<LoadingButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LogOutSubpage {
        const NAME: &'static str = "LogOutSubpage";
        type Type = super::LogOutSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LogOutSubpage {}

    impl WidgetImpl for LogOutSubpage {}
    impl NavigationPageImpl for LogOutSubpage {}

    impl LogOutSubpage {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            self.session.set(session);

            self.update_warning();
        }
        /// Update the warning.
        fn update_warning(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let verification_state = session.verification_state();
            let recovery_state = session.recovery_state();

            if verification_state != SessionVerificationState::Verified
                || recovery_state != RecoveryState::Enabled
            {
                self.warning_description.set_label(&gettext("The crypto identity and account recovery are not set up properly. If this is your last connected session and you have no recent local backup of your encryption keys, you will not be able to restore your account."));
                self.warning_box.set_visible(true);
                return;
            }

            let crypto_identity_state = session.crypto_identity_state();

            if crypto_identity_state == CryptoIdentityState::LastManStanding {
                self.warning_description.set_label(&gettext("This is your last connected session. Make sure that you can still access your recovery key or passphrase, or to backup your encryption keys before logging out."));
                self.warning_box.set_visible(true);
                return;
            }

            // No particular problem, do not show the warning.
            self.warning_box.set_visible(false);
        }
    }
}

glib::wrapper! {
    /// Subpage allowing a user to log out from their account.
    pub struct LogOutSubpage(ObjectSubclass<imp::LogOutSubpage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl LogOutSubpage {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Show the security tab of the settings.
    #[template_callback]
    fn view_security(&self) {
        let Some(dialog) = self
            .ancestor(AccountSettings::static_type())
            .and_downcast::<AccountSettings>()
        else {
            return;
        };

        dialog.pop_subpage();
        dialog.set_visible_page_name("security");
    }

    /// Log out the current session.
    #[template_callback]
    async fn logout(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let imp = self.imp();
        imp.logout_button.set_is_loading(true);
        imp.warning_button.set_sensitive(false);

        if let Err(error) = session.logout().await {
            toast!(self, error);
        }

        imp.logout_button.set_is_loading(false);
        imp.warning_button.set_sensitive(true);
    }
}

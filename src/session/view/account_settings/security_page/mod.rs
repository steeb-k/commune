use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

mod ignored_users_subpage;
mod import_export_keys_subpage;

pub use self::{
    ignored_users_subpage::IgnoredUsersSubpage,
    import_export_keys_subpage::{ImportExportKeysSubpage, ImportExportKeysSubpageMode},
};
use crate::{
    components::ButtonCountRow,
    session::model::{CryptoIdentityState, RecoveryState, Session, SessionVerificationState},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/security_page/mod.ui"
    )]
    #[properties(wrapper_type = super::SecurityPage)]
    pub struct SecurityPage {
        #[template_child]
        pub public_read_receipts_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub typing_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub ignored_users_row: TemplateChild<ButtonCountRow>,
        #[template_child]
        pub crypto_identity_row: TemplateChild<adw::PreferencesRow>,
        #[template_child]
        pub crypto_identity_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub crypto_identity_description: TemplateChild<gtk::Label>,
        #[template_child]
        pub crypto_identity_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub recovery_row: TemplateChild<adw::PreferencesRow>,
        #[template_child]
        pub recovery_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub recovery_description: TemplateChild<gtk::Label>,
        #[template_child]
        pub recovery_btn: TemplateChild<gtk::Button>,
        /// The current session.
        #[property(get, set = Self::set_session, nullable)]
        pub session: glib::WeakRef<Session>,
        ignored_users_count_handler: RefCell<Option<glib::SignalHandlerId>>,
        security_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        bindings: RefCell<Vec<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SecurityPage {
        const NAME: &'static str = "SecurityPage";
        type Type = super::SecurityPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SecurityPage {
        fn dispose(&self) {
            if let Some(session) = self.session.upgrade() {
                if let Some(handler) = self.ignored_users_count_handler.take() {
                    session.ignored_users().disconnect(handler);
                }

                let security = session.security();
                for handler in self.security_handlers.take() {
                    security.disconnect(handler);
                }
            }

            for binding in self.bindings.take() {
                binding.unbind();
            }
        }
    }

    impl WidgetImpl for SecurityPage {}
    impl PreferencesPageImpl for SecurityPage {}

    impl SecurityPage {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            let prev_session = self.session.upgrade();

            if prev_session.as_ref() == session {
                return;
            }
            let obj = self.obj();

            if let Some(session) = prev_session {
                if let Some(handler) = self.ignored_users_count_handler.take() {
                    session.ignored_users().disconnect(handler);
                }

                let security = session.security();
                for handler in self.security_handlers.take() {
                    security.disconnect(handler);
                }
            }
            for binding in self.bindings.take() {
                binding.unbind();
            }

            if let Some(session) = session {
                let ignored_users = session.ignored_users();
                let ignored_users_count_handler = ignored_users.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |ignored_users, _, _, _| {
                        imp.ignored_users_row
                            .set_count(ignored_users.n_items().to_string());
                    }
                ));
                self.ignored_users_row
                    .set_count(ignored_users.n_items().to_string());

                self.ignored_users_count_handler
                    .replace(Some(ignored_users_count_handler));

                let session_settings = session.settings();

                let public_read_receipts_binding = session_settings
                    .bind_property(
                        "public-read-receipts-enabled",
                        &*self.public_read_receipts_row,
                        "active",
                    )
                    .bidirectional()
                    .sync_create()
                    .build();
                let typing_binding = session_settings
                    .bind_property("typing-enabled", &*self.typing_row, "active")
                    .bidirectional()
                    .sync_create()
                    .build();

                self.bindings
                    .replace(vec![public_read_receipts_binding, typing_binding]);

                let security = session.security();
                let crypto_identity_state_handler =
                    security.connect_crypto_identity_state_notify(clone!(
                        #[weak]
                        obj,
                        move |_| {
                            obj.update_crypto_identity();
                        }
                    ));
                let verification_state_handler =
                    security.connect_verification_state_notify(clone!(
                        #[weak]
                        obj,
                        move |_| {
                            obj.update_crypto_identity();
                        }
                    ));
                let recovery_state_handler = security.connect_recovery_state_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_recovery();
                    }
                ));

                self.security_handlers.replace(vec![
                    crypto_identity_state_handler,
                    verification_state_handler,
                    recovery_state_handler,
                ]);
            }

            self.session.set(session);

            obj.update_crypto_identity();
            obj.update_recovery();

            obj.notify_session();
        }
    }
}

glib::wrapper! {
    /// Security settings page.
    pub struct SecurityPage(ObjectSubclass<imp::SecurityPage>)
        @extends gtk::Widget, adw::PreferencesPage, @implements gtk::Accessible;
}

impl SecurityPage {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Update the crypto identity section.
    fn update_crypto_identity(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();
        let security = session.security();

        let crypto_identity_state = security.crypto_identity_state();
        if matches!(
            crypto_identity_state,
            CryptoIdentityState::Unknown | CryptoIdentityState::Missing
        ) {
            imp.crypto_identity_icon
                .set_icon_name(Some("verified-danger-symbolic"));
            imp.crypto_identity_icon.remove_css_class("success");
            imp.crypto_identity_icon.remove_css_class("warning");
            imp.crypto_identity_icon.add_css_class("error");

            imp.crypto_identity_row
                .set_title(&gettext("No Crypto Identity"));
            imp.crypto_identity_description.set_label(&gettext(
                "Verifying your own devices or other users is not possible",
            ));

            imp.crypto_identity_btn.set_label(&gettext("Enable…"));
            imp.crypto_identity_btn
                .update_property(&[gtk::accessible::Property::Label(&gettext(
                    "Enable Crypto Identity",
                ))]);
            imp.crypto_identity_btn.add_css_class("suggested-action");

            return;
        }

        let verification_state = security.verification_state();
        if verification_state == SessionVerificationState::Verified {
            imp.crypto_identity_icon
                .set_icon_name(Some("verified-symbolic"));
            imp.crypto_identity_icon.add_css_class("success");
            imp.crypto_identity_icon.remove_css_class("warning");
            imp.crypto_identity_icon.remove_css_class("error");

            imp.crypto_identity_row
                .set_title(&gettext("Crypto Identity Enabled"));
            imp.crypto_identity_description.set_label(&gettext(
                "The crypto identity exists and this device is verified",
            ));

            imp.crypto_identity_btn.set_label(&gettext("Reset…"));
            imp.crypto_identity_btn
                .update_property(&[gtk::accessible::Property::Label(&gettext(
                    "Reset Crypto Identity",
                ))]);
            imp.crypto_identity_btn.remove_css_class("suggested-action");
        } else {
            imp.crypto_identity_icon
                .set_icon_name(Some("verified-warning-symbolic"));
            imp.crypto_identity_icon.remove_css_class("success");
            imp.crypto_identity_icon.add_css_class("warning");
            imp.crypto_identity_icon.remove_css_class("error");

            imp.crypto_identity_row
                .set_title(&gettext("Crypto Identity Incomplete"));
            imp.crypto_identity_description.set_label(&gettext(
                "The crypto identity exists but this device is not verified",
            ));

            imp.crypto_identity_btn.set_label(&gettext("Verify…"));
            imp.crypto_identity_btn
                .update_property(&[gtk::accessible::Property::Label(&gettext(
                    "Verify This Session",
                ))]);
            imp.crypto_identity_btn.add_css_class("suggested-action");
        }
    }

    /// Update the recovery section.
    fn update_recovery(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();

        let recovery_state = session.security().recovery_state();
        match recovery_state {
            RecoveryState::Unknown | RecoveryState::Disabled => {
                imp.recovery_icon.set_icon_name(Some("sync-off-symbolic"));
                imp.recovery_icon.remove_css_class("success");
                imp.recovery_icon.remove_css_class("warning");
                imp.recovery_icon.add_css_class("error");

                imp.recovery_row
                    .set_title(&gettext("Account Recovery Disabled"));
                imp.recovery_description.set_label(&gettext(
                    "Enable recovery to be able to restore your account without another device",
                ));

                imp.recovery_btn.set_label(&gettext("Enable…"));
                imp.recovery_btn
                    .update_property(&[gtk::accessible::Property::Label(&gettext(
                        "Enable Account Recovery",
                    ))]);
                imp.recovery_btn.add_css_class("suggested-action");
            }
            RecoveryState::Enabled => {
                imp.recovery_icon.set_icon_name(Some("sync-on-symbolic"));
                imp.recovery_icon.add_css_class("success");
                imp.recovery_icon.remove_css_class("warning");
                imp.recovery_icon.remove_css_class("error");

                imp.recovery_row
                    .set_title(&gettext("Account Recovery Enabled"));
                imp.recovery_description.set_label(&gettext(
                    "Your signing keys and encryption keys are synchronized",
                ));

                imp.recovery_btn.set_label(&gettext("Reset…"));
                imp.recovery_btn
                    .update_property(&[gtk::accessible::Property::Label(&gettext(
                        "Reset Account Recovery Key",
                    ))]);
                imp.recovery_btn.remove_css_class("suggested-action");
            }
            RecoveryState::Incomplete => {
                imp.recovery_icon
                    .set_icon_name(Some("sync-partial-symbolic"));
                imp.recovery_icon.remove_css_class("success");
                imp.recovery_icon.add_css_class("warning");
                imp.recovery_icon.remove_css_class("error");

                imp.recovery_row
                    .set_title(&gettext("Account Recovery Incomplete"));
                imp.recovery_description.set_label(&gettext(
                    "Recover to synchronize your signing keys and encryption keys",
                ));

                imp.recovery_btn.set_label(&gettext("Recover…"));
                imp.recovery_btn
                    .update_property(&[gtk::accessible::Property::Label(&gettext(
                        "Recover Account Data",
                    ))]);
                imp.recovery_btn.add_css_class("suggested-action");
            }
        }
    }
}

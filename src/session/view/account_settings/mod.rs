use adw::{prelude::*, subclass::prelude::*};
use gtk::{
    glib,
    glib::{clone, closure_local},
    CompositeTemplate,
};
use tracing::error;
use url::Url;

mod general_page;
mod notifications_page;
mod security_page;
mod user_sessions_page;

use self::{
    general_page::{ChangePasswordSubpage, DeactivateAccountSubpage, GeneralPage, LogOutSubpage},
    notifications_page::NotificationsPage,
    security_page::{
        IgnoredUsersSubpage, ImportExportKeysSubpage, ImportExportKeysSubpageMode, SecurityPage,
    },
    user_sessions_page::{UserSessionSubpage, UserSessionsPage},
};
use crate::{
    components::crypto::{CryptoIdentitySetupView, CryptoRecoverySetupView},
    session::model::Session,
    spawn,
    utils::BoundObjectWeakRef,
};

/// A subpage of the account settings.
#[derive(Debug, Clone, Copy, Eq, PartialEq, glib::Variant, strum::AsRefStr)]
pub(crate) enum AccountSettingsSubpage {
    /// A form to change the account's password.
    ChangePassword,
    /// A page to confirm the logout.
    LogOut,
    /// A page to confirm the deactivation of the password.
    DeactivateAccount,
    /// The list of ignored users.
    IgnoredUsers,
    /// A form to import encryption keys.
    ImportKeys,
    /// A form to export encryption keys.
    ExportKeys,
    /// The crypto identity setup view.
    CryptoIdentitySetup,
    /// The recovery setup view.
    RecoverySetup,
}

mod imp {
    use std::{cell::RefCell, sync::LazyLock};

    use glib::subclass::{InitializingObject, Signal};

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/account_settings/mod.ui")]
    #[properties(wrapper_type = super::AccountSettings)]
    pub struct AccountSettings {
        #[template_child]
        general_page: TemplateChild<GeneralPage>,
        #[template_child]
        sessions_page: TemplateChild<UserSessionsPage>,
        #[template_child]
        security_page: TemplateChild<SecurityPage>,
        /// The current session.
        #[property(get, set = Self::set_session, nullable)]
        session: BoundObjectWeakRef<Session>,
        /// The account management URL of the OIDC authentication issuer, if
        /// any.
        account_management_url: RefCell<Option<Url>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AccountSettings {
        const NAME: &'static str = "AccountSettings";
        type Type = super::AccountSettings;
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            NotificationsPage::ensure_type();

            Self::bind_template(klass);

            klass.install_action(
                "account-settings.show-subpage",
                Some(&AccountSettingsSubpage::static_variant_type()),
                |obj, _, param| {
                    let subpage = param
                        .and_then(glib::Variant::get::<AccountSettingsSubpage>)
                        .expect("The parameter should be a valid subpage name");

                    obj.show_subpage(subpage);
                },
            );

            klass.install_action(
                "account-settings.show-session-subpage",
                Some(&String::static_variant_type()),
                |obj, _, param| {
                    obj.show_session_subpage(
                        &param
                            .and_then(glib::Variant::get::<String>)
                            .expect("The parameter should be a string"),
                    );
                },
            );

            klass.install_action_async(
                "account-settings.reload-user-sessions",
                None,
                |obj, _, _| async move {
                    obj.imp().reload_user_sessions().await;
                },
            );

            klass.install_action("account-settings.close", None, |obj, _, _| {
                obj.close();
            });

            klass.install_action("account-settings.close-subpage", None, |obj, _, _| {
                obj.pop_subpage();
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AccountSettings {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("account-management-url-changed").build()]);
            SIGNALS.as_ref()
        }
    }

    impl WidgetImpl for AccountSettings {}
    impl AdwDialogImpl for AccountSettings {}
    impl PreferencesDialogImpl for AccountSettings {}

    impl AccountSettings {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            if self.session.obj() == session {
                return;
            }
            let obj = self.obj();

            self.session.disconnect_signals();
            self.set_account_management_url(None);

            if let Some(session) = session {
                let logged_out_handler = session.connect_logged_out(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.close();
                    }
                ));
                self.session.set(&session, vec![logged_out_handler]);

                // Refresh the list of sessions.
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.reload_user_sessions().await;
                    }
                ));

                // Load the account management URL.
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[weak]
                    session,
                    async move {
                        let account_management_url = session.account_management_url().await;
                        imp.set_account_management_url(account_management_url.cloned());
                    }
                ));
            }

            obj.notify_session();
        }

        /// Set the account management URL of the OIDC authentication issuer.
        fn set_account_management_url(&self, url: Option<Url>) {
            if *self.account_management_url.borrow() == url {
                return;
            }

            self.account_management_url.replace(url);
            self.obj()
                .emit_by_name::<()>("account-management-url-changed", &[]);
        }

        /// The account management URL of the OIDC authentication issuer, if
        /// any.
        pub(super) fn account_management_url(&self) -> Option<Url> {
            self.account_management_url.borrow().clone()
        }

        /// Reload the sessions from the server.
        async fn reload_user_sessions(&self) {
            let Some(session) = self.session.obj() else {
                return;
            };

            session.user_sessions().load().await;
        }
    }
}

glib::wrapper! {
    /// Preference window to display and update account settings.
    pub struct AccountSettings(ObjectSubclass<imp::AccountSettings>)
        @extends gtk::Widget, adw::Dialog, adw::PreferencesDialog, @implements gtk::Accessible;
}

impl AccountSettings {
    /// Construct new `AccountSettings` for the given session.
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// The account management URL of the OIDC authentication issuer, if any.
    fn account_management_url(&self) -> Option<Url> {
        self.imp().account_management_url()
    }

    /// Show the given subpage.
    pub(crate) fn show_subpage(&self, subpage: AccountSettingsSubpage) {
        let Some(session) = self.session() else {
            return;
        };

        let page: adw::NavigationPage = match subpage {
            AccountSettingsSubpage::ChangePassword => ChangePasswordSubpage::new(&session).upcast(),
            AccountSettingsSubpage::LogOut => LogOutSubpage::new(&session).upcast(),
            AccountSettingsSubpage::DeactivateAccount => {
                DeactivateAccountSubpage::new(&session, self).upcast()
            }
            AccountSettingsSubpage::IgnoredUsers => IgnoredUsersSubpage::new(&session).upcast(),
            AccountSettingsSubpage::ImportKeys => {
                ImportExportKeysSubpage::new(&session, ImportExportKeysSubpageMode::Import).upcast()
            }
            AccountSettingsSubpage::ExportKeys => {
                ImportExportKeysSubpage::new(&session, ImportExportKeysSubpageMode::Export).upcast()
            }
            AccountSettingsSubpage::CryptoIdentitySetup => {
                let view = CryptoIdentitySetupView::new(&session);
                view.connect_completed(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    move |_, _| {
                        obj.pop_subpage();
                    }
                ));

                let page = adw::NavigationPage::builder()
                    .tag(AccountSettingsSubpage::CryptoIdentitySetup.as_ref())
                    .child(&view)
                    .build();
                page.connect_shown(clone!(
                    #[weak]
                    view,
                    move |_| {
                        view.grab_focus();
                    }
                ));

                page
            }
            AccountSettingsSubpage::RecoverySetup => {
                let view = CryptoRecoverySetupView::new(&session);
                view.connect_completed(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    move |_| {
                        obj.pop_subpage();
                    }
                ));

                let page = adw::NavigationPage::builder()
                    .tag(AccountSettingsSubpage::RecoverySetup.as_ref())
                    .child(&view)
                    .build();
                page.connect_shown(clone!(
                    #[weak]
                    view,
                    move |_| {
                        view.grab_focus();
                    }
                ));

                page
            }
        };

        self.push_subpage(&page);
    }

    /// Show a subpage with the session details of the given session ID.
    pub(crate) fn show_session_subpage(&self, device_id: &str) {
        let Some(session) = self.session() else {
            return;
        };

        let user_session = session.user_sessions().get(&device_id.into());

        let Some(user_session) = user_session else {
            error!("ID {device_id} is not associated to any device");
            return;
        };

        let page = UserSessionSubpage::new(&user_session, self);

        self.push_subpage(&page);
    }

    /// Connect to the signal emitted when the account management URL changed.
    pub fn connect_account_management_url_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "account-management-url-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

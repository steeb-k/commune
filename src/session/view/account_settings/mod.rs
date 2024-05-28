use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};

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
    user_sessions_page::UserSessionsPage,
};
use crate::{
    components::crypto::{CryptoIdentitySetupView, CryptoRecoverySetupView},
    session::model::Session,
    utils::BoundObjectWeakRef,
};

/// A subpage of the account settings.
#[derive(Debug, Clone, Copy, Eq, PartialEq, glib::Variant, strum::AsRefStr)]
pub enum AccountSettingsSubpage {
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
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            UserSessionsPage::ensure_type();
            GeneralPage::ensure_type();
            NotificationsPage::ensure_type();
            SecurityPage::ensure_type();

            Self::bind_template(klass);

            klass.install_action(
                "account-settings.show-subpage",
                Some(&AccountSettingsSubpage::static_variant_type()),
                |obj, _, param| {
                    let subpage = param
                        .and_then(|variant| variant.get::<AccountSettingsSubpage>())
                        .expect("The parameter should be a valid subpage name");

                    obj.show_subpage(subpage);
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
    impl ObjectImpl for AccountSettings {}

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
        @extends gtk::Widget, adw::Dialog, adw::PreferencesDialog, @implements gtk::Accessible;
}

impl AccountSettings {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Show the given subpage.
    pub fn show_subpage(&self, subpage: AccountSettingsSubpage) {
        let Some(session) = self.session() else {
            return;
        };

        let page: adw::NavigationPage = match subpage {
            AccountSettingsSubpage::ChangePassword => ChangePasswordSubpage::new(&session).upcast(),
            AccountSettingsSubpage::LogOut => LogOutSubpage::new(&session).upcast(),
            AccountSettingsSubpage::DeactivateAccount => {
                DeactivateAccountSubpage::new(&session).upcast()
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
                view.connect_completed(clone!(@weak self as obj => move |_, _| {
                    obj.pop_subpage();
                }));

                let page = adw::NavigationPage::builder()
                    .tag(AccountSettingsSubpage::CryptoIdentitySetup.as_ref())
                    .child(&view)
                    .build();
                page.connect_shown(clone!(@weak view => move |_| {
                    view.grab_focus();
                }));

                page
            }
            AccountSettingsSubpage::RecoverySetup => {
                let view = CryptoRecoverySetupView::new(&session);
                view.connect_completed(clone!(@weak self as obj => move |_| {
                    obj.pop_subpage();
                }));

                let page = adw::NavigationPage::builder()
                    .tag(AccountSettingsSubpage::RecoverySetup.as_ref())
                    .child(&view)
                    .build();
                page.connect_shown(clone!(@weak view => move |_| {
                    view.grab_focus();
                }));

                page
            }
        };

        self.push_subpage(&page);
    }
}

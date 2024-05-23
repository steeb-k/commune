use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::closure_local, CompositeTemplate};
use matrix_sdk::encryption::{
    recovery::{RecoveryError, RecoveryState as SdkRecoveryState},
    secret_storage::SecretStorageError,
};
use tracing::{debug, error};

use crate::{
    components::{AuthDialog, AuthError, LoadingButton},
    session::model::{RecoveryState, Session},
    spawn_tokio, toast,
};

/// A page of the [`CryptoRecoverySetupView`] navigation stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::AsRefStr)]
#[strum(serialize_all = "kebab-case")]
enum CryptoRecoverySetupPage {
    /// Use account recovery.
    Recover,
    /// Reset the recovery and optionally the cross-signing.
    Reset,
    /// Enable recovery.
    Enable,
    /// The recovery was successfully enabled.
    Success,
    /// The recovery was successful but is still incomplete.
    Incomplete,
}

/// The initial page of the [`CryptoRecoverySetupView`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum, strum::AsRefStr)]
#[enum_type(name = "CryptoRecoverySetupInitialPage")]
#[strum(serialize_all = "kebab-case")]
pub enum CryptoRecoverySetupInitialPage {
    /// Use account recovery.
    #[default]
    Recover,
    /// Reset the account recovery recovery.
    Reset,
    /// Enable recovery.
    Enable,
}

mod imp {
    use std::cell::Cell;

    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/crypto/recovery_setup_view.ui")]
    #[properties(wrapper_type = super::CryptoRecoverySetupView)]
    pub struct CryptoRecoverySetupView {
        #[template_child]
        pub navigation: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub recover_entry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub recover_btn: TemplateChild<LoadingButton>,
        #[template_child]
        pub reset_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub reset_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub reset_description: TemplateChild<gtk::Label>,
        #[template_child]
        pub reset_entry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub reset_btn: TemplateChild<LoadingButton>,
        #[template_child]
        pub enable_entry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub enable_btn: TemplateChild<LoadingButton>,
        #[template_child]
        pub success_description: TemplateChild<gtk::Label>,
        #[template_child]
        pub success_key_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub success_key_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub success_key_copy_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub success_confirm_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub incomplete_confirm_btn: TemplateChild<gtk::Button>,
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// Whether resetting should also reset the crypto identity.
        #[property(get, set = Self::set_reset_identity, construct_only)]
        pub reset_identity: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CryptoRecoverySetupView {
        const NAME: &'static str = "CryptoRecoverySetupView";
        type Type = super::CryptoRecoverySetupView;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("setup-view");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CryptoRecoverySetupView {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![
                    // Recovery is enabled.
                    Signal::builder("completed").build(),
                ]
            });
            SIGNALS.as_ref()
        }
    }

    impl WidgetImpl for CryptoRecoverySetupView {
        fn grab_focus(&self) -> bool {
            match self.visible_page() {
                CryptoRecoverySetupPage::Recover => self.recover_entry.grab_focus(),
                CryptoRecoverySetupPage::Reset => self.reset_entry.grab_focus(),
                CryptoRecoverySetupPage::Enable => self.enable_entry.grab_focus(),
                CryptoRecoverySetupPage::Success => self.success_confirm_btn.grab_focus(),
                CryptoRecoverySetupPage::Incomplete => self.incomplete_confirm_btn.grab_focus(),
            }
        }
    }

    impl BinImpl for CryptoRecoverySetupView {}

    impl CryptoRecoverySetupView {
        /// The visible page of the view.
        fn visible_page(&self) -> CryptoRecoverySetupPage {
            self.navigation
                .visible_page()
                .and_then(|p| p.tag())
                .and_then(|t| t.as_str().try_into().ok())
                .unwrap()
        }

        /// Set the current session.
        fn set_session(&self, session: &Session) {
            self.session.set(Some(session));

            let recovery_state = session.recovery_state();
            let initial_page = match recovery_state {
                RecoveryState::Unknown | RecoveryState::Disabled => {
                    CryptoRecoverySetupInitialPage::Enable
                }
                RecoveryState::Enabled => CryptoRecoverySetupInitialPage::Reset,
                RecoveryState::Incomplete => CryptoRecoverySetupInitialPage::Recover,
            };

            self.set_initial_page(initial_page);
        }

        /// Set whether resetting should also reset the crypto identity.
        fn set_reset_identity(&self, reset: bool) {
            self.reset_identity.set(reset);

            let title = if reset {
                gettext("Reset Crypto Identity and Account Recovery Key")
            } else {
                gettext("Reset Account Recovery Key")
            };
            self.reset_title.set_label(&title);
            self.reset_page.set_title(&title);

            let description = if reset {
                gettext("This will invalidate the verifications of all users and sessions, and you might not be able to read your encrypted messages anymore.")
            } else {
                gettext("You might not be able to read your encrypted messages anymore.")
            };
            self.reset_description.set_label(&description);
        }

        /// Set the initial page of this view.
        pub(super) fn set_initial_page(&self, initial_page: CryptoRecoverySetupInitialPage) {
            self.navigation.replace_with_tags(&[initial_page.as_ref()]);
        }

        /// Update the success page for the given recovery key.
        pub(super) fn update_success(&self, key: Option<String>) {
            let has_key = key.is_some();

            let description = if has_key {
                gettext("Make sure to store this recovery key in a safe place. You will need it to recover your account if you lose access to all your sessions.")
            } else {
                gettext("Make sure to remember your passphrase or to store it in a safe place. You will need it to recover your account if you lose access to all your sessions.")
            };
            self.success_description.set_label(&description);

            if let Some(key) = key {
                self.success_key_label.set_label(&key);
            }
            self.success_key_box.set_visible(has_key);
        }
    }
}

glib::wrapper! {
    /// A view with the different flows to use or set up account recovery.
    pub struct CryptoRecoverySetupView(ObjectSubclass<imp::CryptoRecoverySetupView>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl CryptoRecoverySetupView {
    pub fn new(session: &Session, reset_identity: bool) -> Self {
        glib::Object::builder()
            .property("session", session)
            .property("reset-identity", reset_identity)
            .build()
    }

    /// Set the initial page of this view.
    pub fn set_initial_page(&self, initial_page: CryptoRecoverySetupInitialPage) {
        self.imp().set_initial_page(initial_page);
    }

    /// Focus the proper widget for the current page.
    #[template_callback]
    fn grab_focus(&self) {
        self.imp().grab_focus();
    }

    /// The content of the recover entry changed.
    #[template_callback]
    fn recover_entry_changed(&self) {
        let imp = self.imp();

        let can_recover = !imp.recover_entry.text().is_empty();
        imp.recover_btn.set_sensitive(can_recover);
    }

    /// Recover the data.
    #[template_callback]
    async fn recover(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let imp = self.imp();
        let key = imp.recover_entry.text();

        if key.is_empty() {
            return;
        }

        imp.recover_btn.set_is_loading(true);

        let encryption = session.client().encryption();
        let recovery = encryption.recovery();
        let handle = spawn_tokio!(async move { recovery.recover(&key).await });

        match handle.await.unwrap() {
            Ok(_) => {
                // Even if recovery was successful, the recovery data may not have been
                // complete. Because the SDK uses multiple threads, we are only
                // sure of the SDK's recovery state at this point, not the Session's.
                if encryption.recovery().state() == SdkRecoveryState::Incomplete {
                    imp.navigation
                        .push_by_tag(CryptoRecoverySetupPage::Incomplete.as_ref());
                } else {
                    self.emit_completed();
                }
            }
            Err(error) => {
                error!("Could not recover account: {error}");

                match error {
                    RecoveryError::SecretStorage(SecretStorageError::SecretStorageKey(_)) => {
                        toast!(self, gettext("The recovery passphrase or key is invalid"));
                    }
                    _ => {
                        toast!(self, gettext("Could not access recovery data"));
                    }
                }
            }
        }

        imp.recover_btn.set_is_loading(false);
    }

    /// Reset recovery and optionally cross-signing.
    #[template_callback]
    async fn reset(&self) {
        let imp = self.imp();

        imp.reset_btn.set_is_loading(true);

        if self.reset_identity() && self.bootstrap_cross_signing().await.is_err() {
            imp.reset_btn.set_is_loading(false);
            return;
        }

        let passphrase = imp.reset_entry.text();
        self.reset_recovery(passphrase).await;

        imp.reset_btn.set_is_loading(false);
    }

    async fn bootstrap_cross_signing(&self) -> Result<(), ()> {
        let Some(session) = self.session() else {
            return Err(());
        };

        let dialog = AuthDialog::new(&session);

        let result = dialog
            .authenticate(self, move |client, auth| async move {
                client.encryption().bootstrap_cross_signing(auth).await
            })
            .await;

        match result {
            Ok(_) => Ok(()),
            Err(AuthError::UserCancelled) => {
                debug!("User cancelled authentication for cross-signing bootstrap");
                Err(())
            }
            Err(error) => {
                error!("Could not bootstrap cross-signing: {error:?}");
                toast!(self, gettext("Could not create the crypto identity",));
                Err(())
            }
        }
    }

    async fn reset_recovery(&self, passphrase: glib::GString) {
        let Some(session) = self.session() else {
            return;
        };

        let passphrase = Some(passphrase).filter(|s| !s.is_empty());
        let has_passphrase = passphrase.is_some();

        let recovery = session.client().encryption().recovery();
        let handle = spawn_tokio!(async move {
            let mut reset = recovery.reset_key();
            if let Some(passphrase) = passphrase.as_deref() {
                reset = reset.with_passphrase(passphrase);
            }

            reset.await
        });

        match handle.await.unwrap() {
            Ok(key) => {
                let imp = self.imp();
                let key = if has_passphrase { None } else { Some(key) };

                imp.update_success(key);
                imp.navigation
                    .push_by_tag(CryptoRecoverySetupPage::Success.as_ref());
            }
            Err(error) => {
                error!("Could not reset account recovery key: {error}");
                toast!(self, gettext("Could not reset account recovery key"));
            }
        }
    }

    /// Enable recovery.
    #[template_callback]
    async fn enable(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();

        imp.enable_btn.set_is_loading(true);

        let passphrase = Some(imp.enable_entry.text()).filter(|s| !s.is_empty());
        let has_passphrase = passphrase.is_some();

        let recovery = session.client().encryption().recovery();
        let handle = spawn_tokio!(async move {
            let mut enable = recovery.enable();
            if let Some(passphrase) = passphrase.as_deref() {
                enable = enable.with_passphrase(passphrase);
            }

            enable.await
        });

        match handle.await.unwrap() {
            Ok(key) => {
                let key = if has_passphrase { None } else { Some(key) };

                imp.update_success(key);
                imp.navigation
                    .push_by_tag(CryptoRecoverySetupPage::Success.as_ref());
            }
            Err(error) => {
                error!("Could not enable account recovery: {error}");
                toast!(self, gettext("Could not enable account recovery"));
            }
        }

        imp.enable_btn.set_is_loading(false);
    }

    /// Copy the recovery key to the clipboard.
    #[template_callback]
    fn copy_key(&self) {
        let key = self.imp().success_key_label.label();

        let clipboard = self.clipboard();
        clipboard.set_text(&key);

        toast!(self, "Recovery key copied to clipboard");
    }

    // Emit the `completed` signal.
    #[template_callback]
    fn emit_completed(&self) {
        self.emit_by_name::<()>("completed", &[]);
    }

    /// Connect to the signal emitted when the recovery was successfully
    /// enabled.
    pub fn connect_completed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "completed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

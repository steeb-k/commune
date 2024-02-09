use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    CompositeTemplate,
};
use tracing::{debug, error};

use super::IdentityVerificationView;
use crate::{
    components::{AuthDialog, AuthError, SpinnerButton},
    session::model::{IdentityVerification, Session},
    spawn, spawn_tokio, toast,
    utils::BoundObjectWeakRef,
};

/// The state of the cross-signing identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "SessionVerificationIdentityState")]
pub enum IdentityState {
    /// It does not exist.
    #[default]
    Missing,
    /// There are no verified sessions.
    NoSessions,
    /// We should be able to verify this session with another session.
    CanVerify,
}

mod imp {
    use std::cell::Cell;

    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/verification_view/session_verification_view.ui")]
    #[properties(wrapper_type = super::SessionVerificationView)]
    pub struct SessionVerificationView {
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// The ongoing identity verification, if any.
        #[property(get)]
        pub verification: BoundObjectWeakRef<IdentityVerification>,
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub send_request_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub choose_bootstrap_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub bootstrap_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub bootstrap_setup_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub verification_page: TemplateChild<IdentityVerificationView>,
        /// The state of the cross-signing identity.
        #[property(get, builder(IdentityState::default()))]
        pub identity_state: Cell<IdentityState>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionVerificationView {
        const NAME: &'static str = "SessionVerificationView";
        type Type = super::SessionVerificationView;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("session-verification");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SessionVerificationView {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![
                    // The session verification was completed.
                    Signal::builder("completed").build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            self.main_stack.connect_transition_running_notify(
                clone!(@weak self as imp => move |stack|
                    if !stack.is_transition_running() {
                        // Focus the default widget when the transition has ended.
                        imp.grab_focus();
                    }
                ),
            );
        }

        fn dispose(&self) {
            if let Some(verification) = self.verification.obj() {
                spawn!(clone!(@strong verification => async move {
                    let _ = verification.cancel().await;
                }));
            }
        }
    }

    impl WidgetImpl for SessionVerificationView {
        fn grab_focus(&self) -> bool {
            let Some(name) = self.main_stack.visible_child_name() else {
                return false;
            };

            match name.as_str() {
                "choose-method" => {
                    if self.send_request_btn.is_visible() {
                        self.send_request_btn.grab_focus()
                    } else {
                        self.choose_bootstrap_btn.grab_focus()
                    }
                }
                "verification" => self.verification_page.grab_focus(),
                "bootstrap" => self.bootstrap_setup_btn.grab_focus(),
                _ => false,
            }
        }
    }

    impl BinImpl for SessionVerificationView {}

    impl SessionVerificationView {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            self.session.set(session.as_ref());

            spawn!(clone!(@weak self as imp => async move {
                imp.load().await;
            }));
        }

        /// Set the state of the cross-signing identity.
        fn set_identity_state(&self, state: IdentityState) {
            self.identity_state.set(state);

            let obj = self.obj();
            obj.notify_identity_state();
            obj.update_bootstrap_page();
        }

        /// Load the cross-signing state for the current page.
        async fn load(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let client = session.client();

            let client_clone = client.clone();
            let user_identity_handle = spawn_tokio!(async move {
                let user_id = client_clone.user_id().unwrap();
                client_clone.encryption().get_user_identity(user_id).await
            });

            let has_identity = match user_identity_handle.await.unwrap() {
                Ok(Some(_)) => true,
                Ok(None) => {
                    debug!("No encryption user identity found");
                    false
                }
                Err(error) => {
                    error!("Failed to get encryption user identity: {error}");
                    false
                }
            };

            if !has_identity {
                self.set_identity_state(IdentityState::Missing);
                self.obj().show_bootstrap();
                return;
            }

            let devices_handle = spawn_tokio!(async move {
                let user_id = client.user_id().unwrap();
                client.encryption().get_user_devices(user_id).await
            });

            let has_sessions = match devices_handle.await.unwrap() {
                Ok(devices) => devices.devices().any(|d| d.is_cross_signed_by_owner()),
                Err(error) => {
                    error!("Failed to get user devices: {error}");
                    // If there are actually no other devices, the user can still
                    // reset the cross-signing identity.
                    true
                }
            };

            if !has_sessions {
                self.set_identity_state(IdentityState::NoSessions);
                self.obj().show_bootstrap();
                return;
            }

            self.set_identity_state(IdentityState::CanVerify);

            // Use received verification requests too.
            let verification_list = session.verification_list();
            verification_list.connect_items_changed(
                clone!(@weak self as imp => move |verification_list, _, _, _| {
                    if imp.verification.obj().is_some() {
                        // We don't want to override the current verification.
                        return;
                    }

                    if let Some(verification) = verification_list.ongoing_session_verification() {
                        imp.set_verification(Some(verification));
                    }
                }),
            );

            if let Some(verification) = verification_list.ongoing_session_verification() {
                self.set_verification(Some(verification));
            } else {
                self.obj().choose_method();
            }
        }

        /// Set the ongoing identity verification.
        ///
        /// Cancels the previous verification if it's not finished.
        pub fn set_verification(&self, verification: Option<IdentityVerification>) {
            let prev_verification = self.verification.obj();

            if prev_verification == verification {
                return;
            }
            let obj = self.obj();

            if let Some(verification) = prev_verification {
                if !verification.is_finished() {
                    spawn!(clone!(@strong verification => async move {
                        let _ = verification.cancel().await;
                    }));
                }

                self.verification.disconnect_signals();
            }

            if let Some(verification) = &verification {
                let replaced_handler = verification.connect_replaced(
                    clone!(@weak self as imp => move |_, new_verification| {
                        imp.set_verification(Some(new_verification.clone()));
                    }),
                );
                let done_handler = verification.connect_done(
                    clone!(@weak obj => @default-return glib::Propagation::Stop, move |verification| {
                        obj.emit_by_name::<()>("completed", &[]);
                        obj.imp().set_verification(None);
                        verification.remove_from_list();

                        glib::Propagation::Stop
                    }),
                );
                let remove_handler = verification.connect_dismiss(clone!(@weak obj => move |_| {
                    obj.choose_method();
                    obj.imp().set_verification(None);
                }));

                self.verification.set(
                    verification,
                    vec![replaced_handler, done_handler, remove_handler],
                );
            }

            let has_verification = verification.is_some();
            self.verification_page.set_verification(verification);

            if has_verification {
                obj.show_verification();
            }

            obj.notify_verification();
        }
    }
}

glib::wrapper! {
    /// A view with the different flows to verify a session.
    pub struct SessionVerificationView(ObjectSubclass<imp::SessionVerificationView>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl SessionVerificationView {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Reset the UI to its initial state.
    fn reset(&self) {
        let imp = self.imp();
        imp.bootstrap_setup_btn.set_loading(false);
        imp.send_request_btn.set_loading(false);
    }

    /// Show the page to choose a verification method.
    fn choose_method(&self) {
        self.reset();
        let imp = self.imp();
        imp.set_verification(None);
        imp.main_stack.set_visible_child_name("choose-method");
    }

    /// Show the verification flow.
    fn show_verification(&self) {
        self.imp().main_stack.set_visible_child_name("verification");
    }

    /// Show the recovery flow.
    #[template_callback]
    fn show_recovery(&self) {
        let imp = self.imp();
        imp.set_verification(None);
        imp.main_stack.set_visible_child_name("recovery");
    }

    /// Show the bootstrap page.
    #[template_callback]
    fn show_bootstrap(&self) {
        let imp = self.imp();
        imp.set_verification(None);
        imp.main_stack.set_visible_child_name("bootstrap");
    }

    /// Update the bootstrap page according to the current state.
    fn update_bootstrap_page(&self) {
        let identity_state = self.identity_state();

        let imp = self.imp();
        let label = &imp.bootstrap_label;
        let setup_btn = &imp.bootstrap_setup_btn;

        match identity_state {
            IdentityState::Missing => {
                label.set_label(&gettext(
                    "You need to set up an encryption identity, since it has never been created.",
                ));
                setup_btn.add_css_class("suggested-action");
                setup_btn.remove_css_class("destructive-action");
                setup_btn.set_label(&gettext("Set Up"));
            }
            IdentityState::NoSessions => {
                label.set_label(&gettext("No other sessions are available to verify this session. You can either restore cross-signing from another session and restart this process, or reset the encryption identity."));
                setup_btn.remove_css_class("suggested-action");
                setup_btn.add_css_class("destructive-action");
                setup_btn.set_label(&gettext("Reset"));
            }
            IdentityState::CanVerify => {
                label.set_label(&gettext("If you lost access to all other sessions, you can create a new encryption identity. Be careful because this will cancel the verifications of all users and sessions."));
                setup_btn.remove_css_class("suggested-action");
                setup_btn.add_css_class("destructive-action");
                setup_btn.set_label(&gettext("Reset"));
            }
        }
    }

    /// Go to the previous step.
    ///
    /// Return `true` if the action was handled, `false` if the stack cannot go
    /// back.
    pub fn go_previous(&self) -> bool {
        let imp = self.imp();
        let Some(page) = imp.main_stack.visible_child_name() else {
            return false;
        };

        match &*page {
            "verification" => {
                self.choose_method();
                true
            }
            "bootstrap" => {
                if self.identity_state() == IdentityState::CanVerify {
                    self.choose_method();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Create a new encryption user identity.
    #[template_callback]
    fn bootstrap_cross_signing(&self) {
        self.imp().bootstrap_setup_btn.set_loading(true);

        spawn!(clone!(@weak self as obj => async move {
            obj.bootstrap_cross_signing_inner().await;
        }));
    }

    async fn bootstrap_cross_signing_inner(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let dialog = AuthDialog::new(self.root().and_downcast_ref::<gtk::Window>(), &session);

        let result = dialog
            .authenticate(move |client, auth| async move {
                client.encryption().bootstrap_cross_signing(auth).await
            })
            .await;

        let error_message = match result {
            Ok(_) => None,
            Err(AuthError::UserCancelled) => {
                error!("Failed to bootstrap cross-signing: User cancelled the authentication");
                Some(gettext(
                    "You cancelled the authentication needed to create the encryption identity.",
                ))
            }
            Err(error) => {
                error!("Failed to bootstrap cross-signing: {error:?}");
                Some(gettext(
                    "An error occurred during the creation of the encryption identity.",
                ))
            }
        };

        if let Some(error_message) = error_message {
            toast!(self, error_message);
            self.reset();
        } else {
            self.emit_by_name::<()>("completed", &[]);
        }
    }

    /// Create a new verification request.
    #[template_callback]
    fn send_request(&self) {
        let Some(session) = self.session() else {
            return;
        };

        self.imp().send_request_btn.set_loading(true);

        spawn!(clone!(@weak self as obj, @weak session => async move {
            match session.verification_list().create(None).await {
                Ok(verification) => {
                    obj.imp().set_verification(Some(verification));
                    obj.show_verification();
                }
                Err(()) => {
                    toast!(obj, gettext("Failed to send a new verification request"));
                    obj.reset();
                }
            }
        }));
    }

    /// Connect to the signal emitted when the verification was completed.
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

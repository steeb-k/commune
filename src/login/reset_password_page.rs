use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::glib;
use ruma::{
    ClientSecret, OwnedClientSecret, OwnedServerName, OwnedSessionId,
    api::{
        client::{
            account::{change_password, request_password_change_token_via_email},
            uiaa::{AuthData, ThirdpartyIdCredentials},
        },
        error::{ErrorBody, ErrorKind, StandardErrorBody},
    },
};
use tracing::{error, warn};
use url::Url;

use super::Login;
use crate::{
    components::{LoadingButton, OfflineBanner},
    gettext_f,
    prelude::*,
    spawn_tokio, toast,
    utils::{
        matrix::validate_password,
        password::{draw_password_confirmation, draw_password_validity},
    },
};

/// The session a reset is happening in, as far as the homeserver is concerned.
#[derive(Debug, Clone)]
struct EmailSession {
    /// The address the link was sent to.
    address: String,
    /// The session ID the homeserver gave us for it.
    sid: OwnedSessionId,
    /// The secret that proves the session is ours.
    ///
    /// It is generated once per page rather than once per request, because it
    /// is what ties the `requestToken` call to the `password` call.
    client_secret: OwnedClientSecret,
    /// How many times the link has been asked for.
    ///
    /// The spec uses this to tell "send it again" apart from a retried
    /// request, so it must go up only when the user asks for another email.
    send_attempt: u32,
}

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/reset_password_page.ui")]
    #[properties(wrapper_type = super::LoginResetPasswordPage)]
    pub struct LoginResetPasswordPage {
        #[template_child]
        title: TemplateChild<gtk::Label>,
        #[template_child]
        homeserver_url: TemplateChild<gtk::Label>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        email_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        send_button: TemplateChild<LoadingButton>,
        #[template_child]
        sent_label: TemplateChild<gtk::Label>,
        #[template_child]
        password_entry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        password_progress: TemplateChild<gtk::LevelBar>,
        #[template_child]
        password_error_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        password_error: TemplateChild<gtk::Label>,
        #[template_child]
        confirm_password_entry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        confirm_password_error_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        confirm_password_error: TemplateChild<gtk::Label>,
        #[template_child]
        reset_button: TemplateChild<LoadingButton>,
        #[template_child]
        resend_button: TemplateChild<LoadingButton>,
        /// The parent `Login` object.
        #[property(get, set, nullable)]
        login: glib::WeakRef<Login>,
        /// The session the homeserver opened for this reset, once it has.
        session: RefCell<Option<EmailSession>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginResetPasswordPage {
        const NAME: &'static str = "LoginResetPasswordPage";
        type Type = super::LoginResetPasswordPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            LoadingButton::ensure_type();
            OfflineBanner::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LoginResetPasswordPage {
        fn constructed(&self) {
            self.parent_constructed();

            // The same offsets as every other password meter in the app.
            // Blueprint has no syntax for them.
            for (name, value) in [
                ("low", 1.0),
                ("step2", 2.0),
                ("step3", 3.0),
                ("high", 4.0),
                ("full", 5.0),
            ] {
                self.password_progress.add_offset_value(name, value);
            }
        }
    }

    impl WidgetImpl for LoginResetPasswordPage {
        fn grab_focus(&self) -> bool {
            if self.stack.visible_child_name().as_deref() == Some("password") {
                self.password_entry.grab_focus()
            } else {
                self.email_entry.grab_focus()
            }
        }
    }

    impl NavigationPageImpl for LoginResetPasswordPage {
        fn shown(&self) {
            self.grab_focus();
        }
    }

    #[gtk::template_callbacks]
    impl LoginResetPasswordPage {
        /// Update the homeserver URL displayed under the title.
        pub(super) fn update_title(
            &self,
            homeserver_url: &Url,
            server_name: Option<&OwnedServerName>,
        ) {
            let title = if let Some(server_name) = server_name {
                gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', this is a
                    // variable name.
                    "Reset your password on {domain_name}",
                    &[(
                        "domain_name",
                        &format!("<span segment=\"word\">{server_name}</span>"),
                    )],
                )
            } else {
                gettext("Reset Password")
            };
            self.title.set_markup(&title);

            let homeserver_url = homeserver_url.as_str().trim_end_matches('/');
            self.homeserver_url.set_label(homeserver_url);
        }

        /// Whether an email address has been typed at all.
        ///
        /// Nothing here validates the shape of an address: the homeserver is
        /// the only thing that knows which addresses it has, and a client that
        /// refuses an address the server would have accepted is worse than one
        /// that asks.
        fn can_send(&self) -> bool {
            !self.email_entry.text().trim().is_empty()
        }

        /// Update the state of the button that asks for the email.
        #[template_callback]
        fn update_send_state(&self) {
            self.send_button.set_sensitive(self.can_send());
        }

        /// Ask the homeserver to email a link.
        #[template_callback]
        async fn send_email(&self) {
            if !self.can_send() {
                return;
            }

            let Some(login) = self.login.upgrade() else {
                return;
            };
            let Some(client) = login.client().await else {
                return;
            };

            let address = self.email_entry.text().trim().to_owned();

            // A resend keeps the secret and bumps the attempt; a first send
            // makes both.
            let (client_secret, send_attempt) = match self.session.borrow().as_ref() {
                Some(session) if session.address == address => {
                    (session.client_secret.clone(), session.send_attempt + 1)
                }
                _ => (ClientSecret::new(), 1),
            };

            let is_resend = self.stack.visible_child_name().as_deref() == Some("password");
            let button = if is_resend {
                &self.resend_button
            } else {
                &self.send_button
            };
            button.set_is_loading(true);

            let request = request_password_change_token_via_email::v3::Request::new(
                client_secret.clone(),
                address.clone(),
                send_attempt.into(),
            );
            let handle = spawn_tokio!(async move { client.send(request).await });

            match handle.await.expect("task was not aborted") {
                Ok(response) => {
                    self.session.replace(Some(EmailSession {
                        address: address.clone(),
                        sid: response.sid,
                        client_secret,
                        send_attempt,
                    }));

                    self.sent_label.set_label(&gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}', this is
                        // a variable name.
                        "Open the link sent to {email}, then set a new password here.",
                        &[("email", &address)],
                    ));

                    self.stack.set_visible_child_name("password");
                    self.validate_password();
                    self.password_entry.grab_focus();
                }
                Err(error) => {
                    warn!("Could not request a password reset email: {error}");
                    toast!(self.obj(), reset_email_error(&error));
                }
            }

            button.set_is_loading(false);
        }

        /// Validate the new password, and show what is missing from it.
        #[template_callback]
        fn validate_password(&self) {
            draw_password_validity(
                &self.password_entry,
                &self.password_progress,
                &self.password_error_revealer,
                &self.password_error,
            );

            self.validate_password_confirmation();
        }

        /// Check that the password confirmation matches the password.
        #[template_callback]
        fn validate_password_confirmation(&self) {
            draw_password_confirmation(
                &self.password_entry.text(),
                &self.confirm_password_entry,
                &self.confirm_password_error_revealer,
                &self.confirm_password_error,
            );

            self.reset_button.set_sensitive(self.can_reset());
        }

        /// Whether the current state allows to reset the password.
        fn can_reset(&self) -> bool {
            let password = self.password_entry.text();

            self.session.borrow().is_some()
                && validate_password(&password).progress == 100
                && password == self.confirm_password_entry.text()
        }

        /// Set the new password, if the link has been opened.
        #[template_callback]
        async fn reset_password(&self) {
            if !self.can_reset() {
                return;
            }

            let Some(login) = self.login.upgrade() else {
                return;
            };
            let Some(client) = login.client().await else {
                return;
            };
            let Some(session) = self.session.borrow().clone() else {
                return;
            };

            let Some(auth) = email_identity_auth(&session) else {
                toast!(self.obj(), gettext("Could not reset password"));
                return;
            };

            self.reset_button.set_is_loading(true);

            let password = self.password_entry.text().to_string();
            let handle = spawn_tokio!(async move {
                let mut request = change_password::v3::Request::new(password);
                // The account has been out of the owner's hands for as long as
                // the password was unknown to them, so every other session
                // goes. This is the endpoint's own default, said out loud.
                request.logout_devices = true;
                request.auth = Some(auth);

                client.send(request).await
            });

            match handle.await.expect("task was not aborted") {
                Ok(_) => {
                    toast!(
                        self.obj(),
                        gettext("Password reset. Log in with the new one.")
                    );
                    login.pop_page();
                }
                Err(error) => {
                    if error.as_uiaa_response().is_some() {
                        // The homeserver is still waiting for the link to be
                        // opened. That is not a failure, it is a "not yet".
                        toast!(
                            self.obj(),
                            gettext("Open the link in the email first, then try again")
                        );
                    } else {
                        warn!("Could not reset the password: {error}");
                        toast!(self.obj(), error.to_user_facing());
                    }

                    self.reset_button.set_is_loading(false);
                }
            }
        }

        /// Reset this page.
        pub(super) fn clean(&self) {
            self.session.take();
            self.email_entry.set_text("");
            self.password_entry.set_text("");
            self.confirm_password_entry.set_text("");
            self.send_button.set_is_loading(false);
            self.resend_button.set_is_loading(false);
            self.reset_button.set_is_loading(false);
            self.stack.set_visible_child_name("email");
            self.update_send_state();
            self.validate_password();
        }
    }
}

glib::wrapper! {
    /// The login page to set a new password after forgetting one.
    pub struct LoginResetPasswordPage(ObjectSubclass<imp::LoginResetPasswordPage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl LoginResetPasswordPage {
    /// The tag for this page.
    pub(super) const TAG: &str = "reset-password";

    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Update this page with the given data.
    pub(super) fn update(&self, homeserver_url: &Url, server_name: Option<&OwnedServerName>) {
        self.imp().update_title(homeserver_url, server_name);
    }

    /// Reset this page.
    pub(super) fn clean(&self) {
        self.imp().clean();
    }
}

/// The authentication data proving the email address of the given session was
/// confirmed.
///
/// `EmailIdentity` is `non_exhaustive`, so it is built through `AuthData::new`
/// from the JSON the spec describes rather than from its fields.
fn email_identity_auth(session: &EmailSession) -> Option<AuthData> {
    let credentials =
        ThirdpartyIdCredentials::new(session.sid.clone(), session.client_secret.clone());

    let credentials = match serde_json::to_value(&credentials) {
        Ok(credentials) => credentials,
        Err(error) => {
            error!("Could not serialize the third-party identifier credentials: {error}");
            return None;
        }
    };

    let mut data = serde_json::Map::new();
    data.insert("threepid_creds".to_owned(), credentials);

    match AuthData::new("m.login.email.identity", None, data) {
        Ok(auth) => Some(auth),
        Err(error) => {
            error!("Could not construct the email identity authentication data: {error}");
            None
        }
    }
}

/// What to say about a homeserver that would not send the email.
fn reset_email_error(error: &matrix_sdk::HttpError) -> String {
    let Some(ErrorBody::Standard(StandardErrorBody { kind, .. })) =
        error.as_client_api_error().map(|error| &error.body)
    else {
        return error.to_user_facing();
    };

    match kind {
        // The address is not on any account here. Said plainly rather than
        // vaguely: a homeserver that answers this has already told anybody
        // asking, so there is nothing to protect by being coy.
        ErrorKind::ThreepidNotFound => {
            gettext("No account on this homeserver uses that email address.")
        }
        // What Synapse answers when it has no email configuration at all, as
        // well as when it refuses the address itself.
        ErrorKind::ThreepidDenied => {
            gettext("This homeserver cannot send email, so a password cannot be reset here.")
        }
        _ => error.to_user_facing(),
    }
}

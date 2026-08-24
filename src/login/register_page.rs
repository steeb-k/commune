use std::time::Duration;

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone};
use ruma::{
    OwnedServerName,
    api::{
        client::account::{get_username_availability, register},
        error::{ErrorBody, ErrorKind, StandardErrorBody},
    },
};
use tracing::warn;
use url::Url;

use super::Login;
use crate::{
    components::{AuthDialog, AuthError, LoadingButton, OfflineBanner},
    gettext_f,
    prelude::*,
    spawn, spawn_tokio, toast,
    utils::matrix::validate_password,
};

/// How long to wait after the last keystroke before asking the homeserver
/// whether a username is free.
const AVAILABILITY_DEBOUNCE: Duration = Duration::from_millis(500);

/// What the homeserver says about the username that was typed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum UsernameState {
    /// Nothing has been typed, or the answer is still being waited for.
    #[default]
    Pending,
    /// The homeserver says the username is free.
    Free,
    /// The homeserver says the username cannot be registered.
    Refused,
    /// The homeserver did not say, so the username is treated as usable.
    ///
    /// The register request is the authority anyway; this endpoint is a
    /// courtesy and a homeserver is allowed not to answer it.
    Unknown,
}

impl UsernameState {
    /// Whether registration may be attempted with a username in this state.
    const fn allows_register(self) -> bool {
        matches!(self, Self::Free | Self::Unknown)
    }
}

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/register_page.ui")]
    #[properties(wrapper_type = super::LoginRegisterPage)]
    pub struct LoginRegisterPage {
        #[template_child]
        title: TemplateChild<gtk::Label>,
        #[template_child]
        homeserver_url: TemplateChild<gtk::Label>,
        #[template_child]
        username_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        username_error_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        username_error: TemplateChild<gtk::Label>,
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
        next_button: TemplateChild<LoadingButton>,
        /// The parent `Login` object.
        #[property(get, set, nullable)]
        login: glib::WeakRef<Login>,
        /// What the homeserver said about the username that was typed.
        username_state: Cell<UsernameState>,
        /// The pending debounce of the username entry.
        availability_timeout: RefCell<Option<glib::SourceId>>,
        /// The identifier of the current availability request.
        ///
        /// It is bumped every time the username changes, so the answer to a
        /// request that was overtaken by a newer one is discarded.
        generation: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginRegisterPage {
        const NAME: &'static str = "LoginRegisterPage";
        type Type = super::LoginRegisterPage;
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
    impl ObjectImpl for LoginRegisterPage {
        fn constructed(&self) {
            self.parent_constructed();

            // The same offsets as the "Change Password" page, so that the two
            // meters are read the same way. Blueprint has no syntax for them.
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

        fn dispose(&self) {
            if let Some(source) = self.availability_timeout.take() {
                source.remove();
            }
        }
    }

    impl WidgetImpl for LoginRegisterPage {
        fn grab_focus(&self) -> bool {
            self.username_entry.grab_focus()
        }
    }

    impl NavigationPageImpl for LoginRegisterPage {
        fn shown(&self) {
            self.grab_focus();
        }
    }

    #[gtk::template_callbacks]
    impl LoginRegisterPage {
        /// The username entered by the user.
        fn username(&self) -> glib::GString {
            self.username_entry.text()
        }

        /// The password entered by the user.
        fn password(&self) -> glib::GString {
            self.password_entry.text()
        }

        /// Update the domain name and URL displayed in the title.
        pub(super) fn update_title(
            &self,
            homeserver_url: &Url,
            server_name: Option<&OwnedServerName>,
        ) {
            let title = if let Some(server_name) = server_name {
                gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', this is a
                    // variable name.
                    "Create an account on {domain_name}",
                    &[(
                        "domain_name",
                        &format!("<span segment=\"word\">{server_name}</span>"),
                    )],
                )
            } else {
                gettext("Create an account")
            };
            self.title.set_markup(&title);

            let homeserver_url = homeserver_url.as_str().trim_end_matches('/');
            self.homeserver_url.set_label(homeserver_url);
        }

        /// Handle a change of the username entry, debounced.
        #[template_callback]
        fn username_changed(&self) {
            if let Some(source) = self.availability_timeout.take() {
                source.remove();
            }

            // Bump the generation right away, so that the answer to the request
            // for the previous username is discarded even before this one
            // starts.
            self.generation.set(self.generation.get().wrapping_add(1));

            self.username_state.set(UsernameState::Pending);
            self.set_username_error(None);
            self.update_next_state();

            if self.username().is_empty() {
                return;
            }

            let source = glib::timeout_add_local_once(
                AVAILABILITY_DEBOUNCE,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        imp.availability_timeout.take();
                        spawn!(async move {
                            imp.check_username_availability().await;
                        });
                    }
                ),
            );
            self.availability_timeout.replace(Some(source));
        }

        /// Ask the homeserver whether the current username can be registered.
        async fn check_username_availability(&self) {
            let Some(login) = self.login.upgrade() else {
                return;
            };
            let Some(client) = login.client().await else {
                return;
            };

            let username = self.username();
            if username.is_empty() {
                return;
            }

            let generation = self.generation.get();
            let request = get_username_availability::v3::Request::new(username.to_string());
            let handle = spawn_tokio!(async move { client.send(request).await });

            let result = handle.await.expect("task was not aborted");

            if self.generation.get() != generation {
                // The username changed while we were asking about this one.
                return;
            }

            let (state, error) = match result {
                Ok(response) if response.available => (UsernameState::Free, None),
                Ok(_) => (
                    UsernameState::Refused,
                    Some(gettext("This username is not available")),
                ),
                Err(error) => match error_kind(&error) {
                    Some(ErrorKind::UserInUse) => (
                        UsernameState::Refused,
                        Some(gettext("This username is already taken")),
                    ),
                    Some(ErrorKind::InvalidUsername) => (
                        UsernameState::Refused,
                        Some(gettext("This username is not valid on this homeserver")),
                    ),
                    Some(ErrorKind::Exclusive) => (
                        UsernameState::Refused,
                        Some(gettext("This username is reserved by the homeserver")),
                    ),
                    _ => {
                        // The homeserver does not have to answer this, and the
                        // register request settles it anyway.
                        warn!("Could not check whether the username is available: {error}");
                        (UsernameState::Unknown, None)
                    }
                },
            };

            self.username_state.set(state);
            self.set_username_error(error.as_deref());
            self.update_next_state();
        }

        /// Show the given error about the username, or none.
        fn set_username_error(&self, error: Option<&str>) {
            if let Some(error) = error {
                self.username_error.set_label(error);
                self.username_error_revealer.set_reveal_child(true);
                self.username_entry.remove_css_class("success");
                self.username_entry.add_css_class("warning");
            } else {
                self.username_error_revealer.set_reveal_child(false);
                self.username_entry.remove_css_class("warning");

                if self.username_state.get() == UsernameState::Free {
                    self.username_entry.add_css_class("success");
                } else {
                    self.username_entry.remove_css_class("success");
                }
            }
        }

        /// Validate the password, and show what is missing from it.
        #[template_callback]
        fn validate_password(&self) {
            let entry = &self.password_entry;
            let progress = &self.password_progress;
            let revealer = &self.password_error_revealer;
            let label = &self.password_error;
            let password = entry.text();

            if password.is_empty() {
                revealer.set_reveal_child(false);
                entry.remove_css_class("success");
                entry.remove_css_class("warning");
                progress.set_value(0.0);
                progress.remove_css_class("success");
                progress.remove_css_class("warning");
                self.validate_password_confirmation();
                return;
            }

            let validity = validate_password(&password);

            progress.set_value(f64::from(validity.progress) / 20.0);
            if validity.progress == 100 {
                revealer.set_reveal_child(false);
                entry.add_css_class("success");
                entry.remove_css_class("warning");
                progress.add_css_class("success");
                progress.remove_css_class("warning");
            } else {
                entry.remove_css_class("success");
                entry.add_css_class("warning");
                progress.remove_css_class("success");
                progress.add_css_class("warning");

                if !validity.has_length {
                    label.set_label(&gettext("Password must be at least 8 characters long"));
                } else if !validity.has_lowercase {
                    label.set_label(&gettext(
                        "Password must have at least one lower-case letter",
                    ));
                } else if !validity.has_uppercase {
                    label.set_label(&gettext(
                        "Password must have at least one upper-case letter",
                    ));
                } else if !validity.has_number {
                    label.set_label(&gettext("Password must have at least one digit"));
                } else if !validity.has_symbol {
                    label.set_label(&gettext("Password must have at least one symbol"));
                }

                revealer.set_reveal_child(true);
            }

            self.validate_password_confirmation();
        }

        /// Check that the password confirmation matches the password.
        #[template_callback]
        fn validate_password_confirmation(&self) {
            let entry = &self.confirm_password_entry;
            let revealer = &self.confirm_password_error_revealer;
            let label = &self.confirm_password_error;
            let password = self.password();
            let confirmation = entry.text();

            if confirmation.is_empty() {
                revealer.set_reveal_child(false);
                entry.remove_css_class("success");
                entry.remove_css_class("warning");
                self.update_next_state();
                return;
            }

            if password == confirmation {
                revealer.set_reveal_child(false);
                entry.add_css_class("success");
                entry.remove_css_class("warning");
            } else {
                entry.remove_css_class("success");
                entry.add_css_class("warning");
                label.set_label(&gettext("Passwords do not match"));
                revealer.set_reveal_child(true);
            }

            self.update_next_state();
        }

        /// Whether the current state allows to create an account.
        fn can_register(&self) -> bool {
            let username = self.username();
            let password = self.password();

            !username.is_empty()
                && self.username_state.get().allows_register()
                && validate_password(&password).progress == 100
                && password == self.confirm_password_entry.text()
        }

        /// Update the state of the "Create Account" button.
        fn update_next_state(&self) {
            self.next_button.set_sensitive(self.can_register());
        }

        /// Create the account with the data that was entered.
        #[template_callback]
        async fn register(&self) {
            if !self.can_register() {
                return;
            }

            let Some(login) = self.login.upgrade() else {
                return;
            };
            let Some(client) = login.client().await else {
                return;
            };

            self.next_button.set_is_loading(true);
            login.freeze();

            let username = self.username().to_string();
            let password = self.password().to_string();

            // There is no session yet: the account is what this flow creates,
            // so the dialog is built from the client alone.
            let dialog = AuthDialog::for_client(client, None);
            let result = dialog
                .authenticate(&*self.obj(), move |client, auth| {
                    let username = username.clone();
                    let password = password.clone();
                    async move {
                        let mut request = register::v3::Request::new();
                        request.username = Some(username);
                        request.password = Some(password);
                        request.initial_device_display_name = Some(crate::APP_NAME.to_owned());
                        request.auth = auth;

                        client.matrix_auth().register(request).await
                    }
                })
                .await;

            match result {
                Ok(_) => {
                    // `MatrixAuth::register` sets the session from the response,
                    // so this is the same path a password login takes.
                    login.create_session().await;
                }
                Err(AuthError::UserCancelled) => {}
                Err(AuthError::ServerResponse(error)) => {
                    warn!("Could not create account: {error}");

                    let message =
                        if matches!(error.client_api_error_kind(), Some(ErrorKind::Forbidden)) {
                            // The catch-all for `M_FORBIDDEN` is "Invalid
                            // credentials", which is not what it means here.
                            gettext("This homeserver does not allow creating an account")
                        } else {
                            error.to_user_facing()
                        };
                    toast!(self.obj(), message);
                }
                Err(error) => {
                    warn!("Could not create account: {error}");
                    toast!(self.obj(), gettext("Could not create account"));
                }
            }

            self.next_button.set_is_loading(false);
            login.unfreeze();
        }

        /// Reset this page.
        pub(super) fn clean(&self) {
            if let Some(source) = self.availability_timeout.take() {
                source.remove();
            }
            self.generation.set(self.generation.get().wrapping_add(1));

            self.username_entry.set_text("");
            self.password_entry.set_text("");
            self.confirm_password_entry.set_text("");
            self.username_state.set(UsernameState::Pending);
            self.set_username_error(None);
            self.next_button.set_is_loading(false);
            self.validate_password();
        }
    }
}

glib::wrapper! {
    /// The login page to create an account with a username and a password.
    pub struct LoginRegisterPage(ObjectSubclass<imp::LoginRegisterPage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl LoginRegisterPage {
    /// The tag for this page.
    pub(super) const TAG: &str = "register";

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

/// The error kind of the given HTTP error, if it is a Matrix API error.
fn error_kind(error: &matrix_sdk::HttpError) -> Option<&ErrorKind> {
    let ErrorBody::Standard(StandardErrorBody { kind, .. }) = &error.as_client_api_error()?.body
    else {
        return None;
    };

    Some(kind)
}

use crate::Session;

use adw::subclass::prelude::BinImpl;
use gettextrs::gettext;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};
use log::debug;
use std::fmt;
use std::time::Duration;
use url::{ParseError, Url};

mod imp {
    use super::*;
    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/login.ui")]
    pub struct Login {
        #[template_child]
        pub next_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub next_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub next_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub next_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub homeserver_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub username_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub password_entry: TemplateChild<gtk::PasswordEntry>,
        #[template_child]
        pub error_message: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Login {
        const NAME: &'static str = "Login";
        type Type = super::Login;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_accessible_role(gtk::AccessibleRole::Group);
            klass.install_action("login.next", None, move |widget, _, _| widget.forward());
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Login {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder(
                    "new-session",
                    &[Session::static_type().into()],
                    <()>::static_type().into(),
                )
                .build()]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self, obj: &Self::Type) {
            obj.action_set_enabled("login.next", false);

            self.parent_constructed(obj);

            self.homeserver_entry
                .connect_changed(clone!(@weak obj => move |_| obj.enable_next_action()));
            self.username_entry
                .connect_changed(clone!(@weak obj => move |_| obj.enable_next_action()));
            self.password_entry
                .connect_changed(clone!(@weak obj => move |_| obj.enable_next_action()));
        }
    }

    impl WidgetImpl for Login {}

    impl BinImpl for Login {}
}

glib::wrapper! {
    pub struct Login(ObjectSubclass<imp::Login>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Login {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create Login")
    }

    fn enable_next_action(&self) {
        let priv_ = imp::Login::from_instance(self);
        let homeserver = priv_.homeserver_entry.text();
        let username = priv_.username_entry.text_length();
        let password = priv_.password_entry.text().len();

        self.action_set_enabled(
            "login.next",
            homeserver.len() != 0
                && build_homeserver_url(homeserver.as_str()).is_ok()
                && username != 0
                && password != 0,
        );
    }

    fn forward(&self) {
        self.login();
    }

    fn login(&self) {
        let priv_ = imp::Login::from_instance(self);
        let homeserver = priv_.homeserver_entry.text().to_string();
        let username = priv_.username_entry.text().to_string();
        let password = priv_.password_entry.text().to_string();

        self.freeze();

        let session = Session::new();

        self.set_handler_for_prepared_session(&session);

        session.login_with_password(
            build_homeserver_url(homeserver.as_str()).unwrap(),
            username,
            password,
        );
    }

    fn clean(&self) {
        let priv_ = imp::Login::from_instance(self);
        priv_.homeserver_entry.set_text("");
        priv_.username_entry.set_text("");
        priv_.password_entry.set_text("");
        self.unfreeze();
    }

    fn freeze(&self) {
        let priv_ = imp::Login::from_instance(self);

        self.action_set_enabled("login.next", false);
        priv_
            .next_stack
            .set_visible_child(&priv_.next_spinner.get());
        priv_.main_stack.set_sensitive(false);
    }

    fn unfreeze(&self) {
        let priv_ = imp::Login::from_instance(self);

        self.action_set_enabled("login.next", true);
        priv_.next_stack.set_visible_child(&priv_.next_label.get());
        priv_.main_stack.set_sensitive(true);
    }

    pub fn connect_new_session<F: Fn(&Self, &Session) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("new-session", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            let session = values[1].get::<Session>().unwrap();

            f(&obj, &session);

            None
        })
        .unwrap()
    }

    pub fn default_widget(&self) -> gtk::Widget {
        imp::Login::from_instance(self).next_button.get().upcast()
    }

    pub fn set_handler_for_prepared_session(&self, session: &Session) {
        session.connect_prepared(clone!(@weak self as login => move |session| {
            if let Some(error) = session.get_error() {
                let error_message = &imp::Login::from_instance(&login).error_message;
                error_message.set_text(&error.to_string());
                error_message.show();
                debug!("Failed to create a new session: {:?}", error);

                login.unfreeze();
            } else {
                debug!("A new session was prepared");
                login.emit_by_name("new-session", &[&session]).unwrap();
                login.clean();
            }
        }));
    }
}

impl Default for Login {
    fn default() -> Self {
        Self::new()
    }
}

fn build_homeserver_url(server: &str) -> Result<Url, ParseError> {
    if server.starts_with("http://") || server.starts_with("https://") {
        Url::parse(server)
    } else {
        Url::parse(&format!("https://{}", server))
    }
}

#[derive(Debug)]
pub enum LoginError {
    ServerNotFound,
    Forbidden,
    UserDeactivated,
    LimitExceeded(Option<Duration>),
    Unknown(String),
}

impl fmt::Display for LoginError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let error_msg = match &self {
            LoginError::ServerNotFound => gettext("⚠️ Homeserver not found."),
            LoginError::Forbidden => gettext("⚠️ Invalid credentials."),
            LoginError::UserDeactivated => gettext("⚠️ The user is deactivated."),
            LoginError::LimitExceeded(retry_ms) => {
                if let Some(ms) = retry_ms {
                    gettext(format!(
                        "⚠️ Exceeded rate limit, retry in {} seconds.",
                        ms.as_secs()
                    ))
                } else {
                    gettext("⚠️ Exceeded rate limit, try again later.")
                }
            }
            LoginError::Unknown(info) => {
                debug!("Unknown error occurred during login: {}", info);
                gettext("⚠️ Login Failed! Unknown error.")
            }
        };
        f.write_str(&error_msg)
    }
}

impl From<matrix_sdk::Error> for LoginError {
    /// Transform a matrix_sdk error into a LoginError based on the login with password
    /// Logging in can result in the following errors:
    /// M_FORBIDDEN: The provided authentication data was incorrect.
    /// M_USER_DEACTIVATED: The user has been deactivated.
    /// M_LIMIT_EXCEEDED: This request was rate-limited.
    /// M_UNKNOWN: An unknown error occurred
    /// or the home server was not found/unavailable (a Reqwest error)
    fn from(error: matrix_sdk::Error) -> Self {
        use matrix_sdk::ruma::api::client::error::ErrorKind::{
            Forbidden, LimitExceeded, UserDeactivated,
        };
        use matrix_sdk::ruma::api::error::{FromHttpResponseError, ServerError};
        use matrix_sdk::Error::Http;
        use matrix_sdk::HttpError::{ClientApi, Reqwest};
        match error {
            Http(Reqwest(_)) => LoginError::ServerNotFound,
            Http(ClientApi(FromHttpResponseError::Http(ServerError::Known(server_err)))) => {
                match server_err.kind {
                    Forbidden => LoginError::Forbidden,
                    UserDeactivated => LoginError::UserDeactivated,
                    LimitExceeded { retry_after_ms } => LoginError::LimitExceeded(retry_after_ms),
                    e => LoginError::Unknown(e.to_string()),
                }
            }
            e => LoginError::Unknown(e.to_string()),
        }
    }
}

use crate::Session;

use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};
use log::debug;
use url::{ParseError, Url};

mod imp {
    use super::*;
    use glib::subclass::{InitializingObject, Signal};
    use glib::SignalHandlerId;
    use once_cell::sync::Lazy;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/login.ui")]
    pub struct Login {
        pub current_session: RefCell<Option<Session>>,
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
        pub back_to_session_button: TemplateChild<gtk::Button>,
        pub prepared_source_id: RefCell<Option<SignalHandlerId>>,
        pub logged_out_source_id: RefCell<Option<SignalHandlerId>>,
        pub ready_source_id: RefCell<Option<SignalHandlerId>>,
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
        let username_length = priv_.username_entry.text_length();
        let password_length = priv_.password_entry.text().len();

        self.action_set_enabled(
            "login.next",
            homeserver.len() != 0
                && build_homeserver_url(homeserver.as_str()).is_ok()
                && username_length != 0
                && password_length != 0,
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
        priv_.current_session.replace(Some(session));
    }

    pub fn clean(&self) {
        let priv_ = imp::Login::from_instance(self);
        priv_.homeserver_entry.set_text("");
        priv_.username_entry.set_text("");
        priv_.password_entry.set_text("");
        self.unfreeze();
        self.drop_session_reference();
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

    pub fn connect_new_session<F: Fn(&Self, Session) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("new-session", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            let session = values[1].get::<Session>().unwrap();

            f(&obj, session);

            None
        })
        .unwrap()
    }

    fn drop_session_reference(&self) {
        let priv_ = imp::Login::from_instance(self);

        if let Some(session) = priv_.current_session.take() {
            if let Some(id) = priv_.prepared_source_id.take() {
                session.disconnect(id);
            }
            if let Some(id) = priv_.logged_out_source_id.take() {
                session.disconnect(id);
            }
            if let Some(id) = priv_.ready_source_id.take() {
                session.disconnect(id);
            }
        }
    }

    pub fn default_widget(&self) -> gtk::Widget {
        imp::Login::from_instance(self).next_button.get().upcast()
    }

    pub fn show_back_to_session_button(&self, show: bool) {
        let priv_ = imp::Login::from_instance(self);

        priv_.back_to_session_button.set_visible(show);
    }

    fn set_handler_for_prepared_session(&self, session: &Session) {
        let priv_ = imp::Login::from_instance(self);
        priv_
            .prepared_source_id
            .replace(Some(session.connect_prepared(
                clone!(@weak self as login => move |session, error| {
                    match error {
                        Some(e) => {
                            login.parent_window().append_error(&e);
                            login.unfreeze();
                        },
                        None => {
                            debug!("A new session was prepared");
                            login.emit_by_name("new-session", &[&session]).unwrap();
                        }
                    }
                }),
            )));

        priv_.ready_source_id.replace(Some(session.connect_ready(
            clone!(@weak self as login => move |_| {
                login.clean();
            }),
        )));

        priv_
            .logged_out_source_id
            .replace(Some(session.connect_logged_out(
                clone!(@weak self as login => move |_| {
                    login.parent_window().switch_to_login_page(false);
                    login.drop_session_reference();
                    login.unfreeze();
                }),
            )));
    }

    fn parent_window(&self) -> crate::Window {
        self.root()
            .and_then(|root| root.downcast().ok())
            .expect("Login needs to have a parent window")
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

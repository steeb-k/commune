use crate::Session;

use adw;
use adw::subclass::prelude::BinImpl;
use gettextrs::gettext;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};
use log::debug;

mod imp {
    use super::*;
    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/login.ui")]
    pub struct Login {
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
        let priv_ = imp::Login::from_instance(&self);
        let homeserver = priv_.homeserver_entry.text();
        let username = priv_.username_entry.text_length();
        let password = priv_.password_entry.text().len();

        self.action_set_enabled(
            "login.next",
            homeserver.len() != 0
                && url::Url::parse(homeserver.as_str()).is_ok()
                && username != 0
                && password != 0,
        );
    }

    fn forward(&self) {
        self.login();
    }

    fn login(&self) {
        let priv_ = imp::Login::from_instance(&self);
        let homeserver = priv_.homeserver_entry.text().to_string();
        let username = priv_.username_entry.text().to_string();
        let password = priv_.password_entry.text().to_string();

        self.freeze();

        let session = Session::new();

        session.connect_prepared(clone!(@weak self as obj, @strong session => move |_| {
            if let Some(error) = session.get_error() {
                let error_message = &imp::Login::from_instance(&obj).error_message;
                // TODO: show more specific error
                error_message.set_text(&gettext("⚠️ The Login failed."));
                error_message.show();
                debug!("Failed to create a new session: {:?}", error);

                obj.unfreeze();
            } else {
                debug!("A new session was prepared");
                obj.emit_by_name("new-session", &[&session]).unwrap();
                obj.clean();
            }
        }));

        session.login_with_password(
            url::Url::parse(homeserver.as_str()).unwrap(),
            username,
            password,
        );
    }

    fn clean(&self) {
        let priv_ = imp::Login::from_instance(&self);
        priv_.homeserver_entry.set_text("");
        priv_.username_entry.set_text("");
        priv_.password_entry.set_text("");
        self.unfreeze();
    }

    fn freeze(&self) {
        let priv_ = imp::Login::from_instance(&self);

        self.action_set_enabled("login.next", false);
        priv_
            .next_stack
            .set_visible_child(&priv_.next_spinner.get());
        priv_.main_stack.set_sensitive(false);
    }

    fn unfreeze(&self) {
        let priv_ = imp::Login::from_instance(&self);

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
}

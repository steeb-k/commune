use adw::subclass::prelude::*;
use gtk::{self, glib, CompositeTemplate};

pub enum ErrorSubpage {
    SecretError,
    SessionError,
}

impl ErrorSubpage {
    fn as_str(&self) -> &str {
        match self {
            Self::SecretError => "secret-error",
            Self::SessionError => "session-error",
        }
    }
}

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/error_page.ui")]
    pub struct ErrorPage {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub secret_error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub session_error_page: TemplateChild<adw::StatusPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ErrorPage {
        const NAME: &'static str = "ErrorPage";
        type Type = super::ErrorPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ErrorPage {}
    impl WidgetImpl for ErrorPage {}
    impl BinImpl for ErrorPage {}
}

glib::wrapper! {
    /// A view displaying an error.
    pub struct ErrorPage(ObjectSubclass<imp::ErrorPage>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl ErrorPage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn display_secret_error(&self, message: &str) {
        let imp = self.imp();
        imp.secret_error_page.set_description(Some(message));
        imp.stack
            .set_visible_child_name(ErrorSubpage::SecretError.as_str());
    }

    pub fn display_session_error(&self, message: &str) {
        let imp = self.imp();
        imp.session_error_page.set_description(Some(message));
        imp.stack
            .set_visible_child_name(ErrorSubpage::SessionError.as_str());
    }
}

use adw::subclass::prelude::*;
use gtk::{self, glib, CompositeTemplate};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/login/sso_page.ui")]
    pub struct LoginSsoPage {}

    #[glib::object_subclass]
    impl ObjectSubclass for LoginSsoPage {
        const NAME: &'static str = "LoginSsoPage";
        type Type = super::LoginSsoPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LoginSsoPage {}
    impl WidgetImpl for LoginSsoPage {}
    impl NavigationPageImpl for LoginSsoPage {}
}

glib::wrapper! {
    /// A page shown while the user is logging in via SSO.
    pub struct LoginSsoPage(ObjectSubclass<imp::LoginSsoPage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

impl LoginSsoPage {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

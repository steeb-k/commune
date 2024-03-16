use adw::subclass::prelude::BinImpl;
use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::components::OfflineBanner;

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/greeter.ui")]
    pub struct Greeter {
        #[template_child]
        pub login_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Greeter {
        const NAME: &'static str = "Greeter";
        type Type = super::Greeter;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            OfflineBanner::ensure_type();

            Self::bind_template(klass);
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Greeter {}

    impl WidgetImpl for Greeter {}
    impl BinImpl for Greeter {}
    impl AccessibleImpl for Greeter {}
}

glib::wrapper! {
    /// The welcome screen of the app.
    pub struct Greeter(ObjectSubclass<imp::Greeter>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Greeter {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn default_widget(&self) -> gtk::Widget {
        self.imp().login_button.get().upcast()
    }
}

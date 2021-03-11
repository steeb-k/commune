use adw;
use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, CompositeTemplate};

mod imp {
    use super::*;
    use glib::subclass;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/login.ui")]
    pub struct FrctlLogin {
        #[template_child]
        pub headerbar: TemplateChild<gtk::HeaderBar>,
    }

    impl ObjectSubclass for FrctlLogin {
        const NAME: &'static str = "FrctlLogin";
        type Type = super::FrctlLogin;
        type ParentType = adw::Bin;
        type Interfaces = ();
        type Instance = subclass::simple::InstanceStruct<Self>;
        type Class = subclass::simple::ClassStruct<Self>;

        glib::object_subclass!();

        fn new() -> Self {
            Self {
                headerbar: TemplateChild::default(),
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        // You must call `Widget`'s `init_template()` within `instance_init()`.
        fn instance_init(obj: &glib::subclass::InitializingObject<Self::Type>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FrctlLogin {}
    impl WidgetImpl for FrctlLogin {}
    impl BinImpl for FrctlLogin {}
}

glib::wrapper! {
    pub struct FrctlLogin(ObjectSubclass<imp::FrctlLogin>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl FrctlLogin {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create FrctlLogin")
    }
}

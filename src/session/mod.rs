mod content;
mod sidebar;

use self::content::FrctlContent;
use self::sidebar::FrctlSidebar;
use adw;
use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, CompositeTemplate};

mod imp {
    use super::*;
    use glib::subclass;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/session.ui")]
    pub struct FrctlSession {
        #[template_child]
        pub sidebar: TemplateChild<FrctlSidebar>,
        #[template_child]
        pub content: TemplateChild<FrctlContent>,
    }

    impl ObjectSubclass for FrctlSession {
        const NAME: &'static str = "FrctlSession";
        type Type = super::FrctlSession;
        type ParentType = adw::Bin;
        type Interfaces = ();
        type Instance = subclass::simple::InstanceStruct<Self>;
        type Class = subclass::simple::ClassStruct<Self>;

        glib::object_subclass!();

        fn new() -> Self {
            Self {
                sidebar: TemplateChild::default(),
                content: TemplateChild::default(),
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

    impl ObjectImpl for FrctlSession {}
    impl WidgetImpl for FrctlSession {}
    impl BinImpl for FrctlSession {}
}

glib::wrapper! {
    pub struct FrctlSession(ObjectSubclass<imp::FrctlSession>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl FrctlSession {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create FrctlSession")
    }
}

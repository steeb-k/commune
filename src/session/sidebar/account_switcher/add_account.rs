use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, CompositeTemplate};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/add-account-row.ui")]
    pub struct AddAccountRow;

    #[glib::object_subclass]
    impl ObjectSubclass for AddAccountRow {
        const NAME: &'static str = "AddAccountRow";
        type Type = super::AddAccountRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AddAccountRow {}
    impl WidgetImpl for AddAccountRow {}
    impl BinImpl for AddAccountRow {}
}

glib::wrapper! {
    pub struct AddAccountRow(ObjectSubclass<imp::AddAccountRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl AddAccountRow {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create AddAccountRow")
    }
}

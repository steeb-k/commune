use gettextrs::gettext;
use gtk::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Spinner {
        inner: gtk::Spinner,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Spinner {
        const NAME: &'static str = "Spinner";
        type Type = super::Spinner;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_css_name("spinner-wrapper");
            klass.set_accessible_role(gtk::AccessibleRole::Status);
        }
    }

    impl ObjectImpl for Spinner {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.inner.set_parent(&*obj);
            obj.update_property(&[gtk::accessible::Property::Label(&gettext("Loading"))])
        }

        fn dispose(&self) {
            self.inner.unparent();
        }
    }

    impl WidgetImpl for Spinner {
        fn map(&self) {
            self.parent_map();
            self.inner.start();
        }

        fn unmap(&self) {
            self.inner.stop();
            self.parent_unmap();
        }
    }

    impl AccessibleImpl for Spinner {
        fn first_accessible_child(&self) -> Option<gtk::Accessible> {
            // Hide the children in the a11y tree.
            None
        }
    }
}

glib::wrapper! {
    /// A spinner.
    ///
    /// This is a wrapper around `GtkSpinner` that makes sure the spinner is stopped when it is not mapped.
    pub struct Spinner(ObjectSubclass<imp::Spinner>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

impl Default for Spinner {
    fn default() -> Self {
        glib::Object::new()
    }
}

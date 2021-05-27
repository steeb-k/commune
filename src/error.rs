use gtk::{glib, subclass::prelude::*};

use matrix_sdk::Error as MatrixError;

mod imp {
    use super::*;
    use once_cell::sync::OnceCell;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct Error {
        pub matrix_error: OnceCell<MatrixError>,
        pub widget_builder:
            RefCell<Option<Box<dyn Fn(&super::Error) -> Option<gtk::Widget> + 'static>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Error {
        const NAME: &'static str = "Error";
        type Type = super::Error;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for Error {}
}

glib::wrapper! {
    pub struct Error(ObjectSubclass<imp::Error>);
}

/// An `Error` that can be shown in the UI
impl Error {
    pub fn new<F: Fn(&Self) -> Option<gtk::Widget> + 'static>(error: MatrixError, f: F) -> Self {
        let obj: Self = glib::Object::new(&[]).expect("Failed to create Error");
        obj.set_matrix_error(error);
        obj.set_widget_builder(f);
        obj
    }

    fn set_matrix_error(&self, error: MatrixError) {
        let priv_ = imp::Error::from_instance(&self);
        priv_.matrix_error.set(error).unwrap()
    }

    pub fn matrix_error(&self) -> &MatrixError {
        let priv_ = imp::Error::from_instance(&self);
        priv_.matrix_error.get().unwrap()
    }

    /// Set a function that builds the widget used to display this error in the UI
    pub fn set_widget_builder<F: Fn(&Self) -> Option<gtk::Widget> + 'static>(&self, f: F) {
        let priv_ = imp::Error::from_instance(&self);
        priv_.widget_builder.replace(Some(Box::new(f)));
    }

    /// Produces a widget via the function set in `Self::set_widget_builder()`
    pub fn widget(&self) -> Option<gtk::Widget> {
        let priv_ = imp::Error::from_instance(&self);
        let widget_builder = priv_.widget_builder.borrow();
        let widget_builder = widget_builder.as_ref()?;
        widget_builder(self)
    }
}

use gtk::{glib, subclass::prelude::*};

type WidgetBuilderFn = Box<dyn Fn(&super::Error) -> Option<gtk::Widget> + 'static>;

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    pub struct Error {
        pub widget_builder: RefCell<Option<WidgetBuilderFn>>,
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
    /// An `Error` that can be shown in the UI.
    pub struct Error(ObjectSubclass<imp::Error>);
}

impl Error {
    pub fn new<F: Fn(&Self) -> Option<gtk::Widget> + 'static>(f: F) -> Self {
        let obj: Self = glib::Object::new(&[]).expect("Failed to create Error");
        obj.set_widget_builder(f);
        obj
    }

    /// Set a function that builds the widget used to display this error in the
    /// UI
    pub fn set_widget_builder<F: Fn(&Self) -> Option<gtk::Widget> + 'static>(&self, f: F) {
        let priv_ = imp::Error::from_instance(self);
        priv_.widget_builder.replace(Some(Box::new(f)));
    }

    /// Produces a widget via the function set in `Self::set_widget_builder()`
    pub fn widget(&self) -> Option<gtk::Widget> {
        let priv_ = imp::Error::from_instance(self);
        let widget_builder = priv_.widget_builder.borrow();
        let widget_builder = widget_builder.as_ref()?;
        widget_builder(self)
    }
}

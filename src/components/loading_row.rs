use glib::subclass::Signal;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};

use super::Spinner;

mod imp {
    use std::marker::PhantomData;

    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/loading_row.ui")]
    #[properties(wrapper_type = super::LoadingRow)]
    pub struct LoadingRow {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub error_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub retry_button: TemplateChild<gtk::Button>,
        /// Whether this row is showing the spinner.
        #[property(get = Self::is_loading)]
        pub loading: PhantomData<bool>,
        /// The error message to display.
        #[property(get = Self::error, set = Self::set_error, explicit_notify, nullable)]
        pub error: PhantomData<Option<glib::GString>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoadingRow {
        const NAME: &'static str = "ComponentsLoadingRow";
        type Type = super::LoadingRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            Spinner::static_type();

            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LoadingRow {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("retry").build()]);
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.retry_button
                .connect_clicked(clone!(@weak obj => move |_| {
                    obj.emit_by_name::<()>("retry", &[]);
                }));
        }
    }

    impl WidgetImpl for LoadingRow {}
    impl ListBoxRowImpl for LoadingRow {}

    impl LoadingRow {
        /// Whether this row is showing the spinner.
        fn is_loading(&self) -> bool {
            self.stack.visible_child_name().as_deref() == Some("loading")
        }

        /// The error message to display.
        fn error(&self) -> Option<glib::GString> {
            if self.is_loading() {
                return None;
            }

            let message = self.error_label.text();
            if message.is_empty() {
                None
            } else {
                Some(message)
            }
        }

        /// Set the error message to display.
        ///
        /// If this is `Some`, the error will be shown, otherwise the spinner
        /// will be shown.
        fn set_error(&self, message: Option<&str>) {
            if let Some(message) = message {
                self.error_label.set_text(message);
                self.stack.set_visible_child_name("error");
            } else {
                self.stack.set_visible_child_name("loading");
            }

            let obj = self.obj();
            obj.notify_loading();
            obj.notify_error();
        }
    }
}

glib::wrapper! {
    /// A `ListBoxRow` containing a loading spinner.
    ///
    /// It's also possible to set an error once the loading fails, including a retry button.
    pub struct LoadingRow(ObjectSubclass<imp::LoadingRow>)
        @extends gtk::Widget, gtk::ListBoxRow, @implements gtk::Accessible;
}

impl LoadingRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn connect_retry<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("retry", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            f(&obj);
            None
        })
    }
}

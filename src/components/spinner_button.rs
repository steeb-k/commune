use adw::subclass::prelude::*;
use gtk::{glib, prelude::*, CompositeTemplate};

use super::LoadingBin;

mod imp {
    use std::marker::PhantomData;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/spinner_button.ui")]
    #[properties(wrapper_type = super::SpinnerButton)]
    pub struct SpinnerButton {
        #[template_child]
        pub loading_bin: TemplateChild<LoadingBin>,
        #[template_child]
        pub child_label: TemplateChild<gtk::Label>,
        /// The label of the button.
        #[property(get = Self::label, set = Self::set_label, override_class = gtk::Button)]
        pub label: PhantomData<glib::GString>,
        /// Whether to display the loading spinner.
        ///
        /// If this is `false`, the text will be displayed.
        #[property(get = Self::is_loading, set = Self::set_loading, explicit_notify)]
        pub loading: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpinnerButton {
        const NAME: &'static str = "SpinnerButton";
        type Type = super::SpinnerButton;
        type ParentType = gtk::Button;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SpinnerButton {}

    impl WidgetImpl for SpinnerButton {}
    impl ButtonImpl for SpinnerButton {}

    impl SpinnerButton {
        /// The label of the button.
        fn label(&self) -> glib::GString {
            self.child_label.label()
        }

        /// Set the label of the button.
        fn set_label(&self, label: &str) {
            if self.child_label.label().as_str() == label {
                return;
            }

            self.child_label.set_label(label);
            self.obj().notify_label();
        }

        /// Whether to display the loading spinner.
        ///
        /// If this is `false`, the text will be displayed.
        fn is_loading(&self) -> bool {
            self.loading_bin.is_loading()
        }

        /// Set whether to display the loading spinner.
        fn set_loading(&self, is_loading: bool) {
            if self.is_loading() == is_loading {
                return;
            }
            let obj = self.obj();

            // The action should have been enabled or disabled so the sensitive
            // state should update itself.
            if obj.action_name().is_none() {
                obj.set_sensitive(!is_loading);
            }

            self.loading_bin.set_is_loading(is_loading);

            obj.notify_loading();
        }
    }
}

glib::wrapper! {
    /// Button showing either a spinner or a label.
    pub struct SpinnerButton(ObjectSubclass<imp::SpinnerButton>)
        @extends gtk::Widget, gtk::Button, @implements gtk::Accessible, gtk::Actionable;
}

impl SpinnerButton {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for SpinnerButton {
    fn default() -> Self {
        Self::new()
    }
}

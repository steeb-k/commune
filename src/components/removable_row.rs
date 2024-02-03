use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::closure_local, CompositeTemplate};

use super::SpinnerButton;

mod imp {
    use std::marker::PhantomData;

    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/removable_row.ui")]
    #[properties(wrapper_type = super::RemovableRow)]
    pub struct RemovableRow {
        #[template_child]
        pub remove_button: TemplateChild<SpinnerButton>,
        /// The tooltip text of the remove button.
        #[property(get = Self::remove_button_tooltip_text, set = Self::set_remove_button_tooltip_text, explicit_notify, nullable)]
        pub remove_button_tooltip_text: PhantomData<Option<glib::GString>>,
        /// Whether this row is loading.
        #[property(get = Self::is_loading, set = Self::set_is_loading, explicit_notify)]
        pub is_loading: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RemovableRow {
        const NAME: &'static str = "RemovableRow";
        type Type = super::RemovableRow;
        type ParentType = adw::ActionRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RemovableRow {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("remove").build()]);
            SIGNALS.as_ref()
        }
    }

    impl WidgetImpl for RemovableRow {}
    impl ListBoxRowImpl for RemovableRow {}
    impl PreferencesRowImpl for RemovableRow {}
    impl ActionRowImpl for RemovableRow {}

    impl RemovableRow {
        /// The tooltip text of the remove button.
        fn remove_button_tooltip_text(&self) -> Option<glib::GString> {
            self.remove_button.tooltip_text()
        }

        /// Set the tooltip text of the remove button.
        fn set_remove_button_tooltip_text(&self, tooltip_text: Option<glib::GString>) {
            if self.remove_button_tooltip_text() == tooltip_text {
                return;
            }

            self.remove_button.set_tooltip_text(tooltip_text.as_deref());
            self.obj().notify_remove_button_tooltip_text();
        }

        /// Whether this row is loading.
        fn is_loading(&self) -> bool {
            self.remove_button.loading()
        }

        /// Set whether this row is loading.
        fn set_is_loading(&self, is_loading: bool) {
            if self.is_loading() == is_loading {
                return;
            }

            self.remove_button.set_loading(is_loading);

            let obj = self.obj();
            obj.set_sensitive(!is_loading);
            obj.notify_is_loading();
        }
    }
}

glib::wrapper! {
    /// An `AdwActionRow` with a "remove" button.
    pub struct RemovableRow(ObjectSubclass<imp::RemovableRow>)
        @extends gtk::Widget, gtk::ListBoxRow, adw::PreferencesRow, adw::ActionRow,
        @implements gtk::Actionable, gtk::Accessible;
}

#[gtk::template_callbacks]
impl RemovableRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Emit the `remove` signal.
    #[template_callback]
    fn remove(&self) {
        self.emit_by_name::<()>("remove", &[]);
    }

    /// Connect to the `remove` signal.
    pub fn connect_remove<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "remove",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

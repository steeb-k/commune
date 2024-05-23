use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::closure_local, CompositeTemplate};

use crate::components::LoadingButton;

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/rows/removable_row.ui")]
    #[properties(wrapper_type = super::RemovableRow)]
    pub struct RemovableRow {
        #[template_child]
        pub remove_button: TemplateChild<LoadingButton>,
        #[template_child]
        pub extra_suffix_bin: TemplateChild<adw::Bin>,
        /// The tooltip text of the remove button.
        #[property(get = Self::remove_button_tooltip_text, set = Self::set_remove_button_tooltip_text, explicit_notify, nullable)]
        pub remove_button_tooltip_text: PhantomData<Option<glib::GString>>,
        /// The accessible label of the remove button.
        #[property(get, set = Self::set_remove_button_accessible_label, explicit_notify, nullable)]
        pub remove_button_accessible_label: RefCell<Option<String>>,
        /// Whether this row is loading.
        #[property(get = Self::is_loading, set = Self::set_is_loading, explicit_notify)]
        pub is_loading: PhantomData<bool>,
        /// The extra suffix widget of this row.
        ///
        /// The widget is placed before the remove button.
        #[property(get = Self::extra_suffix, set = Self::set_extra_suffix, explicit_notify, nullable)]
        pub extra_suffix: PhantomData<Option<gtk::Widget>>,
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

        /// Set the accessible label of the remove button.
        fn set_remove_button_accessible_label(&self, label: Option<String>) {
            if *self.remove_button_accessible_label.borrow() == label {
                return;
            }

            if let Some(label) = &label {
                self.remove_button
                    .update_property(&[gtk::accessible::Property::Label(label)]);
            } else {
                self.remove_button
                    .reset_property(gtk::AccessibleProperty::Label);
            }

            self.remove_button_accessible_label.replace(label);
            self.obj().notify_remove_button_accessible_label();
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

        /// The extra suffix widget of this row.
        fn extra_suffix(&self) -> Option<gtk::Widget> {
            self.extra_suffix_bin.child()
        }

        /// Set the extra suffix widget of this row.
        fn set_extra_suffix(&self, widget: Option<&gtk::Widget>) {
            if self.extra_suffix().as_ref() == widget {
                return;
            }

            self.extra_suffix_bin.set_child(widget);
            self.obj().notify_extra_suffix();
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

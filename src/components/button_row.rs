use adw::subclass::prelude::*;
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    CompositeTemplate,
};

mod imp {
    use std::cell::Cell;

    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/button_row.ui")]
    #[properties(wrapper_type = super::ButtonRow)]
    pub struct ButtonRow {
        /// Whether activating this button opens a subpage.
        #[property(get, set = Self::set_to_subpage, explicit_notify)]
        pub to_subpage: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ButtonRow {
        const NAME: &'static str = "ComponentsButtonRow";
        type Type = super::ButtonRow;
        type ParentType = adw::PreferencesRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ButtonRow {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("activated").build()]);
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            self.obj().connect_parent_notify(|obj| {
                if let Some(listbox) = obj.parent().and_downcast_ref::<gtk::ListBox>() {
                    listbox.connect_row_activated(clone!(@weak obj => move |_, row| {
                        if row == obj.upcast_ref::<gtk::ListBoxRow>() {
                            obj.emit_by_name::<()>("activated", &[]);
                        }
                    }));
                }
            });
        }
    }

    impl WidgetImpl for ButtonRow {}
    impl ListBoxRowImpl for ButtonRow {}
    impl PreferencesRowImpl for ButtonRow {}

    impl ButtonRow {
        /// Set whether activating this button opens a subpage.
        fn set_to_subpage(&self, to_subpage: bool) {
            if self.to_subpage.get() == to_subpage {
                return;
            }

            self.to_subpage.replace(to_subpage);
            self.obj().notify_to_subpage();
        }
    }
}

glib::wrapper! {
    /// An `AdwPreferencesRow` usable as a button.
    pub struct ButtonRow(ObjectSubclass<imp::ButtonRow>)
        @extends gtk::Widget, gtk::ListBoxRow, adw::PreferencesRow, @implements gtk::Accessible;
}

impl ButtonRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn connect_activated<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "activated",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

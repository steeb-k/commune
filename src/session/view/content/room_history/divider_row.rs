use adw::subclass::prelude::*;
use gtk::{glib, prelude::*, CompositeTemplate};

mod imp {
    use std::marker::PhantomData;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/room_history/divider_row.ui")]
    #[properties(wrapper_type = super::DividerRow)]
    pub struct DividerRow {
        #[template_child]
        pub inner_label: TemplateChild<gtk::Label>,
        /// The label of this divider.
        #[property(get = Self::label, set = Self::set_label)]
        label: PhantomData<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DividerRow {
        const NAME: &'static str = "ContentDividerRow";
        type Type = super::DividerRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for DividerRow {}

    impl WidgetImpl for DividerRow {}
    impl BinImpl for DividerRow {}

    impl DividerRow {
        /// The label of this divider.
        fn label(&self) -> String {
            self.inner_label.text().into()
        }

        /// Set the label of this divider.
        fn set_label(&self, label: String) {
            self.inner_label.set_text(&label);
        }
    }
}

glib::wrapper! {
    /// A row presenting a divider in the timeline.
    pub struct DividerRow(ObjectSubclass<imp::DividerRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl DividerRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn with_label(label: String) -> Self {
        glib::Object::builder().property("label", &label).build()
    }
}

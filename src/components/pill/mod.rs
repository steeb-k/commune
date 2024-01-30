use adw::subclass::prelude::*;
use gtk::{glib, prelude::*, CompositeTemplate};

mod source;
mod source_row;

pub use self::{
    source::{PillSource, PillSourceExt, PillSourceImpl},
    source_row::PillSourceRow,
};
use super::Avatar;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/pill/mod.ui")]
    #[properties(wrapper_type = super::Pill)]
    pub struct Pill {
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        /// The source of the data displayed by this widget.
        #[property(get, set = Self::set_source, explicit_notify, nullable)]
        pub source: RefCell<Option<PillSource>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Pill {
        const NAME: &'static str = "Pill";
        type Type = super::Pill;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Pill {}

    impl WidgetImpl for Pill {}
    impl BinImpl for Pill {}

    impl Pill {
        /// Set the source of the data displayed by this widget.
        fn set_source(&self, source: Option<PillSource>) {
            if *self.source.borrow() == source {
                return;
            }

            self.source.replace(source);
            self.obj().notify_source();
        }
    }
}

glib::wrapper! {
    /// Inline widget displaying an emphasized `PillSource`.
    pub struct Pill(ObjectSubclass<imp::Pill>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Pill {
    /// Create a pill with the given source.
    pub fn new(source: &impl IsA<PillSource>) -> Self {
        glib::Object::builder().property("source", source).build()
    }
}

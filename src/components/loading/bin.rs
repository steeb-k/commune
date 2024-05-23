use adw::prelude::*;
use gtk::{glib, subclass::prelude::*, CompositeTemplate};

use super::Spinner;

mod imp {
    use std::marker::PhantomData;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/loading/bin.ui")]
    #[properties(wrapper_type = super::LoadingBin)]
    pub struct LoadingBin {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub child_bin: TemplateChild<adw::Bin>,
        /// The child widget.
        #[property(get = Self::child, set = Self::set_child, explicit_notify, nullable)]
        pub child: PhantomData<Option<gtk::Widget>>,
        /// Whether this is showing the spinner.
        #[property(get = Self::is_loading, set = Self::set_is_loading, explicit_notify)]
        pub is_loading: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoadingBin {
        const NAME: &'static str = "LoadingBin";
        type Type = super::LoadingBin;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();

            Spinner::ensure_type();

            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LoadingBin {
        fn dispose(&self) {
            self.stack.unparent();
        }
    }

    impl WidgetImpl for LoadingBin {}

    impl LoadingBin {
        /// Whether this row is showing the spinner.
        fn is_loading(&self) -> bool {
            self.stack.visible_child_name().as_deref() == Some("loading")
        }

        /// Set whether this row is showing the spinner.
        fn set_is_loading(&self, loading: bool) {
            if self.is_loading() == loading {
                return;
            }

            let child_name = if loading { "loading" } else { "child" };
            self.stack.set_visible_child_name(child_name);
            self.obj().notify_is_loading();
        }

        /// The child widget.
        fn child(&self) -> Option<gtk::Widget> {
            self.child_bin.child()
        }

        /// Set the child widget.
        fn set_child(&self, child: Option<gtk::Widget>) {
            if self.child() == child {
                return;
            }

            self.child_bin.set_child(child.as_ref());
            self.obj().notify_child();
        }
    }
}

glib::wrapper! {
    /// A Bin that shows either its child or a loading spinner.
    pub struct LoadingBin(ObjectSubclass<imp::LoadingBin>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

impl LoadingBin {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for LoadingBin {
    fn default() -> Self {
        Self::new()
    }
}

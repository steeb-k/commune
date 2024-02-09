use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, CompositeTemplate};

mod imp {
    use std::marker::PhantomData;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/toastable_window.ui")]
    #[properties(wrapper_type = super::ToastableWindow)]
    pub struct ToastableWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[property(get = Self::child_content, set = Self::set_child_content, nullable)]
        pub child_content: PhantomData<Option<gtk::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ToastableWindow {
        const NAME: &'static str = "ToastableWindow";
        const ABSTRACT: bool = true;
        type Type = super::ToastableWindow;
        type ParentType = adw::Window;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ToastableWindow {}

    impl WidgetImpl for ToastableWindow {}
    impl WindowImpl for ToastableWindow {}
    impl AdwWindowImpl for ToastableWindow {}

    impl ToastableWindow {
        fn child_content(&self) -> Option<gtk::Widget> {
            self.toast_overlay.child()
        }

        fn set_child_content(&self, content: Option<&gtk::Widget>) {
            self.toast_overlay.set_child(content);
        }
    }
}

glib::wrapper! {
    /// A window that can display toasts.
    pub struct ToastableWindow(ObjectSubclass<imp::ToastableWindow>)
        @extends gtk::Widget, gtk::Window, adw::Window, gtk::Root, @implements gtk::Accessible;
}

pub trait ToastableWindowExt: 'static {
    /// Get the content of this window.
    #[allow(dead_code)]
    fn child_content(&self) -> Option<gtk::Widget>;

    /// Set content of this window.
    ///
    /// Use this instead of `set_child` or `set_content`, otherwise it will
    /// panic.
    #[allow(dead_code)]
    fn set_child_content(&self, content: Option<&gtk::Widget>);

    /// Add a toast.
    fn add_toast(&self, toast: adw::Toast);
}

impl<O: IsA<ToastableWindow>> ToastableWindowExt for O {
    fn child_content(&self) -> Option<gtk::Widget> {
        self.upcast_ref().child_content()
    }

    fn set_child_content(&self, content: Option<&gtk::Widget>) {
        self.upcast_ref().set_child_content(content);
    }

    fn add_toast(&self, toast: adw::Toast) {
        self.upcast_ref().imp().toast_overlay.add_toast(toast);
    }
}

/// Public trait that must be implemented for everything that derives from
/// `ToastableWindow`.
pub trait ToastableWindowImpl: adw::subclass::prelude::WindowImpl {}

unsafe impl<T> IsSubclassable<T> for ToastableWindow where T: ToastableWindowImpl {}

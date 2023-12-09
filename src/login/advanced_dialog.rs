use std::cell::Cell;

use adw::subclass::prelude::*;
use gtk::{gdk, glib, prelude::*, CompositeTemplate};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/advanced_dialog.ui")]
    #[properties(wrapper_type = super::LoginAdvancedDialog)]
    pub struct LoginAdvancedDialog {
        /// Whether auto-discovery is enabled.
        #[property(get, set, default = true)]
        pub autodiscovery: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginAdvancedDialog {
        const NAME: &'static str = "LoginAdvancedDialog";
        type Type = super::LoginAdvancedDialog;
        type ParentType = adw::PreferencesWindow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.add_binding_action(
                gdk::Key::Escape,
                gdk::ModifierType::empty(),
                "window.close",
                None,
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LoginAdvancedDialog {}

    impl WidgetImpl for LoginAdvancedDialog {}
    impl WindowImpl for LoginAdvancedDialog {}
    impl AdwWindowImpl for LoginAdvancedDialog {}
    impl PreferencesWindowImpl for LoginAdvancedDialog {}
}

glib::wrapper! {
    pub struct LoginAdvancedDialog(ObjectSubclass<imp::LoginAdvancedDialog>)
        @extends gtk::Widget, gtk::Window, adw::Window, adw::PreferencesWindow, @implements gtk::Accessible;
}

impl LoginAdvancedDialog {
    pub fn new(window: &gtk::Window) -> Self {
        glib::Object::builder()
            .property("transient-for", window)
            .build()
    }

    pub async fn run_future(&self) {
        let (sender, receiver) = futures_channel::oneshot::channel();
        let sender = Cell::new(Some(sender));

        self.connect_close_request(move |_| {
            if let Some(sender) = sender.take() {
                sender.send(()).unwrap();
            }
            glib::Propagation::Proceed
        });

        self.present();
        receiver.await.unwrap();
    }
}

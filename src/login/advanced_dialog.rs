use adw::{prelude::*, subclass::prelude::*};
use futures_channel::oneshot;
use gtk::{CompositeTemplate, glib};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/advanced_dialog.ui")]
    #[properties(wrapper_type = super::LoginAdvancedDialog)]
    pub struct LoginAdvancedDialog {
        /// Whether auto-discovery is enabled.
        #[property(get, set, default = true)]
        autodiscovery: Cell<bool>,
        sender: RefCell<Option<oneshot::Sender<()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginAdvancedDialog {
        const NAME: &'static str = "LoginAdvancedDialog";
        type Type = super::LoginAdvancedDialog;
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LoginAdvancedDialog {}

    impl WidgetImpl for LoginAdvancedDialog {}

    impl AdwDialogImpl for LoginAdvancedDialog {
        fn closed(&self) {
            if let Some(sender) = self.sender.take() {
                sender.send(()).expect("receiver was not dropped");
            }
        }
    }

    impl PreferencesDialogImpl for LoginAdvancedDialog {}

    impl LoginAdvancedDialog {
        /// Present this dialog.
        ///
        /// Returns when the dialog is closed.
        pub(super) async fn run_future(&self, parent: &gtk::Widget) {
            let (sender, receiver) = oneshot::channel();
            self.sender.replace(Some(sender));

            self.obj().present(Some(parent));
            receiver.await.expect("sender was not dropped");
        }
    }
}

glib::wrapper! {
    /// A dialog with advanced settings for the login flow.
    pub struct LoginAdvancedDialog(ObjectSubclass<imp::LoginAdvancedDialog>)
        @extends gtk::Widget, adw::Dialog, adw::PreferencesDialog,
        @implements gtk::Accessible;
}

impl LoginAdvancedDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Present this dialog.
    ///
    /// Returns when the dialog is closed.
    pub(crate) async fn run_future(&self, parent: &impl IsA<gtk::Widget>) {
        self.imp().run_future(parent.upcast_ref()).await;
    }
}

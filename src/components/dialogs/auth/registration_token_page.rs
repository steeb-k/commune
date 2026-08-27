use std::fmt::Debug;

use adw::{prelude::*, subclass::prelude::*};
use gtk::glib;

use crate::components::LoadingButton;

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/components/dialogs/auth/registration_token_page.ui"
    )]
    pub struct AuthDialogRegistrationTokenPage {
        #[template_child]
        pub(super) token: TemplateChild<gtk::Entry>,
        #[template_child]
        pub(super) confirm_button: TemplateChild<LoadingButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AuthDialogRegistrationTokenPage {
        const NAME: &'static str = "AuthDialogRegistrationTokenPage";
        type Type = super::AuthDialogRegistrationTokenPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            LoadingButton::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AuthDialogRegistrationTokenPage {}
    impl WidgetImpl for AuthDialogRegistrationTokenPage {}
    impl BinImpl for AuthDialogRegistrationTokenPage {}

    #[gtk::template_callbacks]
    impl AuthDialogRegistrationTokenPage {
        /// Whether the user can proceed given the current state.
        fn can_proceed(&self) -> bool {
            !self.token.text().trim().is_empty()
        }

        /// Update the confirm button for the current state.
        #[template_callback]
        fn update_confirm(&self) {
            self.confirm_button.set_sensitive(self.can_proceed());
        }

        /// Proceed to authentication with the current token.
        #[template_callback]
        fn proceed(&self) {
            if !self.can_proceed() {
                return;
            }

            self.confirm_button.set_is_loading(true);
            let _ = self.obj().activate_action("auth-dialog.continue", None);
        }

        /// Retry this stage.
        pub(super) fn retry(&self) {
            self.confirm_button.set_is_loading(false);
            self.update_confirm();
        }
    }
}

glib::wrapper! {
    /// Page to pass the registration token stage for the [`AuthDialog`].
    pub struct AuthDialogRegistrationTokenPage(ObjectSubclass<imp::AuthDialogRegistrationTokenPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AuthDialogRegistrationTokenPage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Get the default widget of this page.
    pub fn default_widget(&self) -> &gtk::Widget {
        self.imp().confirm_button.upcast_ref()
    }

    /// Get the current token in the entry.
    pub fn token(&self) -> String {
        self.imp().token.text().trim().to_owned()
    }

    /// Retry this stage.
    pub fn retry(&self) {
        self.imp().retry();
    }
}

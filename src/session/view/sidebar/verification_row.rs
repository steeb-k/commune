use adw::subclass::prelude::BinImpl;
use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::model::IdentityVerification;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/sidebar/verification_row.ui")]
    #[properties(wrapper_type = super::VerificationRow)]
    pub struct VerificationRow {
        /// The identity verification represented by this row.
        #[property(get, set = Self::set_identity_verification, explicit_notify, nullable)]
        pub identity_verification: RefCell<Option<IdentityVerification>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VerificationRow {
        const NAME: &'static str = "SidebarVerificationRow";
        type Type = super::VerificationRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for VerificationRow {}

    impl WidgetImpl for VerificationRow {}
    impl BinImpl for VerificationRow {}

    impl VerificationRow {
        /// Set the identity verification represented by this row.
        fn set_identity_verification(&self, verification: Option<IdentityVerification>) {
            if *self.identity_verification.borrow() == verification {
                return;
            }

            self.identity_verification.replace(verification);
            self.obj().notify_identity_verification();
        }
    }
}

glib::wrapper! {
    /// A sidebar row representing an identity verification.
    pub struct VerificationRow(ObjectSubclass<imp::VerificationRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl VerificationRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

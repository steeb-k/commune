use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

mod accept_request_page;
mod cancelled_page;
mod choose_method_page;
mod completed_page;
mod confirm_qr_code_page;
mod no_supported_methods_page;
mod qr_code_scanned_page;
mod sas_emoji;
mod sas_page;
mod scan_qr_code_page;
mod wait_for_other_page;

use self::{
    accept_request_page::AcceptRequestPage, cancelled_page::CancelledPage,
    choose_method_page::ChooseMethodPage, completed_page::CompletedPage,
    confirm_qr_code_page::ConfirmQrCodePage, no_supported_methods_page::NoSupportedMethodsPage,
    qr_code_scanned_page::QrCodeScannedPage, sas_page::SasPage, scan_qr_code_page::ScanQrCodePage,
    wait_for_other_page::WaitForOtherPage,
};
use crate::{
    session::model::{IdentityVerification, VerificationState},
    utils::BoundObject,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/verification_view/identity_verification_view/mod.ui"
    )]
    #[properties(wrapper_type = super::IdentityVerificationView)]
    pub struct IdentityVerificationView {
        /// The current identity verification.
        #[property(get, set = Self::set_verification, explicit_notify, nullable)]
        pub verification: BoundObject<IdentityVerification>,
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub accept_request_page: TemplateChild<AcceptRequestPage>,
        #[template_child]
        pub wait_for_other_page: TemplateChild<WaitForOtherPage>,
        #[template_child]
        pub no_supported_methods_page: TemplateChild<NoSupportedMethodsPage>,
        #[template_child]
        pub choose_method_page: TemplateChild<ChooseMethodPage>,
        #[template_child]
        pub scan_qr_code_page: TemplateChild<ScanQrCodePage>,
        #[template_child]
        pub qr_code_scanned_page: TemplateChild<QrCodeScannedPage>,
        #[template_child]
        pub confirm_qr_code_page: TemplateChild<ConfirmQrCodePage>,
        #[template_child]
        pub sas_page: TemplateChild<SasPage>,
        #[template_child]
        pub completed_page: TemplateChild<CompletedPage>,
        #[template_child]
        pub cancelled_page: TemplateChild<CancelledPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IdentityVerificationView {
        const NAME: &'static str = "IdentityVerificationView";
        type Type = super::IdentityVerificationView;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for IdentityVerificationView {
        fn constructed(&self) {
            self.parent_constructed();

            self.main_stack.connect_transition_running_notify(
                clone!(@weak self as imp => move |stack|
                    if !stack.is_transition_running() {
                        // Focus the default widget when the transition has ended.
                        imp.grab_focus();
                    }
                ),
            );
        }
    }

    impl WidgetImpl for IdentityVerificationView {
        fn grab_focus(&self) -> bool {
            let Some(name) = self.main_stack.visible_child_name() else {
                return false;
            };

            match name.as_str() {
                "accept-request" => self.accept_request_page.grab_focus(),
                "no-supported-methods" => self.no_supported_methods_page.grab_focus(),
                "choose-method" => self.choose_method_page.grab_focus(),
                "confirm-qr-code" => self.confirm_qr_code_page.grab_focus(),
                "sas" => self.sas_page.grab_focus(),
                "completed" => self.completed_page.grab_focus(),
                "cancelled" => self.cancelled_page.grab_focus(),
                _ => false,
            }
        }
    }

    impl BinImpl for IdentityVerificationView {}

    impl IdentityVerificationView {
        /// Set the current identity verification.
        fn set_verification(&self, verification: Option<IdentityVerification>) {
            let prev_verification = self.verification.obj();

            if prev_verification == verification {
                return;
            }
            let obj = self.obj();

            self.verification.disconnect_signals();

            if let Some(verification) = verification {
                let state_handler =
                    verification.connect_state_notify(clone!(@weak obj => move |_| {
                        obj.update_view();
                    }));

                verification.set_was_viewed(true);
                self.verification.set(verification, vec![state_handler]);
            }

            obj.update_view();
            obj.notify_verification();
        }
    }
}

glib::wrapper! {
    /// A view to show the different stages of an identity verification.
    pub struct IdentityVerificationView(ObjectSubclass<imp::IdentityVerificationView>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl IdentityVerificationView {
    pub fn new(verification: &IdentityVerification) -> Self {
        glib::Object::builder()
            .property("verification", verification)
            .build()
    }

    /// Update this view for the current state of the verification.
    fn update_view(&self) {
        let Some(verification) = self.verification() else {
            return;
        };
        let imp = self.imp();

        match verification.state() {
            VerificationState::Created => {
                imp.wait_for_other_page.reset();
                imp.main_stack
                    .set_visible_child_name("wait-for-other-party");
            }
            VerificationState::Requested => {
                imp.accept_request_page.reset();
                imp.main_stack.set_visible_child_name("accept-request");
            }
            VerificationState::NoSupportedMethods => {
                imp.no_supported_methods_page.reset();
                imp.main_stack
                    .set_visible_child_name("no-supported-methods");
            }
            VerificationState::Ready => {
                imp.choose_method_page.reset();
                imp.main_stack.set_visible_child_name("choose-method");
            }
            VerificationState::QrScan => {
                imp.scan_qr_code_page.reset();
                imp.main_stack.set_visible_child_name("scan-qr-code");
            }
            VerificationState::QrScanned => {
                imp.qr_code_scanned_page.reset();
                imp.main_stack.set_visible_child_name("qr-code-scanned");
            }
            VerificationState::QrConfirm => {
                imp.confirm_qr_code_page.reset();
                imp.main_stack.set_visible_child_name("confirm-qr-code");
            }
            VerificationState::SasConfirm => {
                imp.sas_page.reset();
                imp.main_stack.set_visible_child_name("sas");
            }
            VerificationState::Done => {
                imp.main_stack.set_visible_child_name("completed");
            }
            VerificationState::Cancelled | VerificationState::Error => {
                imp.cancelled_page.reset();
                imp.main_stack.set_visible_child_name("cancelled");
            }
            // Nothing to do, this view should be closed.
            VerificationState::Dismissed => {}
        }
    }
}

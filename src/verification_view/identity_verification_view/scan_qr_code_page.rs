use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};
use matrix_sdk::encryption::verification::QrVerificationData;

use crate::{
    components::SpinnerButton,
    contrib::QrCodeScanner,
    gettext_f,
    prelude::*,
    session::model::{IdentityVerification, VerificationSupportedMethods},
    spawn, toast,
    utils::BoundObjectWeakRef,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/verification_view/identity_verification_view/scan_qr_code_page.ui"
    )]
    #[properties(wrapper_type = super::ScanQrCodePage)]
    pub struct ScanQrCodePage {
        /// The current identity verification.
        #[property(get, set = Self::set_verification, explicit_notify, nullable)]
        pub verification: BoundObjectWeakRef<IdentityVerification>,
        pub display_name_handler: RefCell<Option<glib::SignalHandlerId>>,
        #[template_child]
        pub title: TemplateChild<gtk::Label>,
        #[template_child]
        pub instructions: TemplateChild<gtk::Label>,
        #[template_child]
        pub qr_code_scanner: TemplateChild<QrCodeScanner>,
        #[template_child]
        pub show_qr_code_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub start_sas_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub cancel_btn: TemplateChild<SpinnerButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ScanQrCodePage {
        const NAME: &'static str = "IdentityVerificationScanQrCodePage";
        type Type = super::ScanQrCodePage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ScanQrCodePage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.qr_code_scanner
                .connect_code_detected(clone!(@weak obj => move |_, data| {
                    obj.code_detected(data);
                }));
        }

        fn dispose(&self) {
            if let Some(verification) = self.verification.obj() {
                if let Some(handler) = self.display_name_handler.take() {
                    verification.user().disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for ScanQrCodePage {
        fn map(&self) {
            self.parent_map();

            spawn!(clone!(@weak self as imp => async move {
                imp.qr_code_scanner.start().await;
            }));
        }

        fn unmap(&self) {
            self.qr_code_scanner.stop();
            self.parent_unmap();
        }
    }

    impl BinImpl for ScanQrCodePage {}

    impl ScanQrCodePage {
        /// Set the current identity verification.
        fn set_verification(&self, verification: Option<IdentityVerification>) {
            let prev_verification = self.verification.obj();

            if prev_verification == verification {
                return;
            }
            let obj = self.obj();

            if let Some(verification) = prev_verification {
                if let Some(handler) = self.display_name_handler.take() {
                    verification.user().disconnect(handler);
                }
            }
            self.verification.disconnect_signals();

            if let Some(verification) = &verification {
                let display_name_handler =
                    verification
                        .user()
                        .connect_display_name_notify(clone!(@weak obj => move |_| {
                            obj.update_labels();
                        }));
                self.display_name_handler
                    .replace(Some(display_name_handler));

                let supported_methods_handler =
                    verification.connect_supported_methods_notify(clone!(@weak obj => move |_| {
                        obj.update_page();
                    }));

                self.verification
                    .set(verification, vec![supported_methods_handler]);
            }

            obj.update_labels();
            obj.update_page();
            obj.notify_verification()
        }
    }
}

glib::wrapper! {
    /// A page to scan a QR code.
    pub struct ScanQrCodePage(ObjectSubclass<imp::ScanQrCodePage>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl ScanQrCodePage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Update the labels for the current verification.
    fn update_labels(&self) {
        let Some(verification) = self.verification() else {
            return;
        };
        let imp = self.imp();

        if verification.is_self_verification() {
            imp.title.set_label(&gettext("Verify Session"));
            imp.instructions
                .set_label(&gettext("Scan the QR code displayed by the other session."));
        } else {
            let name = verification.user().display_name();
            imp.title.set_markup(&gettext("Verification Request"));
            imp.instructions.set_markup(&gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                "Scan the QR code shown on the device of {user}.",
                &[("user", &format!("<b>{name}</b>"))],
            ));
        }
    }

    /// Update the UI for the available verification methods.
    fn update_page(&self) {
        let Some(verification) = self.verification() else {
            return;
        };
        let imp = self.imp();
        let supported_methods = verification.supported_methods();

        let show_qr_code_visible = supported_methods
            .contains(VerificationSupportedMethods::QR_SHOW)
            && verification.has_qr_code();
        let sas_visible = supported_methods.contains(VerificationSupportedMethods::SAS);

        imp.show_qr_code_btn.set_visible(show_qr_code_visible);
        imp.start_sas_btn.set_visible(sas_visible);
    }

    /// Reset the UI to its initial state.
    pub fn reset(&self) {
        let imp = self.imp();

        imp.start_sas_btn.set_loading(false);
        imp.cancel_btn.set_loading(false);

        self.set_sensitive(true);
    }

    /// Handle a detected QR Code.
    fn code_detected(&self, data: QrVerificationData) {
        let Some(verification) = self.verification() else {
            return;
        };

        spawn!(clone!(@weak self as obj, @weak verification => async move {
            if verification.qr_code_scanned(data).await.is_err() {
                toast!(obj, gettext("Could not validate scanned QR Code"));
            }
        }));
    }

    /// Switch to the screen to scan a QR Code.
    #[template_callback]
    fn show_qr_code(&self) {
        let Some(verification) = self.verification() else {
            return;
        };

        verification.choose_method();
    }

    /// Start a SAS verification.
    #[template_callback]
    fn start_sas(&self) {
        let Some(verification) = self.verification() else {
            return;
        };
        let imp = self.imp();

        imp.start_sas_btn.set_loading(true);
        self.set_sensitive(false);

        spawn!(clone!(@weak self as obj, @weak verification => async move {
            if verification.start_sas().await.is_err() {
                toast!(obj, gettext("Could not start emoji verification"));
                obj.reset();
            }
        }));
    }

    /// Cancel the verification.
    #[template_callback]
    fn cancel(&self) {
        let Some(verification) = self.verification() else {
            return;
        };

        self.imp().cancel_btn.set_loading(true);
        self.set_sensitive(false);

        spawn!(clone!(@weak self as obj, @weak verification => async move {
            if verification.cancel().await.is_err() {
                toast!(obj, gettext("Could not cancel the verification"));
                obj.reset();
            }
        }));
    }
}

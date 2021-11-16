use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use log::warn;

use crate::components::SpinnerButton;
use crate::contrib::screenshot;
use crate::contrib::QRCode;
use crate::contrib::QRCodeExt;
use crate::contrib::QrCodeScanner;
use crate::session::verification::{Emoji, IdentityVerification, VerificationMode};
use crate::spawn;
use gettextrs::gettext;
use matrix_sdk::encryption::verification::QrVerificationData;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use glib::SignalHandlerId;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/incoming-verification.ui")]
    pub struct IncomingVerification {
        pub request: RefCell<Option<IdentityVerification>>,
        #[template_child]
        pub qrcode: TemplateChild<QRCode>,
        #[template_child]
        pub emoji_row_1: TemplateChild<gtk::Box>,
        #[template_child]
        pub emoji_row_2: TemplateChild<gtk::Box>,
        #[template_child]
        pub emoji_match_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub emoji_not_match_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub start_emoji_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub start_emoji_btn2: TemplateChild<SpinnerButton>,
        #[template_child]
        pub start_emoji_btn3: TemplateChild<SpinnerButton>,
        #[template_child]
        pub scan_qr_code_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub accept_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub dismiss_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub take_screenshot_btn2: TemplateChild<SpinnerButton>,
        #[template_child]
        pub take_screenshot_btn3: TemplateChild<SpinnerButton>,
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub qr_code_scanner: TemplateChild<QrCodeScanner>,
        #[template_child]
        pub done_btn: TemplateChild<gtk::Button>,
        pub mode_handler: RefCell<Option<SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IncomingVerification {
        const NAME: &'static str = "IncomingVerification";
        type Type = super::IncomingVerification;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            SpinnerButton::static_type();
            QRCode::static_type();
            Emoji::static_type();
            QrCodeScanner::static_type();

            klass.install_action("verification.dismiss", None, move |obj, _, _| {
                obj.dismiss();
            });

            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for IncomingVerification {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "request",
                    "Request",
                    "The Object holding the data for the verification",
                    IdentityVerification::static_type(),
                    glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "request" => obj.set_request(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "request" => obj.request().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);
            self.accept_btn
                .connect_clicked(clone!(@weak obj => move |button| {
                    let priv_ = imp::IncomingVerification::from_instance(&obj);
                    button.set_loading(true);
                    priv_.dismiss_btn.set_sensitive(false);
                    obj.accept();
                }));

            self.emoji_match_btn
                .connect_clicked(clone!(@weak obj => move |button| {
                    let priv_ = imp::IncomingVerification::from_instance(&obj);
                    button.set_loading(true);
                    priv_.emoji_not_match_btn.set_sensitive(false);
                    if let Some(request) = obj.request() {
                        request.emoji_match();
                    }
                }));

            self.emoji_not_match_btn
                .connect_clicked(clone!(@weak obj => move |button| {
                    let priv_ = imp::IncomingVerification::from_instance(&obj);
                    button.set_loading(true);
                    priv_.emoji_match_btn.set_sensitive(false);
                    if let Some(request) = obj.request() {
                        request.emoji_not_match();
                    }
                }));

            self.start_emoji_btn
                .connect_clicked(clone!(@weak obj => move |button| {
                    let priv_ = imp::IncomingVerification::from_instance(&obj);
                    button.set_loading(true);
                    priv_.scan_qr_code_btn.set_sensitive(false);
                    if let Some(request) = obj.request() {
                        request.start_sas();
                    }
                }));
            self.start_emoji_btn2
                .connect_clicked(clone!(@weak obj => move |button| {
                    let priv_ = imp::IncomingVerification::from_instance(&obj);
                    button.set_loading(true);
                    priv_.take_screenshot_btn2.set_sensitive(false);
                    if let Some(request) = obj.request() {
                        request.start_sas();
                    }
                }));
            self.start_emoji_btn3
                .connect_clicked(clone!(@weak obj => move |button| {
                    let priv_ = imp::IncomingVerification::from_instance(&obj);
                    button.set_loading(true);
                    priv_.take_screenshot_btn3.set_sensitive(false);
                    if let Some(request) = obj.request() {
                        request.start_sas();
                    }
                }));

            self.scan_qr_code_btn
                .connect_clicked(clone!(@weak obj => move |button| {
                    let priv_ = imp::IncomingVerification::from_instance(&obj);
                    button.set_loading(true);
                    priv_.start_emoji_btn.set_sensitive(false);
                    if priv_.qr_code_scanner.has_camera() {
                        obj.start_scanning();
                    } else {
                        obj.take_screenshot();
                    }
                }));

            self.take_screenshot_btn2
                .connect_clicked(clone!(@weak obj => move |button| {
                    let priv_ = imp::IncomingVerification::from_instance(&obj);
                    button.set_loading(true);
                    priv_.start_emoji_btn2.set_sensitive(false);
                    obj.take_screenshot();
                }));

            self.take_screenshot_btn3
                .connect_clicked(clone!(@weak obj => move |button| {
                    let priv_ = imp::IncomingVerification::from_instance(&obj);
                    button.set_loading(true);
                    priv_.start_emoji_btn3.set_sensitive(false);
                    obj.take_screenshot();
                }));

            self.done_btn.connect_clicked(clone!(@weak obj => move |_| {
                obj.dismiss();
            }));

            self.qr_code_scanner
                .connect_code_detected(clone!(@weak obj => move |_, data| {
                    obj.finish_scanning(data);
                }));

            self.qr_code_scanner.connect_notify_local(
                Some("has-camera"),
                clone!(@weak obj => move |_, _| {
                    obj.update_camera_state();
                }),
            );
            obj.update_camera_state();
        }

        fn dispose(&self, obj: &Self::Type) {
            if let Some(request) = obj.request() {
                if let Some(handler) = self.mode_handler.take() {
                    request.disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for IncomingVerification {
        fn map(&self, widget: &Self::Type) {
            self.parent_map(widget);
            widget.update_view();
        }
    }
    impl BinImpl for IncomingVerification {}
}

glib::wrapper! {
    pub struct IncomingVerification(ObjectSubclass<imp::IncomingVerification>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl IncomingVerification {
    pub fn new(request: &IdentityVerification) -> Self {
        glib::Object::new(&[("request", request)]).expect("Failed to create IncomingVerification")
    }

    pub fn request(&self) -> Option<IdentityVerification> {
        let priv_ = imp::IncomingVerification::from_instance(self);
        priv_.request.borrow().clone()
    }

    pub fn set_request(&self, request: Option<IdentityVerification>) {
        let priv_ = imp::IncomingVerification::from_instance(self);
        let previous_request = self.request();

        if previous_request == request {
            return;
        }

        self.reset();

        if let Some(previous_request) = previous_request {
            if let Some(handler) = priv_.mode_handler.take() {
                previous_request.disconnect(handler);
            }
        }

        if let Some(ref request) = request {
            let handler = request.connect_notify_local(
                Some("mode"),
                clone!(@weak self as obj => move |_, _| {
                    obj.update_view();
                }),
            );
            self.update_view();

            priv_.mode_handler.replace(Some(handler));
        }

        priv_.request.replace(request);
        self.notify("request");
    }

    fn reset(&self) {
        let priv_ = imp::IncomingVerification::from_instance(self);
        priv_.accept_btn.set_loading(false);
        priv_.accept_btn.set_sensitive(true);
        priv_.dismiss_btn.set_sensitive(true);
        priv_.scan_qr_code_btn.set_loading(false);
        priv_.scan_qr_code_btn.set_sensitive(true);
        priv_.emoji_not_match_btn.set_loading(false);
        priv_.emoji_not_match_btn.set_sensitive(true);
        priv_.emoji_match_btn.set_loading(false);
        priv_.emoji_match_btn.set_sensitive(true);
        priv_.start_emoji_btn.set_loading(false);
        priv_.start_emoji_btn.set_sensitive(true);
        priv_.start_emoji_btn2.set_loading(false);
        priv_.start_emoji_btn2.set_sensitive(true);
        priv_.start_emoji_btn3.set_loading(false);
        priv_.start_emoji_btn3.set_sensitive(true);
        priv_.take_screenshot_btn2.set_loading(false);
        priv_.take_screenshot_btn2.set_sensitive(true);
        priv_.take_screenshot_btn3.set_loading(false);
        priv_.take_screenshot_btn3.set_sensitive(true);

        self.clean_emoji();
    }

    fn clean_emoji(&self) {
        let priv_ = imp::IncomingVerification::from_instance(self);

        while let Some(child) = priv_.emoji_row_1.first_child() {
            priv_.emoji_row_1.remove(&child);
        }

        while let Some(child) = priv_.emoji_row_2.first_child() {
            priv_.emoji_row_2.remove(&child);
        }
    }

    pub fn accept(&self) {
        if let Some(request) = self.request() {
            request.accept_incoming();
        }
    }

    pub fn dismiss(&self) {
        if let Some(request) = self.request() {
            request.dismiss();
        }
    }

    fn update_view(&self) {
        let priv_ = imp::IncomingVerification::from_instance(self);
        if let Some(request) = self.request() {
            match request.mode() {
                VerificationMode::IdentityNotFound => {
                    // TODO: what should we do if we don't find the identity
                }
                VerificationMode::Requested => {
                    priv_.main_stack.set_visible_child_name("accept-request");
                }
                VerificationMode::QrV1Show => {
                    if let Some(qrcode) = request.qr_code() {
                        priv_.qrcode.set_qrcode(qrcode);
                        priv_.main_stack.set_visible_child_name("qrcode");
                    } else {
                        warn!("Failed to get qrcode for QrVerification");
                        request.start_sas();
                    }
                }
                VerificationMode::QrV1Scan => {
                    self.start_scanning();
                }
                VerificationMode::SasV1 => {
                    self.clean_emoji();
                    // TODO: implement sas fallback when emojis arn't supported
                    if let Some(emoji) = request.emoji() {
                        for (index, emoji) in emoji.iter().enumerate() {
                            if index < 4 {
                                priv_.emoji_row_1.append(&Emoji::new(emoji));
                            } else {
                                priv_.emoji_row_2.append(&Emoji::new(emoji));
                            }
                        }
                        priv_.main_stack.set_visible_child_name("emoji");
                    }
                }
                VerificationMode::Completed => {
                    priv_.main_stack.set_visible_child_name("completed");
                }
                _ => {}
            }
        }
    }

    fn start_scanning(&self) {
        spawn!(clone!(@weak self as obj => async move {
            let priv_ = imp::IncomingVerification::from_instance(&obj);
            if priv_.qr_code_scanner.start().await {
                priv_.main_stack.set_visible_child_name("scan-qr-code");
            } else {
                priv_.main_stack.set_visible_child_name("no-camera");
            }
        }));
    }

    fn take_screenshot(&self) {
        spawn!(clone!(@weak self as obj => async move {
            let root = obj.root().unwrap();
            if let Some(code) = screenshot::capture(&root).await {
                obj.finish_scanning(code);
            } else {
                obj.reset();
            }
        }));
    }

    fn finish_scanning(&self, data: QrVerificationData) {
        let priv_ = imp::IncomingVerification::from_instance(self);
        priv_.qr_code_scanner.stop();
        if let Some(request) = self.request() {
            request.scanned_qr_code(data);
        }
        priv_.main_stack.set_visible_child_name("qr-code-scanned");
    }

    fn update_camera_state(&self) {
        let priv_ = imp::IncomingVerification::from_instance(self);
        if priv_.qr_code_scanner.has_camera() {
            priv_
                .scan_qr_code_btn
                .set_label(&gettext("Scan QR code with this session"))
        } else {
            priv_
                .scan_qr_code_btn
                .set_label(&gettext("Take a Screenshot of a Qr Code"))
        }
    }
}

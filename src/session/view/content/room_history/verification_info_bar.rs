use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

use crate::{
    gettext_f,
    prelude::*,
    session::model::{IdentityVerification, VerificationState},
    spawn, toast, Window,
};

mod imp {
    use std::cell::RefCell;

    use glib::{subclass::InitializingObject, SignalHandlerId};

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/verification_info_bar.ui"
    )]
    #[properties(wrapper_type = super::VerificationInfoBar)]
    pub struct VerificationInfoBar {
        #[template_child]
        pub revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub label: TemplateChild<gtk::Label>,
        #[template_child]
        pub accept_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub cancel_btn: TemplateChild<gtk::Button>,
        /// The identity verification presented by this info bar.
        #[property(get, set = Self::set_verification, explicit_notify)]
        pub verification: RefCell<Option<IdentityVerification>>,
        pub state_handler: RefCell<Option<SignalHandlerId>>,
        pub user_handler: RefCell<Option<SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VerificationInfoBar {
        const NAME: &'static str = "ContentVerificationInfoBar";
        type Type = super::VerificationInfoBar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("infobar");
            Self::bind_template(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Group);

            klass.install_action("verification.accept", None, move |obj, _, _| {
                let Some(window) = obj.root().and_downcast::<Window>() else {
                    return;
                };
                let Some(verification) = obj.verification() else {
                    return;
                };

                if verification.state() == VerificationState::Requested {
                    window.session_view().select_verification(verification);
                } else {
                    spawn!(
                        clone!(@weak obj, @weak verification, @weak window => async move {
                            if verification.accept().await.is_err() {
                                toast!(obj, gettext("Failed to accept verification"));
                            } else {
                                window.session_view().select_verification(verification);
                            }
                        })
                    );
                }
            });

            klass.install_action("verification.decline", None, move |obj, _, _| {
                let Some(verification) = obj.verification() else {
                    return;
                };

                spawn!(clone!(@weak obj, @weak verification => async move {
                    if verification.cancel().await.is_err() {
                        toast!(obj, gettext("Failed to decline verification"));
                    }
                }));
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for VerificationInfoBar {}

    impl WidgetImpl for VerificationInfoBar {}
    impl BinImpl for VerificationInfoBar {}

    impl VerificationInfoBar {
        /// Set the identity verification presented by this info bar.
        fn set_verification(&self, verification: Option<IdentityVerification>) {
            if *self.verification.borrow() == verification {
                return;
            }
            let obj = self.obj();

            if let Some(old_verification) = &*self.verification.borrow() {
                if let Some(handler) = self.state_handler.take() {
                    old_verification.disconnect(handler);
                }

                if let Some(handler) = self.user_handler.take() {
                    old_verification.user().disconnect(handler);
                }
            }

            if let Some(verification) = &verification {
                let handler = verification.connect_state_notify(clone!(@weak obj => move |_| {
                    obj.update_view();
                }));

                self.state_handler.replace(Some(handler));

                let handler =
                    verification
                        .user()
                        .connect_display_name_notify(clone!(@weak obj => move |_| {
                            obj.update_view();
                        }));

                self.user_handler.replace(Some(handler));
            }

            self.verification.replace(verification);

            obj.update_view();
            obj.notify_verification();
        }
    }
}

glib::wrapper! {
    /// An info bar presenting an ongoing identity verification.
    pub struct VerificationInfoBar(ObjectSubclass<imp::VerificationInfoBar>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl VerificationInfoBar {
    pub fn new(label: String) -> Self {
        glib::Object::builder().property("label", &label).build()
    }

    pub fn update_view(&self) {
        let imp = self.imp();
        let visible = if let Some(verification) = self.verification() {
            if verification.is_finished() {
                false
            } else if matches!(verification.state(), VerificationState::Requested) {
                imp.label.set_markup(&gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', this is a
                    // variable name.
                    "{user_name} wants to be verified",
                    &[(
                        "user_name",
                        &format!("<b>{}</b>", verification.user().display_name()),
                    )],
                ));
                imp.accept_btn.set_label(&gettext("Verify"));
                imp.cancel_btn.set_label(&gettext("Decline"));
                true
            } else {
                imp.label.set_label(&gettext("Verification in progress"));
                imp.accept_btn.set_label(&gettext("Continue"));
                imp.cancel_btn.set_label(&gettext("Cancel"));
                true
            }
        } else {
            false
        };

        imp.revealer.set_reveal_child(visible);
    }
}

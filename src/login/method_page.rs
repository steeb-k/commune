use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{self, glib, glib::clone, CompositeTemplate};
use ruma::api::client::session::get_login_types::v3::LoginType;
use tracing::warn;

use super::{idp_button::IdpButton, Login};
use crate::{
    components::LoadingButton, gettext_f, prelude::*, spawn_tokio, toast, utils::BoundObjectWeakRef,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/method_page.ui")]
    #[properties(wrapper_type = super::LoginMethodPage)]
    pub struct LoginMethodPage {
        #[template_child]
        title: TemplateChild<gtk::Label>,
        #[template_child]
        username_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        password_entry: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        sso_idp_box: TemplateChild<gtk::Box>,
        sso_idp_box_children: RefCell<Vec<IdpButton>>,
        #[template_child]
        more_sso_btn: TemplateChild<gtk::Button>,
        #[template_child]
        next_button: TemplateChild<LoadingButton>,
        /// The parent `Login` object.
        #[property(get, set = Self::set_login, nullable)]
        login: BoundObjectWeakRef<Login>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginMethodPage {
        const NAME: &'static str = "LoginMethodPage";
        type Type = super::LoginMethodPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LoginMethodPage {}

    impl WidgetImpl for LoginMethodPage {
        fn grab_focus(&self) -> bool {
            self.username_entry.grab_focus()
        }
    }

    impl NavigationPageImpl for LoginMethodPage {
        fn shown(&self) {
            self.grab_focus();
        }
    }

    #[gtk::template_callbacks]
    impl LoginMethodPage {
        /// Set the parent `Login` object.
        fn set_login(&self, login: Option<&Login>) {
            self.login.disconnect_signals();

            if let Some(login) = login {
                let domain_handler = login.connect_domain_string_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_domain_name();
                    }
                ));
                let login_types_handler = login.connect_login_types_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_sso();
                    }
                ));

                self.login
                    .set(login, vec![domain_handler, login_types_handler]);
            }

            self.update_domain_name();
            self.update_sso();
            self.update_next_state();
        }

        /// The username entered by the user.
        fn username(&self) -> glib::GString {
            self.username_entry.text()
        }

        /// The password entered by the user.
        fn password(&self) -> glib::GString {
            self.password_entry.text()
        }

        /// Update the domain name displayed in the title.
        fn update_domain_name(&self) {
            let Some(login) = self.login.obj() else {
                return;
            };

            let title = &self.title;
            if let Some(domain) = login.domain_string() {
                title.set_markup(&gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', this is a
                    // variable name.
                    "Log in to {domain_name}",
                    &[(
                        "domain_name",
                        &format!("<span segment=\"word\">{domain}</span>"),
                    )],
                ));
            } else {
                title.set_markup(&gettext("Log in"));
            }
        }

        /// Update the SSO group.
        fn update_sso(&self) {
            let Some(login) = self.login.obj() else {
                return;
            };

            let login_types = login.login_types();
            let Some(sso_login) = login_types.into_iter().find_map(|t| match t {
                LoginType::Sso(sso) => Some(sso),
                _ => None,
            }) else {
                self.sso_idp_box.set_visible(false);
                self.more_sso_btn.set_visible(false);
                return;
            };

            self.clean_idp_box();

            let mut has_unknown_methods = false;
            let mut has_known_methods = false;

            if !sso_login.identity_providers.is_empty() {
                let mut sso_idp_box_children = self.sso_idp_box_children.borrow_mut();
                sso_idp_box_children.reserve(sso_login.identity_providers.len());

                for provider in &sso_login.identity_providers {
                    if let Some(btn) = IdpButton::new(provider) {
                        self.sso_idp_box.append(&btn);
                        sso_idp_box_children.push(btn);

                        has_known_methods = true;
                    } else {
                        has_unknown_methods = true;
                    }
                }
            }
            self.sso_idp_box.set_visible(has_known_methods);

            if has_known_methods {
                self.more_sso_btn.set_label(&gettext("More SSO Providers"));
                self.more_sso_btn.set_visible(has_unknown_methods);
            } else {
                self.more_sso_btn.set_label(&gettext("Login via SSO"));
                self.more_sso_btn.set_visible(true);
            }
        }

        /// Whether the current state allows to login with a password.
        fn can_login_with_password(&self) -> bool {
            let username_length = self.username().len();
            let password_length = self.password().len();
            username_length != 0 && password_length != 0
        }

        /// Update the state of the "Next" button.
        #[template_callback]
        fn update_next_state(&self) {
            self.next_button
                .set_sensitive(self.can_login_with_password());
        }

        /// Login with the password login type.
        #[template_callback]
        async fn login_with_password(&self) {
            if !self.can_login_with_password() {
                return;
            }

            let Some(login) = self.login.obj() else {
                return;
            };

            self.next_button.set_is_loading(true);
            login.freeze();

            let username = self.username();
            let password = self.password();

            let client = login.client().await.unwrap();
            let handle = spawn_tokio!(async move {
                client
                    .matrix_auth()
                    .login_username(&username, &password)
                    .initial_device_display_name("Fractal")
                    .send()
                    .await
            });

            match handle.await.unwrap() {
                Ok(response) => {
                    login.handle_login_response(response).await;
                }
                Err(error) => {
                    warn!("Could not log in: {error}");
                    let obj = self.obj();
                    toast!(obj, error.to_user_facing());
                }
            }

            self.next_button.set_is_loading(false);
            login.unfreeze();
        }

        /// Reset this page.
        pub(super) fn clean(&self) {
            self.username_entry.set_text("");
            self.password_entry.set_text("");
            self.next_button.set_is_loading(false);
            self.update_next_state();
            self.clean_idp_box();
        }

        /// Empty the identity providers box.
        fn clean_idp_box(&self) {
            for child in self.sso_idp_box_children.borrow_mut().drain(..) {
                self.sso_idp_box.remove(&child);
            }
        }
    }
}

glib::wrapper! {
    /// The login page allowing to login via password or to choose a SSO provider.
    pub struct LoginMethodPage(ObjectSubclass<imp::LoginMethodPage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

impl LoginMethodPage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Reset this page.
    pub(crate) fn clean(&self) {
        self.imp().clean();
    }
}

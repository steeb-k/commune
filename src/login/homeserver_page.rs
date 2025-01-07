use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{self, glib, glib::clone, CompositeTemplate};
use matrix_sdk::{
    config::RequestConfig, sanitize_server_name, Client, ClientBuildError, ClientBuilder,
};
use tracing::warn;
use url::Url;

use super::Login;
use crate::{
    components::{LoadingButton, OfflineBanner},
    gettext_f,
    prelude::*,
    spawn_tokio, toast,
    utils::BoundObjectWeakRef,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/homeserver_page.ui")]
    #[properties(wrapper_type = super::LoginHomeserverPage)]
    pub struct LoginHomeserverPage {
        #[template_child]
        homeserver_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        homeserver_help: TemplateChild<gtk::Label>,
        #[template_child]
        next_button: TemplateChild<LoadingButton>,
        /// The parent `Login` object.
        #[property(get, set = Self::set_login, explicit_notify, nullable)]
        login: BoundObjectWeakRef<Login>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginHomeserverPage {
        const NAME: &'static str = "LoginHomeserverPage";
        type Type = super::LoginHomeserverPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            OfflineBanner::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for LoginHomeserverPage {}

    impl WidgetImpl for LoginHomeserverPage {
        fn grab_focus(&self) -> bool {
            self.homeserver_entry.grab_focus()
        }
    }

    impl NavigationPageImpl for LoginHomeserverPage {
        fn shown(&self) {
            self.grab_focus();
        }
    }

    #[gtk::template_callbacks]
    impl LoginHomeserverPage {
        /// Set the parent `Login` object.
        fn set_login(&self, login: Option<&Login>) {
            self.login.disconnect_signals();

            if let Some(login) = login {
                let handler = login.connect_autodiscovery_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_next_state();
                        imp.update_text();
                    }
                ));

                self.login.set(login, vec![handler]);
            }

            self.update_next_state();
            self.update_text();
        }

        /// Update the text of this page according to the current settings.
        fn update_text(&self) {
            let Some(login) = self.login.obj() else {
                return;
            };

            if login.autodiscovery() {
                self.homeserver_entry.set_title(&gettext("Domain Name"));
                self.homeserver_help.set_markup(&gettext(
                    "The domain of your Matrix homeserver, for example gnome.org",
                ));
            } else {
                self.homeserver_entry.set_title(&gettext("Homeserver URL"));
                self.homeserver_help.set_markup(&gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', this is a
                    // variable name.
                    "The URL of your Matrix homeserver, for example {address}",
                    &[(
                        "address",
                        "<span segment=\"word\">https://gnome.modular.im</span>",
                    )],
                ));
            }
        }

        /// Reset this page.
        pub(super) fn clean(&self) {
            self.homeserver_entry.set_text("");
            self.next_button.set_is_loading(false);
            self.update_next_state();
        }

        /// Whether the current state allows to go to the next step.
        fn can_go_next(&self) -> bool {
            let Some(login) = self.login.obj() else {
                return false;
            };
            let homeserver = self.homeserver_entry.text();

            if login.autodiscovery() {
                sanitize_server_name(homeserver.as_str()).is_ok()
            } else {
                Url::parse(homeserver.as_str()).is_ok()
            }
        }

        /// Update the state of the "Next" button.
        #[template_callback]
        fn update_next_state(&self) {
            self.next_button.set_sensitive(self.can_go_next());
        }

        /// Fetch the login details of the homeserver.
        #[template_callback]
        async fn fetch_homeserver_details(&self) {
            self.check_homeserver().await;
        }

        /// Check if the homeserver that was entered is valid.
        pub(super) async fn check_homeserver(&self) {
            if !self.can_go_next() {
                return;
            }

            let Some(login) = self.login.obj() else {
                return;
            };

            self.next_button.set_is_loading(true);
            login.freeze();

            let autodiscovery = login.autodiscovery();

            let res = if autodiscovery {
                self.discover_homeserver().await
            } else {
                self.detect_homeserver().await
            };

            match res {
                Ok(client) => {
                    let server_name = autodiscovery
                        .then(|| self.homeserver_entry.text())
                        .and_then(|s| sanitize_server_name(&s).ok());

                    login.set_domain(server_name);
                    login.set_client(Some(client.clone()));

                    self.homeserver_login_types(client).await;
                }
                Err(error) => {
                    let obj = self.obj();
                    toast!(obj, error.to_user_facing());
                }
            };

            self.next_button.set_is_loading(false);
            login.unfreeze();
        }

        /// Try to discover the homeserver.
        async fn discover_homeserver(&self) -> Result<Client, ClientBuildError> {
            let homeserver = self.homeserver_entry.text();
            let handle = spawn_tokio!(async move {
                Self::client_builder()
                    .server_name_or_homeserver_url(homeserver)
                    .build()
                    .await
            });

            match handle.await.expect("task was not aborted") {
                Ok(client) => Ok(client),
                Err(error) => {
                    warn!("Could not discover homeserver: {error}");
                    Err(error)
                }
            }
        }

        /// Check if the URL points to a homeserver.
        async fn detect_homeserver(&self) -> Result<Client, ClientBuildError> {
            let homeserver = self.homeserver_entry.text();
            spawn_tokio!(async move {
                let client = Self::client_builder()
                    .respect_login_well_known(false)
                    .homeserver_url(homeserver)
                    .build()
                    .await?;

                // This method calls the `GET /versions` endpoint if it was not called
                // previously.
                client.unstable_features().await?;

                Ok(client)
            })
            .await
            .expect("task was not aborted")
        }

        /// Fetch the login types supported by the homeserver.
        async fn homeserver_login_types(&self, client: Client) {
            let Some(login) = self.login.obj() else {
                return;
            };

            let handle = spawn_tokio!(async move { client.matrix_auth().get_login_types().await });

            match handle.await.expect("task was not aborted") {
                Ok(res) => {
                    login.set_login_types(res.flows);
                    login.show_login_screen();
                }
                Err(error) => {
                    warn!("Could not get available login types: {error}");
                    let obj = self.obj();
                    toast!(obj, "Could not get available login types");

                    // Drop the client because it is bound to the homeserver.
                    login.drop_client();
                }
            };
        }

        /// Construct a [`ClientBuilder`] with the proper configuration.
        fn client_builder() -> ClientBuilder {
            Client::builder().request_config(RequestConfig::new().retry_limit(2))
        }
    }
}

glib::wrapper! {
    /// The login page to provide the homeserver and login settings.
    pub struct LoginHomeserverPage(ObjectSubclass<imp::LoginHomeserverPage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

impl LoginHomeserverPage {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Reset this page.
    pub(crate) fn clean(&self) {
        self.imp().clean();
    }

    /// Check if the homeserver that was entered is valid.
    pub(crate) async fn check_homeserver(&self) {
        self.imp().check_homeserver().await;
    }
}

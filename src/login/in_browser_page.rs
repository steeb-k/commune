use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, CompositeTemplate};
use ruma::api::client::session::SsoRedirectOidcAction;
use tracing::{error, warn};
use url::Url;

use super::Login;
use crate::{prelude::*, spawn, spawn_tokio, toast};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/in_browser_page.ui")]
    #[properties(wrapper_type = super::LoginInBrowserPage)]
    pub struct LoginInBrowserPage {
        #[template_child]
        continue_btn: TemplateChild<gtk::Button>,
        /// The ancestor `Login` object.
        #[property(get, set, nullable)]
        login: glib::WeakRef<Login>,
        /// Whether we are logging in with OIDC compatibility.
        #[property(get, set)]
        oidc_compatibility: Cell<bool>,
        /// The identity provider to use when logging in with SSO.
        #[property(get, set, nullable)]
        sso_idp_id: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginInBrowserPage {
        const NAME: &'static str = "LoginInBrowserPage";
        type Type = super::LoginInBrowserPage;
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
    impl ObjectImpl for LoginInBrowserPage {}

    impl WidgetImpl for LoginInBrowserPage {
        fn grab_focus(&self) -> bool {
            self.continue_btn.grab_focus()
        }
    }

    impl NavigationPageImpl for LoginInBrowserPage {
        fn shown(&self) {
            self.grab_focus();
        }
    }

    #[gtk::template_callbacks]
    impl LoginInBrowserPage {
        /// Open the URL of the SSO login page.
        #[template_callback]
        async fn login_with_sso(&self) {
            let Some(login) = self.login.upgrade() else {
                return;
            };

            let client = login.client().await.expect("client was constructed");
            let oidc_compatibility = self.oidc_compatibility.get();
            let sso_idp_id = self.sso_idp_id.borrow().clone();

            let handle = spawn_tokio!(async move {
                let mut sso_login = client
                    .matrix_auth()
                    .login_sso(|sso_url| async move {
                        let ctx = glib::MainContext::default();
                        ctx.spawn(async move {
                            spawn!(async move {
                                let mut sso_url = sso_url;

                                if oidc_compatibility {
                                    if let Ok(mut parsed_url) = Url::parse(&sso_url) {
                                        // Add an action query parameter manually.
                                        parsed_url.query_pairs_mut().append_pair(
                                            "action",
                                            SsoRedirectOidcAction::Login.as_str(),
                                        );
                                        sso_url = parsed_url.into();
                                    } else {
                                        // If parsing fails, just use the provided URL.
                                        error!("Failed to parse SSO URL: {sso_url}");
                                    }
                                }

                                if let Err(error) = gtk::UriLauncher::new(&sso_url)
                                    .launch_future(gtk::Window::NONE)
                                    .await
                                {
                                    // FIXME: We should forward the error.
                                    error!("Could not launch URI: {error}");
                                }
                            });
                        });
                        Ok(())
                    })
                    .initial_device_display_name("Fractal");

                if let Some(sso_idp_id) = &sso_idp_id {
                    sso_login = sso_login.identity_provider_id(sso_idp_id);
                }

                sso_login.send().await
            });

            match handle.await.expect("task was not aborted") {
                Ok(response) => {
                    login.handle_login_response(response).await;
                }
                Err(error) => {
                    warn!("Could not log in via SSO: {error}");
                    let obj = self.obj();
                    toast!(obj, error.to_user_facing());
                }
            }
        }
    }
}

glib::wrapper! {
    /// A page shown while the user is logging in via SSO.
    pub struct LoginInBrowserPage(ObjectSubclass<imp::LoginInBrowserPage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

impl LoginInBrowserPage {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gio, glib,
    glib::{clone, closure_local},
    CompositeTemplate,
};
use matrix_sdk::Client;
use ruma::{
    api::client::session::{get_login_types::v3::LoginType, login},
    OwnedServerName,
};
use tracing::warn;
use url::Url;

mod advanced_dialog;
mod greeter;
mod homeserver_page;
mod in_browser_page;
mod method_page;
mod session_setup_view;
mod sso_idp_button;

use self::{
    advanced_dialog::LoginAdvancedDialog, greeter::Greeter, homeserver_page::LoginHomeserverPage,
    in_browser_page::LoginInBrowserPage, method_page::LoginMethodPage,
    session_setup_view::SessionSetupView,
};
use crate::{
    components::OfflineBanner, prelude::*, secret::store_session, session::model::Session, spawn,
    spawn_tokio, toast, Application, Window, RUNTIME, SETTINGS_KEY_CURRENT_SESSION,
};

/// A page of the login stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::AsRefStr)]
#[strum(serialize_all = "kebab-case")]
enum LoginPage {
    /// The greeter page.
    Greeter,
    /// The homeserver page.
    Homeserver,
    /// The page to select a login method.
    Method,
    /// The page to log in with the browser.
    InBrowser,
    /// The loading page.
    Loading,
    /// The session setup stack.
    SessionSetup,
    /// The login is completed.
    Completed,
}

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
        sync::LazyLock,
    };

    use glib::subclass::{InitializingObject, Signal};

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/mod.ui")]
    #[properties(wrapper_type = super::Login)]
    pub struct Login {
        #[template_child]
        navigation: TemplateChild<adw::NavigationView>,
        #[template_child]
        greeter: TemplateChild<Greeter>,
        #[template_child]
        homeserver_page: TemplateChild<LoginHomeserverPage>,
        #[template_child]
        method_page: TemplateChild<LoginMethodPage>,
        #[template_child]
        in_browser_page: TemplateChild<LoginInBrowserPage>,
        #[template_child]
        done_button: TemplateChild<gtk::Button>,
        /// Whether auto-discovery is enabled.
        #[property(get, set = Self::set_autodiscovery, construct, explicit_notify, default = true)]
        autodiscovery: Cell<bool>,
        /// The login types supported by the homeserver.
        login_types: RefCell<Vec<LoginType>>,
        /// The domain of the homeserver to log into.
        domain: RefCell<Option<OwnedServerName>>,
        /// The domain of the homeserver to log into, as a string.
        #[property(get = Self::domain_string)]
        domain_string: PhantomData<Option<String>>,
        /// The URL of the homeserver to log into.
        homeserver_url: RefCell<Option<Url>>,
        /// The URL of the homeserver to log into, as a string.
        #[property(get = Self::homeserver_url_string)]
        homeserver_url_string: PhantomData<Option<String>>,
        /// The Matrix client used to log in.
        client: RefCell<Option<Client>>,
        /// The session that was just logged in.
        session: RefCell<Option<Session>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Login {
        const NAME: &'static str = "Login";
        type Type = super::Login;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            OfflineBanner::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_css_name("login");
            klass.set_accessible_role(gtk::AccessibleRole::Group);

            klass.install_action_async(
                "login.sso",
                Some(&Option::<String>::static_variant_type()),
                |obj, _, variant| async move {
                    let sso_idp_id = variant.and_then(|v| v.get::<Option<String>>()).flatten();
                    obj.imp().show_in_browser_page(sso_idp_id, false);
                },
            );

            klass.install_action_async("login.open-advanced", None, |obj, _, _| async move {
                obj.imp().open_advanced_dialog().await;
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Login {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    // The login types changed.
                    Signal::builder("login-types-changed").build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            let obj = self.obj();
            obj.action_set_enabled("login.next", false);

            self.parent_constructed();

            let monitor = gio::NetworkMonitor::default();
            monitor.connect_network_changed(clone!(
                #[weak]
                obj,
                move |_, available| {
                    obj.action_set_enabled("login.sso", available);
                }
            ));
            obj.action_set_enabled("login.sso", monitor.is_network_available());

            self.navigation.connect_visible_page_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.visible_page_changed();
                }
            ));
        }

        fn dispose(&self) {
            self.drop_client();
            self.drop_session();
        }
    }

    impl WidgetImpl for Login {
        fn grab_focus(&self) -> bool {
            match self.visible_page() {
                LoginPage::Greeter => self.greeter.grab_focus(),
                LoginPage::Homeserver => self.homeserver_page.grab_focus(),
                LoginPage::Method => self.method_page.grab_focus(),
                LoginPage::InBrowser => self.in_browser_page.grab_focus(),
                LoginPage::Loading => false,
                LoginPage::SessionSetup => {
                    if let Some(session_setup) = self.session_setup() {
                        session_setup.grab_focus()
                    } else {
                        false
                    }
                }
                LoginPage::Completed => self.done_button.grab_focus(),
            }
        }
    }

    impl BinImpl for Login {}
    impl AccessibleImpl for Login {}

    #[gtk::template_callbacks]
    impl Login {
        /// The visible page of the view.
        pub(super) fn visible_page(&self) -> LoginPage {
            self.navigation
                .visible_page()
                .and_then(|p| p.tag())
                .and_then(|s| s.as_str().try_into().ok())
                .unwrap()
        }

        /// Set whether auto-discovery is enabled.
        pub fn set_autodiscovery(&self, autodiscovery: bool) {
            if self.autodiscovery.get() == autodiscovery {
                return;
            }

            self.autodiscovery.set(autodiscovery);
            self.obj().notify_autodiscovery();
        }

        /// Get the session setup view, if any.
        pub(super) fn session_setup(&self) -> Option<SessionSetupView> {
            self.navigation
                .find_page(LoginPage::SessionSetup.as_ref())
                .and_downcast()
        }

        /// The visible page changed.
        fn visible_page_changed(&self) {
            match self.visible_page() {
                LoginPage::Greeter => {
                    self.clean();
                }
                LoginPage::Homeserver => {
                    // Drop the client because it is bound to the homeserver.
                    self.drop_client();
                    // Drop the session because it is bound to the homeserver and account.
                    self.drop_session();
                    self.method_page.clean();
                }
                LoginPage::Method => {
                    // Drop the session because it is bound to the account.
                    self.drop_session();
                }
                _ => {}
            }
        }

        /// The Matrix client.
        pub(super) async fn client(&self) -> Option<Client> {
            if let Some(client) = self.client.borrow().clone() {
                return Some(client);
            }

            // If the client was dropped, try to recreate it.
            self.homeserver_page.check_homeserver().await;
            if let Some(client) = self.client.borrow().clone() {
                return Some(client);
            }

            None
        }

        /// Set the Matrix client.
        pub(super) fn set_client(&self, client: Option<Client>) {
            let homeserver = client.as_ref().map(Client::homeserver);

            self.set_homeserver_url(homeserver);
            self.client.replace(client);
        }

        /// Drop the Matrix client.
        pub(super) fn drop_client(&self) {
            if let Some(client) = self.client.take() {
                // The `Client` needs to access a tokio runtime when it is dropped.
                let _guard = RUNTIME.enter();
                drop(client);
            }
        }

        /// Drop the session and clean up its data from the system.
        fn drop_session(&self) {
            if let Some(session) = self.session.take() {
                spawn!(async move {
                    let _ = session.log_out().await;
                });
            }
        }

        /// Set the domain of the homeserver to log into.
        pub(super) fn set_domain(&self, domain: Option<OwnedServerName>) {
            if *self.domain.borrow() == domain {
                return;
            }

            self.domain.replace(domain);
            self.obj().notify_domain_string();
        }

        /// The domain of the homeserver to log into.
        ///
        /// If autodiscovery is enabled, this is the server name, otherwise,
        /// this is the prettified homeserver URL.
        fn domain_string(&self) -> Option<String> {
            if self.autodiscovery.get() {
                self.domain.borrow().clone().map(Into::into)
            } else {
                self.homeserver_url_string()
            }
        }

        /// The pretty-formatted URL of the homeserver to log into.
        fn homeserver_url_string(&self) -> Option<String> {
            self.homeserver_url
                .borrow()
                .as_ref()
                .map(|url| url.as_ref().trim_end_matches('/').to_owned())
        }

        /// Set the URL of the homeserver to log into.
        fn set_homeserver_url(&self, homeserver: Option<Url>) {
            if *self.homeserver_url.borrow() == homeserver {
                return;
            }

            self.homeserver_url.replace(homeserver);

            let obj = self.obj();
            obj.notify_homeserver_url_string();

            if !self.autodiscovery.get() {
                obj.notify_domain_string();
            }
        }

        /// Set the login types supported by the homeserver.
        pub(super) fn set_login_types(&self, types: Vec<LoginType>) {
            self.login_types.replace(types);
            self.obj().emit_by_name::<()>("login-types-changed", &[]);
        }

        /// The login types supported by the homeserver.
        pub(super) fn login_types(&self) -> Vec<LoginType> {
            self.login_types.borrow().clone()
        }

        /// Open the login advanced dialog.
        async fn open_advanced_dialog(&self) {
            let obj = self.obj();
            let dialog = LoginAdvancedDialog::new();
            obj.bind_property("autodiscovery", &dialog, "autodiscovery")
                .sync_create()
                .bidirectional()
                .build();
            dialog.run_future(&*obj).await;
        }

        /// Show the appropriate login page given the current login types.
        pub(super) fn show_login_page(&self) {
            let mut oidc_compatibility = false;
            let mut supports_password = false;

            for login_type in self.login_types.borrow().iter() {
                match login_type {
                    LoginType::Sso(sso) if sso.delegated_oidc_compatibility => {
                        oidc_compatibility = true;
                        // We do not care about password support at this point.
                        break;
                    }
                    LoginType::Password(_) => {
                        supports_password = true;
                    }
                    _ => {}
                }
            }

            if oidc_compatibility || !supports_password {
                self.show_in_browser_page(None, oidc_compatibility);
            } else {
                self.navigation.push_by_tag(LoginPage::Method.as_ref());
            }
        }

        /// Show the page to log in with the browser with the given parameters.
        fn show_in_browser_page(&self, sso_idp_id: Option<String>, oidc_compatibility: bool) {
            self.in_browser_page.set_sso_idp_id(sso_idp_id);
            self.in_browser_page
                .set_oidc_compatibility(oidc_compatibility);

            self.navigation.push_by_tag(LoginPage::InBrowser.as_ref());
        }

        /// Handle the given response after successfully logging in.
        pub(super) async fn handle_login_response(&self, response: login::v3::Response) {
            let client = self.client().await.expect("client was constructed");
            // The homeserver could have changed with the login response so get it from the
            // Client.
            let homeserver = client.homeserver();

            match Session::new(homeserver, (&response).into()).await {
                Ok(session) => {
                    self.init_session(session).await;
                }
                Err(error) => {
                    warn!("Could not create session: {error}");
                    let obj = self.obj();
                    toast!(obj, error.to_user_facing());

                    self.navigation.pop();
                }
            }
        }

        /// Initialize the given session.
        async fn init_session(&self, session: Session) {
            let setup_view = SessionSetupView::new(&session);
            setup_view.connect_completed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.navigation.push_by_tag(LoginPage::Completed.as_ref());
                }
            ));
            self.navigation.push(&setup_view);

            self.drop_client();
            self.session.replace(Some(session.clone()));

            // Save ID of logging in session to GSettings
            let settings = Application::default().settings();
            if let Err(err) =
                settings.set_string(SETTINGS_KEY_CURRENT_SESSION, session.session_id())
            {
                warn!("Could not save current session: {err}");
            }

            let session_info = session.info().clone();
            let handle = spawn_tokio!(async move { store_session(session_info).await });

            if handle.await.expect("task was not aborted").is_err() {
                let obj = self.obj();
                toast!(obj, gettext("Could not store session"));
            }

            session.prepare().await;
        }

        /// Finish the login process and show the session.
        #[template_callback]
        fn finish_login(&self) {
            let Some(window) = self.obj().root().and_downcast::<Window>() else {
                return;
            };

            if let Some(session) = self.session.take() {
                window.add_session(session);
            }

            self.clean();
        }

        /// Reset the login stack.
        pub(super) fn clean(&self) {
            // Clean pages.
            self.homeserver_page.clean();
            self.method_page.clean();

            // Clean data.
            self.set_autodiscovery(true);
            self.set_login_types(vec![]);
            self.set_domain(None);
            self.set_homeserver_url(None);
            self.drop_client();
            self.drop_session();

            // Reinitialize UI.
            self.navigation.pop_to_tag(LoginPage::Greeter.as_ref());
            self.unfreeze();
        }

        /// Freeze the login screen.
        pub(super) fn freeze(&self) {
            self.navigation.set_sensitive(false);
        }

        /// Unfreeze the login screen.
        pub(super) fn unfreeze(&self) {
            self.navigation.set_sensitive(true);
        }
    }
}

glib::wrapper! {
    /// A widget managing the login flows.
    pub struct Login(ObjectSubclass<imp::Login>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Login {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the Matrix client.
    fn set_client(&self, client: Option<Client>) {
        self.imp().set_client(client);
    }

    /// The Matrix client.
    async fn client(&self) -> Option<Client> {
        self.imp().client().await
    }

    /// Drop the Matrix client.
    fn drop_client(&self) {
        self.imp().drop_client();
    }

    /// Set the domain of the homeserver to log into.
    fn set_domain(&self, domain: Option<OwnedServerName>) {
        self.imp().set_domain(domain);
    }

    /// Set the login types supported by the homeserver.
    fn set_login_types(&self, types: Vec<LoginType>) {
        self.imp().set_login_types(types);
    }

    /// The login types supported by the homeserver.
    fn login_types(&self) -> Vec<LoginType> {
        self.imp().login_types()
    }

    /// Handle the given response after successfully logging in.
    async fn handle_login_response(&self, response: login::v3::Response) {
        self.imp().handle_login_response(response).await;
    }

    /// Show the appropriate login screen given the current login types.
    fn show_login_page(&self) {
        self.imp().show_login_page();
    }

    /// Freeze the login screen.
    fn freeze(&self) {
        self.imp().freeze();
    }

    /// Unfreeze the login screen.
    fn unfreeze(&self) {
        self.imp().unfreeze();
    }

    /// Connect to the signal emitted when the login types changed.
    pub fn connect_login_types_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "login-types-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

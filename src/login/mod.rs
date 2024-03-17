use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{self, gio, glib, glib::clone, CompositeTemplate};
use matrix_sdk::Client;
use ruma::{
    api::client::session::{get_login_types::v3::LoginType, login},
    OwnedServerName,
};
use tracing::{error, warn};
use url::Url;

mod advanced_dialog;
mod greeter;
mod homeserver_page;
mod idp_button;
mod method_page;
mod session_setup_view;
mod sso_page;

use self::{
    advanced_dialog::LoginAdvancedDialog, greeter::Greeter, homeserver_page::LoginHomeserverPage,
    method_page::LoginMethodPage, session_setup_view::SessionSetupView, sso_page::LoginSsoPage,
};
use crate::{
    components::OfflineBanner, prelude::*, secret::store_session, session::model::Session, spawn,
    spawn_tokio, toast, Application, Window, RUNTIME,
};

#[derive(Clone, Debug, Default, glib::Boxed)]
#[boxed_type(name = "BoxedLoginTypes")]
pub struct BoxedLoginTypes(Vec<LoginType>);

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
    /// The page to wait for SSO to be finished.
    Sso,
    /// The loading page.
    Loading,
    /// The session setup stack.
    SessionSetup,
    /// The login is completed.
    Completed,
}

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::{subclass::InitializingObject, SignalHandlerId};

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/login/mod.ui")]
    #[properties(wrapper_type = super::Login)]
    pub struct Login {
        #[template_child]
        pub navigation: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub greeter: TemplateChild<Greeter>,
        #[template_child]
        pub homeserver_page: TemplateChild<LoginHomeserverPage>,
        #[template_child]
        pub method_page: TemplateChild<LoginMethodPage>,
        #[template_child]
        pub sso_page: TemplateChild<LoginSsoPage>,
        #[template_child]
        pub done_button: TemplateChild<gtk::Button>,
        pub prepared_source_id: RefCell<Option<SignalHandlerId>>,
        pub logged_out_source_id: RefCell<Option<SignalHandlerId>>,
        pub ready_source_id: RefCell<Option<SignalHandlerId>>,
        /// Whether auto-discovery is enabled.
        #[property(get, set = Self::set_autodiscovery, construct, explicit_notify, default = true)]
        pub autodiscovery: Cell<bool>,
        /// The login types supported by the homeserver.
        #[property(get)]
        pub login_types: RefCell<BoxedLoginTypes>,
        /// The domain of the homeserver to log into.
        #[property(get = Self::domain, type = Option<String>)]
        pub domain: RefCell<Option<OwnedServerName>>,
        /// The URL of the homeserver to log into.
        #[property(get = Self::homeserver, type = Option<String>)]
        pub homeserver: RefCell<Option<Url>>,
        /// The Matrix client used to log in.
        pub client: RefCell<Option<Client>>,
        /// The session that was just logged in.
        pub session: RefCell<Option<Session>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Login {
        const NAME: &'static str = "Login";
        type Type = super::Login;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            OfflineBanner::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("login");
            klass.set_accessible_role(gtk::AccessibleRole::Group);

            klass.install_action(
                "login.sso",
                Some(&Option::<String>::static_variant_type()),
                move |widget, _, variant| {
                    let idp_id = variant.and_then(|v| v.get::<Option<String>>()).flatten();
                    spawn!(clone!(@weak widget => async move {
                        widget.login_with_sso(idp_id).await;
                    }));
                },
            );

            klass.install_action("login.open-advanced", None, move |widget, _, _| {
                spawn!(clone!(@weak widget => async move {
                    widget.open_advanced_dialog().await;
                }));
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Login {
        fn constructed(&self) {
            let obj = self.obj();
            obj.action_set_enabled("login.next", false);

            self.parent_constructed();

            let monitor = gio::NetworkMonitor::default();
            monitor.connect_network_changed(clone!(@weak obj => move |_, available| {
                obj.action_set_enabled("login.sso", available);
            }));
            obj.action_set_enabled("login.sso", monitor.is_network_available());

            self.navigation
                .connect_visible_page_notify(clone!(@weak obj => move |_| {
                    obj.visible_page_changed();
                }));
        }

        fn dispose(&self) {
            let obj = self.obj();

            obj.drop_client();
            obj.drop_session();
        }
    }

    impl WidgetImpl for Login {
        fn grab_focus(&self) -> bool {
            match self.visible_page() {
                LoginPage::Greeter => self.greeter.grab_focus(),
                LoginPage::Homeserver => self.homeserver_page.grab_focus(),
                LoginPage::Method => self.method_page.grab_focus(),
                LoginPage::Sso | LoginPage::Loading => false,
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

        /// The domain of the homeserver to log into.
        ///
        /// If autodiscovery is enabled, this is the server name, otherwise,
        /// this is the prettified homeserver URL.
        fn domain(&self) -> Option<String> {
            if self.autodiscovery.get() {
                self.domain.borrow().clone().map(Into::into)
            } else {
                self.homeserver()
            }
        }

        /// The pretty-formatted URL of the homeserver to log into.
        fn homeserver(&self) -> Option<String> {
            self.homeserver
                .borrow()
                .as_ref()
                .map(|url| url.as_ref().trim_end_matches('/').to_owned())
        }

        /// Get the session setup view, if any.
        pub(super) fn session_setup(&self) -> Option<SessionSetupView> {
            self.navigation
                .find_page(LoginPage::SessionSetup.as_ref())
                .and_downcast()
        }
    }
}

glib::wrapper! {
    /// A widget managing the login flows.
    pub struct Login(ObjectSubclass<imp::Login>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl Login {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the visible page.
    fn set_visible_page(&self, page: LoginPage) {
        let navigation = &self.imp().navigation;

        if page == LoginPage::Greeter {
            navigation.pop_to_tag(page.as_ref());
        } else {
            navigation.push_by_tag(page.as_ref());
        }
    }

    /// The visible page changed.
    fn visible_page_changed(&self) {
        let imp = self.imp();

        match imp.visible_page() {
            LoginPage::Greeter => {
                self.clean();
            }
            LoginPage::Homeserver => {
                // Drop the client because it is bound to the homeserver.
                self.drop_client();
                // Drop the session because it is bound to the homeserver and account.
                self.drop_session();
                self.imp().method_page.clean();
            }
            LoginPage::Method => {
                // Drop the session because it is bound to the account.
                self.drop_session();
            }
            _ => {}
        }
    }

    fn parent_window(&self) -> Window {
        self.root()
            .and_downcast()
            .expect("Login needs to have a parent window")
    }

    /// The Matrix client.
    pub async fn client(&self) -> Option<Client> {
        if let Some(client) = self.imp().client.borrow().clone() {
            return Some(client);
        }

        // If the client was dropped, try to recreate it.
        self.imp().homeserver_page.check_homeserver().await;
        if let Some(client) = self.imp().client.borrow().clone() {
            return Some(client);
        }

        None
    }

    /// Set the Matrix client.
    fn set_client(&self, client: Option<Client>) {
        let homeserver = client.as_ref().map(|client| client.homeserver());

        self.set_homeserver_url(homeserver);
        self.imp().client.replace(client);
    }

    /// Drop the Matrix client.
    pub fn drop_client(&self) {
        if let Some(client) = self.imp().client.take() {
            // The `Client` needs to access a tokio runtime when it is dropped.
            let _guard = RUNTIME.enter();
            drop(client);
        }
    }

    /// Drop the session and clean up its data from the system.
    fn drop_session(&self) {
        if let Some(session) = self.imp().session.take() {
            glib::MainContext::default().block_on(async move {
                let _ = session.logout().await;
            });
        }
    }

    fn set_domain(&self, domain: Option<OwnedServerName>) {
        let imp = self.imp();

        if *imp.domain.borrow() == domain {
            return;
        }

        imp.domain.replace(domain);
        self.notify_domain();
    }

    /// The URL of the homeserver to log into.
    pub fn homeserver_url(&self) -> Option<Url> {
        self.imp().homeserver.borrow().clone()
    }

    /// Set the homeserver to log into.
    pub fn set_homeserver_url(&self, homeserver: Option<Url>) {
        let imp = self.imp();

        if self.homeserver_url() == homeserver {
            return;
        }

        imp.homeserver.replace(homeserver);

        self.notify_homeserver();

        if !self.autodiscovery() {
            self.notify_domain();
        }
    }

    /// Set the login types supported by the homeserver.
    fn set_login_types(&self, types: Vec<LoginType>) {
        self.imp().login_types.replace(BoxedLoginTypes(types));
        self.notify_login_types();
    }

    /// Whether the password login type is supported.
    pub fn supports_password(&self) -> bool {
        self.imp()
            .login_types
            .borrow()
            .0
            .iter()
            .any(|t| matches!(t, LoginType::Password(_)))
    }

    async fn open_advanced_dialog(&self) {
        let dialog = LoginAdvancedDialog::new();
        self.bind_property("autodiscovery", &dialog, "autodiscovery")
            .sync_create()
            .bidirectional()
            .build();
        dialog.run_future(self).await;
    }

    /// Show the appropriate login screen given the current login types.
    fn show_login_screen(&self) {
        if self.supports_password() {
            self.set_visible_page(LoginPage::Method);
        } else {
            spawn!(clone!(@weak self as obj => async move {
                obj.login_with_sso(None).await;
            }));
        }
    }

    /// Log in with the SSO login type.
    async fn login_with_sso(&self, idp_id: Option<String>) {
        self.set_visible_page(LoginPage::Sso);
        let client = self.client().await.unwrap();

        let handle = spawn_tokio!(async move {
            let mut login = client
                .matrix_auth()
                .login_sso(|sso_url| async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
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

            if let Some(idp_id) = idp_id.as_deref() {
                login = login.identity_provider_id(idp_id);
            }

            login.send().await
        });

        match handle.await.unwrap() {
            Ok(response) => {
                self.handle_login_response(response).await;
            }
            Err(error) => {
                warn!("Could not log in: {error}");
                toast!(self, error.to_user_facing());
                self.imp().navigation.pop();
            }
        }
    }

    /// Handle the given response after successfully logging in.
    async fn handle_login_response(&self, response: login::v3::Response) {
        let client = self.client().await.unwrap();
        // The homeserver could have changed with the login response so get it from the
        // Client.
        let homeserver = client.homeserver();

        match Session::new(homeserver, (&response).into()).await {
            Ok(session) => {
                self.init_session(session).await;
            }
            Err(error) => {
                warn!("Could not create session: {error}");
                toast!(self, error.to_user_facing());

                self.imp().navigation.pop();
            }
        }
    }

    pub async fn init_session(&self, session: Session) {
        let imp = self.imp();

        let setup_view = SessionSetupView::new(&session);
        setup_view.connect_completed(clone!(@weak self as obj => move |_| {
            obj.show_completed();
        }));
        imp.navigation.push(&setup_view);

        self.drop_client();
        imp.session.replace(Some(session.clone()));

        // Save ID of logging in session to GSettings
        let settings = Application::default().settings();
        if let Err(err) = settings.set_string("current-session", session.session_id()) {
            warn!("Could not save current session: {err}");
        }

        let session_info = session.info().clone();
        let handle = spawn_tokio!(async move { store_session(session_info).await });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not store session: {error}");
            toast!(self, gettext("Could not store session"));
        }

        session.prepare().await;
    }

    /// Show the completed page.
    #[template_callback]
    pub fn show_completed(&self) {
        let imp = self.imp();

        self.set_visible_page(LoginPage::Completed);
        imp.done_button.grab_focus();
    }

    /// Finish the login process and show the session.
    #[template_callback]
    fn finish_login(&self) {
        let session = self.imp().session.take().unwrap();
        self.parent_window().add_session(session);

        self.clean();
    }

    /// Reset the login stack.
    pub fn clean(&self) {
        let imp = self.imp();

        // Clean pages.
        imp.homeserver_page.clean();
        imp.method_page.clean();

        // Clean data.
        self.set_autodiscovery(true);
        self.set_login_types(vec![]);
        self.set_domain(None);
        self.set_homeserver_url(None);
        self.drop_client();
        self.drop_session();

        // Reinitialize UI.
        self.set_visible_page(LoginPage::Greeter);
        self.unfreeze();
    }

    /// Freeze the login screen.
    fn freeze(&self) {
        self.imp().navigation.set_sensitive(false);
    }

    /// Unfreeze the login screen.
    fn unfreeze(&self) {
        self.imp().navigation.set_sensitive(true);
    }
}

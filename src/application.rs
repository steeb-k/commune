use std::{borrow::Cow, cell::RefCell, fmt, rc::Rc};

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone};
use tracing::{debug, error, info, warn};

use crate::{
    config, intent,
    session::model::{Session, SessionState},
    session_list::{FailedSession, SessionInfo, SessionList},
    spawn,
    system_settings::SystemSettings,
    toast,
    utils::{matrix::MatrixIdUri, BoundObjectWeakRef, LoadingState},
    Window, GETTEXT_PACKAGE,
};

/// The key for the current session setting.
pub const SETTINGS_KEY_CURRENT_SESSION: &str = "current-session";

/// The state of the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkState {
    /// The network is available.
    Unavailable,
    /// The network is available with the given connectivity.
    Available(gio::NetworkConnectivity),
}

impl NetworkState {
    /// Construct the network state with the given network monitor.
    fn with_monitor(monitor: &gio::NetworkMonitor) -> Self {
        if monitor.is_network_available() {
            Self::Available(monitor.connectivity())
        } else {
            Self::Unavailable
        }
    }

    /// Log this network state.
    fn log(self) {
        match self {
            Self::Unavailable => {
                info!("Network is unavailable");
            }
            Self::Available(connectivity) => {
                info!("Network connectivity is {connectivity:?}");
            }
        }
    }
}

impl Default for NetworkState {
    fn default() -> Self {
        Self::Available(gio::NetworkConnectivity::Full)
    }
}

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Debug)]
    pub struct Application {
        /// The application settings.
        pub settings: gio::Settings,
        /// The system settings.
        pub system_settings: SystemSettings,
        /// The list of logged-in sessions.
        pub session_list: SessionList,
        pub intent_handler: BoundObjectWeakRef<glib::Object>,
        last_network_state: Cell<NetworkState>,
    }

    impl Default for Application {
        fn default() -> Self {
            Self {
                settings: gio::Settings::new(config::APP_ID),
                system_settings: Default::default(),
                session_list: Default::default(),
                intent_handler: Default::default(),
                last_network_state: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "Application";
        type Type = super::Application;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for Application {
        fn constructed(&self) {
            self.parent_constructed();
            let app = self.obj();

            // Initialize actions and accelerators.
            app.set_up_gactions();
            app.set_up_accels();

            // Listen to errors in the session list.
            self.session_list.connect_error_notify(clone!(
                #[weak]
                app,
                move |session_list| {
                    if let Some(message) = session_list.error() {
                        let window = app.present_main_window();
                        window.show_secret_error(&message);
                    }
                }
            ));

            // Restore the sessions.
            spawn!(clone!(
                #[weak(rename_to = session_list)]
                self.session_list,
                async move {
                    session_list.restore_sessions().await;
                }
            ));

            // Watch the network to log its state.
            let network_monitor = gio::NetworkMonitor::default();
            network_monitor.connect_network_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |network_monitor, _| {
                    let network_state = NetworkState::with_monitor(network_monitor);

                    if imp.last_network_state.get() == network_state {
                        return;
                    }

                    network_state.log();
                    imp.last_network_state.set(network_state);
                }
            ));
        }
    }

    impl ApplicationImpl for Application {
        fn activate(&self) {
            debug!("Application::activate");

            self.obj().present_main_window();
        }

        fn startup(&self) {
            self.parent_startup();

            // Set icons for shell
            gtk::Window::set_default_icon_name(crate::APP_ID);
        }

        fn open(&self, files: &[gio::File], _hint: &str) {
            debug!("Application::open");

            self.obj().present_main_window();

            if files.len() > 1 {
                warn!("Trying to open several URIs, only the first one will be processed");
            }

            if let Some(uri) = files.first().map(FileExt::uri) {
                self.obj().process_uri(&uri);
            } else {
                debug!("No URI to open");
            }
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}
}

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionMap, gio::ActionGroup;
}

impl Application {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", Some(config::APP_ID))
            .property("flags", gio::ApplicationFlags::HANDLES_OPEN)
            .property("resource-base-path", Some("/org/gnome/Fractal/"))
            .build()
    }

    /// Get or create the main window and make sure it is visible.
    ///
    /// Returns the main window.
    fn present_main_window(&self) -> Window {
        let window = if let Some(window) = self.active_window().and_downcast() {
            window
        } else {
            Window::new(self)
        };

        window.present();
        window
    }

    /// The application settings.
    pub fn settings(&self) -> gio::Settings {
        self.imp().settings.clone()
    }

    /// The system settings.
    pub fn system_settings(&self) -> SystemSettings {
        self.imp().system_settings.clone()
    }

    /// The list of logged-in sessions.
    pub fn session_list(&self) -> &SessionList {
        &self.imp().session_list
    }

    /// Set up the application actions.
    fn set_up_gactions(&self) {
        self.add_action_entries([
            // Quit
            gio::ActionEntry::builder("quit")
                .activate(|app: &Application, _, _| {
                    if let Some(window) = app.active_window() {
                        // This is needed to trigger the delete event
                        // and saving the window state
                        window.close();
                    }

                    app.quit();
                })
                .build(),
            // About
            gio::ActionEntry::builder("about")
                .activate(|app: &Application, _, _| {
                    app.show_about_dialog();
                })
                .build(),
            // Show a room. This is the action triggered when clicking a notification about a message.
            gio::ActionEntry::builder("show-room")
                .parameter_type(Some(&intent::ShowRoomPayload::static_variant_type()))
                .activate(|app: &Application, _, v| {
                    let Some(payload) = v.and_then(glib::Variant::get::<intent::ShowRoomPayload>) else {
                        error!("Triggered `show-room` action without the proper payload");
                        return;
                    };

                    app.process_intent(intent::SessionIntent::ShowRoom(payload));
                })
                .build(),
            // Show an identity verification. This is the action triggered when clicking a notification about a new verification.
            gio::ActionEntry::builder("show-identity-verification")
                .parameter_type(Some(&intent::ShowIdentityVerificationPayload::static_variant_type()))
                .activate(|app: &Application, _, v| {
                    let Some(payload) = v.and_then(glib::Variant::get::<intent::ShowIdentityVerificationPayload>) else {
                        error!("Triggered `show-identity-verification` action without the proper payload");
                        return;
                    };

                    app.process_intent(intent::SessionIntent::ShowIdentityVerification(payload));
                })
                .build(),
        ]);
    }

    /// Sets up keyboard shortcuts for application and window actions.
    fn set_up_accels(&self) {
        self.set_accels_for_action("app.quit", &["<Control>q"]);
        self.set_accels_for_action("win.show-help-overlay", &["<Control>question"]);
    }

    fn show_about_dialog(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name("Fractal")
            .application_icon(config::APP_ID)
            .developer_name(gettext("The Fractal Team"))
            .license_type(gtk::License::Gpl30)
            .website("https://gitlab.gnome.org/World/fractal/")
            .issue_url("https://gitlab.gnome.org/World/fractal/-/issues")
            .support_url("https://matrix.to/#/#fractal:gnome.org")
            .version(config::VERSION)
            .copyright(gettext("© 2017-2024 The Fractal Team"))
            .developers(vec![
                "Alejandro Domínguez".to_string(),
                "Alexandre Franke".to_string(),
                "Bilal Elmoussaoui".to_string(),
                "Christopher Davis".to_string(),
                "Daniel García Moreno".to_string(),
                "Eisha Chen-yen-su".to_string(),
                "Jordan Petridis".to_string(),
                "Julian Sparber".to_string(),
                "Kévin Commaille".to_string(),
                "Saurav Sachidanand".to_string(),
            ])
            .designers(vec!["Tobias Bernard".to_string()])
            .translator_credits(gettext("translator-credits"))
            .build();

        // This can't be added via the builder
        dialog.add_credit_section(Some(&gettext("Name by")), &["Regina Bíró"]);

        // If the user wants our support room, try to open it ourselves.
        dialog.connect_activate_link(clone!(
            #[weak(rename_to = obj)]
            self,
            #[weak]
            dialog,
            #[upgrade_or]
            false,
            move |_, uri| {
                if uri == "https://matrix.to/#/#fractal:gnome.org"
                    && obj.session_list().has_session_ready()
                {
                    obj.process_uri(uri);
                    dialog.close();
                    return true;
                }

                false
            }
        ));

        dialog.present(Some(&self.present_main_window()));
    }

    /// Process the given URI.
    fn process_uri(&self, uri: &str) {
        match MatrixIdUri::parse(uri) {
            Ok(matrix_id) => self.process_intent(matrix_id),
            Err(error) => warn!("Invalid Matrix URI: {error}"),
        }
    }

    /// Process the given intent, as soon as possible.
    fn process_intent(&self, intent: impl Into<intent::AppIntent>) {
        let intent = intent.into();
        debug!("Processing intent {intent:?}");

        // We only handle a single intent at time, the latest one.
        self.imp().intent_handler.disconnect_signals();

        let session_list = self.session_list();

        if session_list.state() == LoadingState::Ready {
            match intent {
                intent::AppIntent::WithSession(session_intent) => {
                    self.process_session_intent(session_intent);
                }
                intent::AppIntent::ShowMatrixId(matrix_uri) => match session_list.n_items() {
                    0 => {
                        warn!("Cannot open URI with no logged in session");
                    }
                    1 => {
                        let session = session_list.first().expect("There should be one session");
                        let session_intent = intent::SessionIntent::with_matrix_uri(
                            session.session_id(),
                            matrix_uri,
                        );
                        self.process_session_intent(session_intent);
                    }
                    _ => {
                        spawn!(clone!(
                            #[weak(rename_to = obj)]
                            self,
                            async move {
                                obj.choose_session_for_uri(matrix_uri).await;
                            }
                        ));
                    }
                },
            }
        } else {
            // Wait for the list to be ready.
            let cell = Rc::new(RefCell::new(Some(intent)));
            let handler = session_list.connect_state_notify(clone!(
                #[weak(rename_to = obj)]
                self,
                #[strong]
                cell,
                move |session_list| {
                    if session_list.state() == LoadingState::Ready {
                        obj.imp().intent_handler.disconnect_signals();

                        if let Some(intent) = cell.take() {
                            obj.process_intent(intent);
                        }
                    }
                }
            ));
            self.imp()
                .intent_handler
                .set(session_list.upcast_ref(), vec![handler]);
        }
    }

    /// Ask the user to choose a session to process the given Matrix ID URI.
    ///
    /// The session list needs to be ready.
    async fn choose_session_for_uri(&self, matrix_uri: MatrixIdUri) {
        let main_window = self.present_main_window();

        let Some(session_id) = main_window.choose_session_for_uri().await else {
            warn!("No session selected to show URI");
            return;
        };

        let session_intent = intent::SessionIntent::with_matrix_uri(session_id, matrix_uri);
        self.process_session_intent(session_intent);
    }

    /// Process the given for a session, as soon as the session is ready.
    fn process_session_intent(&self, intent: intent::SessionIntent) {
        let Some(session_info) = self.session_list().get(intent.session_id()) else {
            warn!("Could not find session to process intent {intent:?}");
            toast!(self.present_main_window(), gettext("Session not found"));
            return;
        };
        if session_info.is::<FailedSession>() {
            // We can't do anything, it should show an error screen.
            warn!("Could not process intent {intent:?} for failed session");
        } else if let Some(session) = session_info.downcast_ref::<Session>() {
            if session.state() == SessionState::Ready {
                self.present_main_window()
                    .process_session_intent_ready(intent);
            } else {
                // Wait for the session to be ready.
                let cell = Rc::new(RefCell::new(Some(intent)));
                let handler = session.connect_ready(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    #[strong]
                    cell,
                    move |_| {
                        obj.imp().intent_handler.disconnect_signals();

                        if let Some(intent) = cell.take() {
                            obj.present_main_window()
                                .process_session_intent_ready(intent);
                        }
                    }
                ));
                self.imp()
                    .intent_handler
                    .set(session.upcast_ref(), vec![handler]);
            }
        } else {
            // Wait for the session to be a `Session`.
            let session_list = self.session_list();
            let cell = Rc::new(RefCell::new(Some(intent)));
            let handler = session_list.connect_items_changed(clone!(
                #[weak(rename_to = obj)]
                self,
                #[strong]
                cell,
                move |session_list, pos, _, added| {
                    if added == 0 {
                        return;
                    }
                    let Some(session_id) =
                        cell.borrow().as_ref().map(|i| i.session_id().to_owned())
                    else {
                        return;
                    };

                    for i in pos..pos + added {
                        let Some(session_info) = session_list.item(i).and_downcast::<SessionInfo>()
                        else {
                            break;
                        };

                        if session_info.session_id() == session_id {
                            obj.imp().intent_handler.disconnect_signals();

                            if let Some(intent) = cell.take() {
                                obj.process_session_intent(intent);
                            }
                            break;
                        }
                    }
                }
            ));
            self.imp()
                .intent_handler
                .set(session_list.upcast_ref(), vec![handler]);
        }
    }

    pub fn run(&self) {
        info!("Fractal ({})", config::APP_ID);
        info!("Version: {} ({})", config::VERSION, config::PROFILE);
        info!("Datadir: {}", config::PKGDATADIR);

        ApplicationExtManual::run(self);
    }
}

impl Default for Application {
    fn default() -> Self {
        gio::Application::default()
            .and_downcast::<Application>()
            .unwrap()
    }
}

/// The profile that was built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AppProfile {
    /// A stable release.
    Stable,
    /// A beta release.
    Beta,
    /// A development release.
    Devel,
}

impl AppProfile {
    /// The string representation of this `AppProfile`.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Devel => "devel",
        }
    }

    /// Whether this `AppProfile` should use the `.devel` CSS class on windows.
    pub fn should_use_devel_class(self) -> bool {
        matches!(self, Self::Devel)
    }

    /// The name of the directory where to put data for this profile.
    pub fn dir_name(self) -> Cow<'static, str> {
        match self {
            AppProfile::Stable => Cow::Borrowed(GETTEXT_PACKAGE),
            _ => Cow::Owned(format!("{GETTEXT_PACKAGE}-{self}")),
        }
    }
}

impl fmt::Display for AppProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

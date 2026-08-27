use std::{borrow::Cow, cell::RefCell, fmt, rc::Rc};

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone};
use tracing::{debug, error, info, warn};

use crate::{
    GETTEXT_PACKAGE, Window, config,
    intent::SessionIntent,
    prelude::*,
    session::{Session, SessionState},
    session_list::{FailedSession, SessionInfo, SessionList},
    spawn,
    system_settings::SystemSettings,
    toast,
    utils::{BoundObjectWeakRef, LoadingState, app_bundle::RuntimePaths, matrix::MatrixIdUri},
};

/// The key for the current session setting.
pub(crate) const SETTINGS_KEY_CURRENT_SESSION: &str = "current-session";
/// The name of the application.
pub(crate) const APP_NAME: &str = "Commune";
/// The URL of the homepage of the application.
pub(crate) const APP_HOMEPAGE_URL: &str = "https://github.com/steeb-k/commune";

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Debug)]
    pub struct Application {
        /// The application settings.
        pub(super) settings: gio::Settings,
        /// The system settings.
        pub(super) system_settings: SystemSettings,
        /// The list of logged-in sessions.
        pub(super) session_list: SessionList,
        intent_handler: BoundObjectWeakRef<glib::Object>,
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

            // Initialize actions and accelerators.
            self.set_up_gactions();
            self.set_up_accels();

            self.update_preferences_action();
            self.session_list.connect_items_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_preferences_action();
                    #[cfg(target_os = "android")]
                    imp.update_sync_service();
                }
            ));

            // Listen to errors in the session list.
            self.session_list.connect_error_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |session_list| {
                    if let Some(message) = session_list.error() {
                        let window = imp.present_main_window();
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

            self.set_up_color_scheme();

            #[cfg(debug_assertions)]
            self.set_up_test_notification();

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
            self.parent_activate();

            debug!("Application::activate");

            self.present_main_window();
        }

        fn startup(&self) {
            self.parent_startup();

            // Set icons for shell
            gtk::Window::set_default_icon_name(crate::APP_ID);

            // Both of these have to come after the parent: it is where GTK's
            // quartz backend builds the menu we replace part of, and where it
            // sends `-finishLaunching`, which installs the Apple Event handlers
            // that ours has to replace in turn.
            #[cfg(target_os = "macos")]
            {
                self.set_up_menu_bar();
                crate::utils::macos_url_events::init();
            }
        }

        fn open(&self, files: &[gio::File], _hint: &str) {
            debug!("Application::open");

            self.present_main_window();

            if files.len() > 1 {
                warn!("Trying to open several URIs, only the first one will be processed");
            }

            if let Some(uri) = files.first().map(FileExt::uri) {
                self.process_uri(&uri);
            } else {
                debug!("No URI to open");
            }
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}

    impl Application {
        /// Get or create the main window and make sure it is visible.
        ///
        /// Returns the main window.
        fn present_main_window(&self) -> Window {
            let window = if let Some(window) = self.obj().active_window().and_downcast() {
                window
            } else {
                Window::new(&self.obj())
            };

            window.present();

            // The first thing on Android that can reach the Java side: until
            // there is a window there is no `Activity`, and until there is an
            // `Activity` there is no `Context` to create the notification
            // channel against or to ask for permission through. Doing it on
            // every present rather than only the first is deliberate — both
            // halves are idempotent, and this way a window recreated after the
            // process was killed sets them up again.
            #[cfg(target_os = "android")]
            {
                crate::utils::android_notifications::init(window.upcast_ref::<gtk::Window>());

                // Sessions restored before the window existed had nowhere to
                // start the service from, since it needs the `Activity` the
                // window carries. This is the first moment there is one.
                self.update_sync_service();

                // Register with a UnifiedPush distributor, if one is
                // installed. Step 1 scaffolding — see
                // `utils::android_push::init()` for what is deliberately not
                // decided here yet.
                crate::utils::android_push::init();
            }

            window
        }

        /// Set up the application actions.
        fn set_up_gactions(&self) {
            self.obj().add_action_entries([
                // Quit
                gio::ActionEntry::builder("quit")
                    .activate(|obj: &super::Application, _, _| {
                        if let Some(window) = obj.active_window() {
                            // This is needed to trigger the close request and save the window
                            // state.
                            window.close();
                        }

                        obj.quit();
                    })
                    .build(),
                // About
                gio::ActionEntry::builder("about")
                    .activate(|obj: &super::Application, _, _| {
                        obj.imp().show_about_dialog();
                    })
                    .build(),
                // Preferences. Only macOS asks for this one: GTK's application
                // menu names it unconditionally and binds Command-comma to it,
                // so without it the item is there and dead.
                gio::ActionEntry::builder("preferences")
                    .activate(|obj: &super::Application, _, _| {
                        obj.imp().show_account_settings();
                    })
                    .build(),
                // Show a room. This is the action triggered when clicking a notification about a
                // message.
                gio::ActionEntry::builder(SessionIntent::SHOW_MATRIX_ID_ACTION_NAME)
                    .parameter_type(Some(&SessionIntent::static_variant_type()))
                    .activate(|obj: &super::Application, _, variant| {
                        debug!(
                            "`{}` action activated",
                            SessionIntent::SHOW_MATRIX_ID_APP_ACTION_NAME
                        );

                        let Some((session_id, intent)) =
                            variant.and_then(SessionIntent::show_matrix_id_from_variant)
                        else {
                            error!(
                                "Activated `{}` action without the proper payload",
                                SessionIntent::SHOW_MATRIX_ID_APP_ACTION_NAME
                            );
                            return;
                        };

                        obj.imp().process_session_intent(session_id, intent);
                    })
                    .build(),
                // Answer, decline or show the call that is ringing. This is the action
                // triggered by the buttons on the notification about it.
                gio::ActionEntry::builder(SessionIntent::CALL_ACTION_ACTION_NAME)
                    .parameter_type(Some(&SessionIntent::static_variant_type()))
                    .activate(|obj: &super::Application, _, variant| {
                        debug!(
                            "`{}` action activated",
                            SessionIntent::CALL_ACTION_APP_ACTION_NAME
                        );

                        let Some((session_id, intent)) =
                            variant.and_then(SessionIntent::call_action_from_variant)
                        else {
                            error!(
                                "Activated `{}` action without the proper payload",
                                SessionIntent::CALL_ACTION_APP_ACTION_NAME
                            );
                            return;
                        };

                        obj.imp().process_session_intent(session_id, intent);
                    })
                    .build(),
                // Show an identity verification. This is the action triggered when clicking a
                // notification about a new verification.
                gio::ActionEntry::builder(SessionIntent::SHOW_IDENTITY_VERIFICATION_ACTION_NAME)
                    .parameter_type(Some(&SessionIntent::static_variant_type()))
                    .activate(|obj: &super::Application, _, variant| {
                        debug!(
                            "`{}` action activated",
                            SessionIntent::SHOW_IDENTITY_VERIFICATION_APP_ACTION_NAME
                        );

                        let Some((session_id, intent)) = variant
                            .and_then(SessionIntent::show_identity_verification_from_variant)
                        else {
                            error!(
                                "Activated `{}` action without the proper payload",
                                SessionIntent::SHOW_IDENTITY_VERIFICATION_APP_ACTION_NAME
                            );
                            return;
                        };

                        obj.imp().process_session_intent(session_id, intent);
                    })
                    .build(),
            ]);

            // Send a notification for the room on screen, so that the platform
            // path can be exercised without waiting for somebody to send a
            // message. Development builds only.
            #[cfg(debug_assertions)]
            self.obj()
                .add_action_entries([gio::ActionEntry::builder("test-notification")
                    .activate(|obj: &super::Application, _, _| {
                        obj.imp().send_test_notification();
                    })
                    .build()]);
        }

        /// Send a notification for the room on screen, if there is a session to
        /// send it for.
        ///
        /// Returns whether one was sent.
        #[cfg(debug_assertions)]
        fn send_test_notification(&self) -> bool {
            let obj = self.obj();
            let session = obj
                .active_window()
                .and_downcast::<Window>()
                .and_then(|window| window.current_session_id())
                .and_then(|session_id| self.session_list.get(&session_id))
                .and_downcast::<Session>();

            let Some(session) = session.filter(|session| session.state() == SessionState::Ready)
            else {
                return false;
            };

            spawn!(async move {
                session.notifications().show_test().await;
            });

            true
        }

        /// Send a test notification a moment after startup, when
        /// `COMMUNE_TEST_NOTIFICATION` is set.
        ///
        /// A key binding would be the obvious way in, but on macOS `AppKit`
        /// takes every Command combination that is not in the menu bar before
        /// GTK ever sees it, so the shortcut silently does nothing. Waiting for
        /// the session instead needs no keyboard and no menu.
        #[cfg(debug_assertions)]
        fn set_up_test_notification(&self) {
            if std::env::var_os("COMMUNE_TEST_NOTIFICATION").is_none() {
                return;
            }

            info!("Will send a test notification once a session is ready");
            glib::timeout_add_seconds_local(
                2,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        if imp.send_test_notification() {
                            glib::ControlFlow::Break
                        } else {
                            glib::ControlFlow::Continue
                        }
                    }
                ),
            );
        }

        /// Sets up keyboard shortcuts for application and window actions.
        fn set_up_accels(&self) {
            let obj = self.obj();
            obj.set_accels_for_action("app.quit", &["<Primary>q"]);
            obj.set_accels_for_action("window.close", &["<Primary>w"]);
        }

        /// Follow the `force-dark-mode` setting.
        ///
        /// Off is [`ColorScheme::Default`] rather than "force light", so the
        /// system keeps deciding — which is what it did before this setting
        /// existed, and what most of the desktop expects.
        fn set_up_color_scheme(&self) {
            let apply = |force_dark: bool| {
                adw::StyleManager::default().set_color_scheme(if force_dark {
                    adw::ColorScheme::ForceDark
                } else {
                    adw::ColorScheme::Default
                });
            };

            apply(self.settings.boolean("force-dark-mode"));

            self.settings
                .connect_changed(Some("force-dark-mode"), move |settings, key| {
                    apply(settings.boolean(key));
                });
        }

        /// Open the account settings of the session that is on screen.
        fn show_account_settings(&self) {
            let window = self.present_main_window();

            let Some(session_id) = window.current_session_id() else {
                warn!("Cannot open the account settings with no session");
                return;
            };

            window.open_account_settings(&session_id);
        }

        /// Enable `app.preferences` only while there is a session to configure.
        fn update_preferences_action(&self) {
            let enabled = self.session_list.n_items() > 0;

            if let Some(action) = self
                .obj()
                .lookup_action("preferences")
                .and_downcast::<gio::SimpleAction>()
            {
                action.set_enabled(enabled);
            }
        }

        /// Sync in the background only while something needs it.
        ///
        /// Two questions decide. With no session there is nothing to keep the
        /// process alive for, and the ongoing notification a foreground
        /// service must show would be claiming work that is not happening. And
        /// with push delivering -- an endpoint whose pusher registration
        /// succeeded, unless the mode setting overrides -- the service running
        /// too would spend all day syncing for messages a push would have
        /// announced anyway; one delivery mode at a time is step 4 of
        /// `doc/android-push-plan.md`.
        ///
        /// This is called from where it is because a foreground service may
        /// only be started while the application is on screen, and the callers
        /// -- presenting the window, the session list changing, and a delivery
        /// change arriving through `update_background_delivery()` while the
        /// app happens to be visible -- are moments when it is, or when
        /// stopping (which is always allowed) is the likely direction.
        #[cfg(target_os = "android")]
        pub(super) fn update_sync_service(&self) {
            let needed = self.session_list.n_items() > 0
                && crate::utils::android_push::service_needed(&self.settings);
            crate::utils::android_sync_service::update(needed);
        }

        /// Fill the macOS menu bar.
        ///
        /// GTK keeps its own application menu -- About, Preferences, Services,
        /// Hide, Quit -- ahead of whatever this sets, and replaces the rest,
        /// which is the fallback menu that Edit and Window come from. Ours
        /// carries both.
        #[cfg(target_os = "macos")]
        fn set_up_menu_bar(&self) {
            let builder = gtk::Builder::from_resource("/org/gnome/Fractal/ui/macos_menu_bar.ui");

            let Some(menu_bar) = builder.object::<gio::MenuModel>("macos_menu_bar") else {
                error!("Could not load the macOS menu bar");
                return;
            };

            self.obj().set_menubar(Some(&menu_bar));
        }

        /// Show the dialog with information about the application.
        fn show_about_dialog(&self) {
            let dialog = adw::AboutDialog::builder()
                .application_name(APP_NAME)
                .application_icon(config::APP_ID)
                .developer_name("steeb-k")
                .license_type(gtk::License::Gpl30)
                .website(APP_HOMEPAGE_URL)
                .issue_url("https://github.com/steeb-k/commune/issues")
                .version(config::VERSION)
                .copyright(gettext("© The Fractal Team and the Commune contributors"))
                .developers(["steeb-k"])
                .translator_credits(gettext("translator-credits"))
                .build();

            // These can't be added via the builder.
            // Commune is a fork of Fractal, and almost all of the code it runs was
            // written by that project. Credit it explicitly.
            dialog.add_credit_section(
                Some(&gettext("Based on Fractal by")),
                &[
                    "Alejandro Domínguez",
                    "Alexandre Franke",
                    "Bilal Elmoussaoui",
                    "Christopher Davis",
                    "Daniel García Moreno",
                    "Eisha Chen-yen-su",
                    "Jordan Petridis",
                    "Julian Sparber",
                    "Kévin Commaille",
                    "Saurav Sachidanand",
                ],
            );
            dialog.add_credit_section(Some(&gettext("Fractal design by")), &["Tobias Bernard"]);
            dialog.add_credit_section(Some(&gettext("Fractal name by")), &["Regina Bíró"]);

            dialog.present(Some(&self.present_main_window()));
        }

        /// Process the given URI.
        fn process_uri(&self, uri: &str) {
            debug!(uri, "Processing URI…");

            // The OAuth 2.0 / Matrix SSO redirect on Android: no browser there
            // will follow a redirect back to another app's loopback listener,
            // so login uses this custom scheme instead, and the `Intent` for it
            // arrives through the same path as a `matrix:` link. It is not one,
            // so it is intercepted here rather than handed to `MatrixIdUri`.
            #[cfg(target_os = "android")]
            if uri.starts_with(crate::login::ANDROID_REDIRECT_URI) {
                if !crate::utils::android::deliver_oauth_redirect(uri.to_owned()) {
                    warn!("Received an OAuth 2.0 / SSO redirect with no login flow waiting");
                }
                return;
            }

            // A tapped notification, which arrives the same way for the same
            // reason: `NotificationManager` can only carry an `Intent`, so the
            // action and target a `GNotification` would have held in memory are
            // written into a URI instead. See `utils::android_notifications`.
            #[cfg(target_os = "android")]
            if uri.starts_with(crate::utils::android_notifications::NOTIFICATION_URI) {
                self.open_notification(uri);
                return;
            }

            match MatrixIdUri::parse(uri) {
                Ok(matrix_id) => {
                    self.select_session_for_intent(SessionIntent::ShowMatrixId(matrix_id));
                }
                Err(error) => warn!("Invalid Matrix URI: {error}"),
            }
        }

        /// Activate the application action a tapped notification names.
        ///
        /// The action is addressed by the name it was posted under and its
        /// target is read back against the type that action declares, rather
        /// than against whatever the text happens to parse as — which is what
        /// keeps `SessionIntent` the only description of the payload. The same
        /// reasoning as `utils::macos_notifications`, which does this for the
        /// same reason on the other platform without `GNotification`.
        #[cfg(target_os = "android")]
        fn open_notification(&self, uri: &str) {
            let Some((action, target)) = crate::utils::android_notifications::tapped_action(uri)
            else {
                warn!("Ignoring a tapped notification that carries no action");
                return;
            };

            // The action was stored with the `app.` prefix it is addressed by,
            // but activating it on the application itself wants the bare name.
            let name = action.strip_prefix("app.").unwrap_or(&action);

            let Some(parameter_type) = self.obj().action_parameter_type(name) else {
                error!("Could not open a notification: no `{name}` action takes a target");
                return;
            };

            let variant = match glib::Variant::parse(Some(&parameter_type), &target) {
                Ok(variant) => variant,
                Err(error) => {
                    error!("Could not read the target of a tapped notification: {error}");
                    return;
                }
            };

            debug!(action = name, "Opening a tapped notification");
            self.obj().activate_action(name, Some(&variant));
        }

        /// Select a session to handle the given intent as soon as possible.
        fn select_session_for_intent(&self, intent: SessionIntent) {
            debug!(?intent, "Selecting session for intent…");

            // We only handle a single intent at time, the latest one.
            self.intent_handler.disconnect_signals();

            if self.session_list.state() == LoadingState::Ready {
                match self.session_list.n_items() {
                    0 => {
                        warn!("Cannot process intent with no logged in session");
                    }
                    1 => {
                        let session = self
                            .session_list
                            .first()
                            .expect("there should be one session");
                        self.process_session_intent(session.session_id(), intent);
                    }
                    _ => {
                        spawn!(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            async move {
                                imp.ask_session_for_intent(intent).await;
                            }
                        ));
                    }
                }
            } else {
                debug!(?intent, "Session list is not ready, queuing intent…");
                // Wait for the list to be ready.
                let cell = Rc::new(RefCell::new(Some(intent)));
                let handler = self.session_list.connect_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[strong]
                    cell,
                    move |session_list| {
                        if session_list.state() == LoadingState::Ready {
                            imp.intent_handler.disconnect_signals();

                            if let Some(intent) = cell.take() {
                                imp.select_session_for_intent(intent);
                            }
                        }
                    }
                ));
                self.intent_handler
                    .set(self.session_list.upcast_ref(), vec![handler]);
            }
        }

        /// Ask the user to choose a session to process the given Matrix ID URI.
        ///
        /// The session list needs to be ready.
        async fn ask_session_for_intent(&self, intent: SessionIntent) {
            debug!(?intent, "Asking to select a session to process intent…");
            let main_window = self.present_main_window();

            let Some(session_id) = main_window.ask_session().await else {
                warn!("No session selected to show intent");
                return;
            };

            self.process_session_intent(session_id, intent);
        }

        /// Process the given intent for the given session, as soon as the
        /// session is ready.
        fn process_session_intent(&self, session_id: String, intent: SessionIntent) {
            let Some(session_info) = self.session_list.get(&session_id) else {
                warn!(
                    session = session_id,
                    ?intent,
                    "Could not find session to process intent"
                );
                toast!(self.present_main_window(), gettext("Session not found"));
                return;
            };

            debug!(session = session_id, ?intent, "Processing session intent…");

            if session_info.is::<FailedSession>() {
                // We can't do anything, it should show an error screen.
                warn!(
                    session = session_id,
                    ?intent,
                    "Could not process intent for failed session"
                );
            } else if let Some(session) = session_info.downcast_ref::<Session>() {
                if session.state() == SessionState::Ready {
                    self.present_main_window()
                        .process_session_intent(session.session_id(), intent);
                } else {
                    debug!(
                        session = session_id,
                        ?intent,
                        "Session is not ready, queuing intent…"
                    );
                    // Wait for the session to be ready.
                    let cell = Rc::new(RefCell::new(Some((session_id, intent))));
                    let handler = session.connect_ready(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        #[strong]
                        cell,
                        move |_| {
                            imp.intent_handler.disconnect_signals();

                            if let Some((session_id, intent)) = cell.take() {
                                imp.present_main_window()
                                    .process_session_intent(&session_id, intent);
                            }
                        }
                    ));
                    self.intent_handler.set(session.upcast_ref(), vec![handler]);
                }
            } else {
                debug!(
                    session = session_id,
                    ?intent,
                    "Session is still loading, queuing intent…"
                );
                // Wait for the session to be a `Session`.
                let cell = Rc::new(RefCell::new(Some((session_id, intent))));
                let handler = self.session_list.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[strong]
                    cell,
                    move |session_list, pos, _, added| {
                        if added == 0 {
                            return;
                        }
                        let Some(session_id) = cell
                            .borrow()
                            .as_ref()
                            .map(|(session_id, _)| session_id.clone())
                        else {
                            return;
                        };

                        for i in pos..pos + added {
                            let Some(session_info) =
                                session_list.item(i).and_downcast::<SessionInfo>()
                            else {
                                break;
                            };

                            if session_info.session_id() == session_id {
                                imp.intent_handler.disconnect_signals();

                                if let Some((session_id, intent)) = cell.take() {
                                    imp.process_session_intent(session_id, intent);
                                }
                                break;
                            }
                        }
                    }
                ));
                self.intent_handler
                    .set(self.session_list.upcast_ref(), vec![handler]);
            }
        }
    }
}

glib::wrapper! {
    /// The Commune application.
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

    /// The application settings.
    pub(crate) fn settings(&self) -> gio::Settings {
        self.imp().settings.clone()
    }

    /// Re-evaluate how messages are delivered while the app is not on screen.
    ///
    /// Called by `utils::android_push` when push delivery becomes available or
    /// stops being. Starting the foreground service only works with the
    /// application on screen; a change that arrives in the background takes
    /// effect at the next present, which calls this too.
    #[cfg(target_os = "android")]
    pub(crate) fn update_background_delivery(&self) {
        self.imp().update_sync_service();
    }

    /// The system settings.
    pub(crate) fn system_settings(&self) -> SystemSettings {
        self.imp().system_settings.clone()
    }

    /// The list of logged-in sessions.
    pub(crate) fn session_list(&self) -> &SessionList {
        &self.imp().session_list
    }

    /// Run Commune.
    ///
    /// The paths are the ones the app actually loaded its resources from, which
    /// inside a macOS bundle are not the ones Meson compiled in.
    pub(crate) fn run(&self, paths: &RuntimePaths) {
        info!("Commune ({})", config::APP_ID);
        info!("Version: {} ({})", config::VERSION, config::PROFILE);
        info!("Datadir: {}", paths.pkgdata_dir().display());

        ApplicationExtManual::run(self);
    }
}

impl Default for Application {
    fn default() -> Self {
        gio::Application::default()
            .and_downcast::<Application>()
            .expect("application should always be available")
    }
}

/// The profile that was built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum AppProfile {
    /// A stable release.
    Stable,
    /// A beta release.
    Beta,
    /// A development release.
    Devel,
}

impl AppProfile {
    /// The string representation of this `AppProfile`.
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Devel => "devel",
        }
    }

    /// Whether this `AppProfile` should use the `.devel` CSS class on windows.
    pub(crate) fn should_use_devel_class(self) -> bool {
        matches!(self, Self::Devel)
    }

    /// The name of the directory where to put data for this profile.
    pub(crate) fn dir_name(self) -> Cow<'static, str> {
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

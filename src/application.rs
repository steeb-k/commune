use std::{borrow::Cow, fmt};

use gettextrs::gettext;
use gio::{ApplicationFlags, Settings};
use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use ruma::{OwnedRoomId, RoomId};
use tracing::{debug, info};

use crate::{config, Window};

mod imp {
    use adw::subclass::prelude::AdwApplicationImpl;

    use super::*;

    #[derive(Debug)]
    pub struct Application {
        pub window: glib::WeakRef<Window>,
        pub settings: Settings,
    }

    impl Default for Application {
        fn default() -> Self {
            Self {
                window: Default::default(),
                settings: Settings::new(config::APP_ID),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "Application";
        type Type = super::Application;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for Application {}

    impl ApplicationImpl for Application {
        fn activate(&self) {
            debug!("GtkApplication<Application>::activate");
            let app = self.obj();

            if let Some(window) = self.window.upgrade() {
                window.present();
                return;
            }

            app.setup_gactions();
            app.setup_accels();

            app.get_or_create_main_window();
        }

        fn startup(&self) {
            debug!("GtkApplication<Application>::startup");
            self.parent_startup();
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}
}

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends gio::Application, gtk::Application, adw::Application, @implements gio::ActionMap, gio::ActionGroup;
}

impl Application {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", Some(config::APP_ID))
            .property("flags", ApplicationFlags::default())
            .property("resource-base-path", Some("/org/gnome/Fractal/"))
            .build()
    }

    /// Get the main window, if any.
    pub fn main_window(&self) -> Option<Window> {
        self.imp().window.upgrade()
    }

    /// Get the main window or create it if it doesn't exist.
    ///
    /// This also ensures that the window is visible by calling
    /// `Window::present()`.
    fn get_or_create_main_window(&self) -> Window {
        if let Some(window) = self.main_window() {
            window.present();
            return window;
        }

        let window = Window::new(self);
        self.imp().window.set(Some(&window));
        window.present();

        window
    }

    pub fn settings(&self) -> Settings {
        self.imp().settings.clone()
    }

    fn setup_gactions(&self) {
        self.add_action_entries([
            // Quit
            gio::ActionEntry::builder("quit")
                .activate(|app: &Application, _, _| {
                    if let Some(window) = app.main_window() {
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
            gio::ActionEntry::builder("show-room")
                .parameter_type(Some(&AppShowRoomPayload::static_variant_type()))
                .activate(|app: &Application, _, v| {
                    if let Some(payload) = v.and_then(|v| v.get::<AppShowRoomPayload>()) {
                        app.get_or_create_main_window()
                            .show_room(&payload.session_id, &payload.room_id);
                    }
                })
                .build(),
        ]);
    }

    /// Sets up keyboard shortcuts for application and window actions.
    fn setup_accels(&self) {
        self.set_accels_for_action("app.quit", &["<Control>q"]);
        self.set_accels_for_action("win.show-help-overlay", &["<Control>question"]);
    }

    fn show_about_dialog(&self) {
        let dialog = adw::AboutWindow::builder()
            .application_name("Fractal")
            .application_icon(config::APP_ID)
            .developer_name(gettext("The Fractal Team"))
            .license_type(gtk::License::Gpl30)
            .website("https://gitlab.gnome.org/GNOME/fractal/")
            .issue_url("https://gitlab.gnome.org/GNOME/fractal/-/issues")
            .support_url("https://matrix.to/#/#fractal:gnome.org")
            .version(config::VERSION)
            .modal(true)
            .copyright(gettext("© 2017-2023 The Fractal Team"))
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

        dialog.set_transient_for(self.main_window().as_ref());

        // This can't be added via the builder
        dialog.add_credit_section(Some(&gettext("Name by")), &["Regina Bíró"]);

        dialog.present();
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
    /// A nightly release.
    Devel,
    /// A development release.
    Hack,
}

impl AppProfile {
    /// The string representation of this `AppProfile`.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Devel => "devel",
            Self::Hack => "hack",
        }
    }

    /// Whether this `AppProfile` should use the `.devel` CSS class on windows.
    pub fn should_use_devel_class(&self) -> bool {
        matches!(self, Self::Devel | Self::Hack)
    }
}

impl fmt::Display for AppProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct AppShowRoomPayload {
    pub session_id: String,
    pub room_id: OwnedRoomId,
}

impl glib::StaticVariantType for AppShowRoomPayload {
    fn static_variant_type() -> Cow<'static, glib::VariantTy> {
        <(String, String)>::static_variant_type()
    }
}

impl glib::ToVariant for AppShowRoomPayload {
    fn to_variant(&self) -> glib::Variant {
        (&self.session_id, self.room_id.as_str()).to_variant()
    }
}

impl glib::FromVariant for AppShowRoomPayload {
    fn from_variant(variant: &glib::Variant) -> Option<Self> {
        let (session_id, room_id) = variant.get::<(String, String)>()?;
        let room_id = RoomId::parse(room_id).ok()?;
        Some(Self {
            session_id,
            room_id,
        })
    }
}

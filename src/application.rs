use crate::config;
use crate::FrctlWindow;
use gettextrs::gettext;
use gio::ApplicationFlags;
use glib::clone;
use glib::WeakRef;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use gtk_macros::action;
use log::{debug, info};
use once_cell::sync::OnceCell;
use std::env;

mod imp {
    use super::*;

    #[derive(Debug)]
    pub struct FrctlApplication {
        pub window: OnceCell<WeakRef<FrctlWindow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FrctlApplication {
        const NAME: &'static str = "FrctlApplication";
        type Type = super::FrctlApplication;
        type ParentType = gtk::Application;

        fn new() -> Self {
            Self {
                window: OnceCell::new(),
            }
        }
    }

    impl ObjectImpl for FrctlApplication {}

    impl ApplicationImpl for FrctlApplication {
        fn activate(&self, app: &Self::Type) {
            debug!("GtkApplication<FrctlApplication>::activate");

            if let Some(window) = self.window.get() {
                let window = window.upgrade().unwrap();
                window.show();
                window.present();
                return;
            }

            app.set_resource_base_path(Some("/org/gnome/FractalNext/"));
            app.setup_css();

            let window = FrctlWindow::new(app);
            self.window
                .set(window.downgrade())
                .expect("Window already set.");

            app.setup_gactions();
            app.setup_accels();

            app.get_main_window().present();
        }

        fn startup(&self, app: &Self::Type) {
            debug!("GtkApplication<FrctlApplication>::startup");
            self.parent_startup(app);
        }
    }

    impl GtkApplicationImpl for FrctlApplication {}
}

glib::wrapper! {
    pub struct FrctlApplication(ObjectSubclass<imp::FrctlApplication>)
        @extends gio::Application, gtk::Application, @implements gio::ActionMap, gio::ActionGroup;
}

impl FrctlApplication {
    pub fn new() -> Self {
        glib::Object::new(&[
            ("application-id", &Some(config::APP_ID)),
            ("flags", &ApplicationFlags::default()),
        ])
        .expect("Application initialization failed...")
    }

    fn get_main_window(&self) -> FrctlWindow {
        imp::FrctlApplication::from_instance(self)
            .window
            .get()
            .unwrap()
            .upgrade()
            .unwrap()
    }

    fn setup_gactions(&self) {
        // Quit
        action!(
            self,
            "quit",
            clone!(@weak self as app => move |_, _| {
                // This is needed to trigger the delete event
                // and saving the window state
                app.get_main_window().close();
                app.quit();
            })
        );

        // About
        action!(
            self,
            "about",
            clone!(@weak self as app => move |_, _| {
                app.show_about_dialog();
            })
        );
    }

    // Sets up keyboard shortcuts
    fn setup_accels(&self) {
        self.set_accels_for_action("app.quit", &["<primary>q"]);
        self.set_accels_for_action("win.show-help-overlay", &["<primary>question"]);
    }

    fn setup_css(&self) {
        let provider = gtk::CssProvider::new();
        provider.load_from_resource("/org/gnome/FractalNext/style.css");
        if let Some(display) = gdk::Display::get_default() {
            gtk::StyleContext::add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    fn show_about_dialog(&self) {
        let dialog = gtk::AboutDialogBuilder::new()
            .program_name("Fractal")
            .logo_icon_name(config::APP_ID)
            .license_type(gtk::License::Gpl30)
            .website("https://gitlab.gnome.org/GNOME/fractal/")
            .website_label(gettext("Learn more about Fractal").as_str())
            .version(config::VERSION)
            .transient_for(&self.get_main_window())
            .modal(true)
            .comments(gettext("A Matrix.org client for GNOME").as_str())
            .copyright(gettext("© 2017-2021 The Fractal Team").as_str())
            .authors(vec![
                "Alejandro Domínguez".to_string(),
                "Alexandre Franke".to_string(),
                "Bilal Elmoussaoui".to_string(),
                "Christopher Davis".to_string(),
                "Daniel García Moreno".to_string(),
                "Eisha Chen-yen-su".to_string(),
                "Jordan Petridis".to_string(),
                "Julian Sparber".to_string(),
                "Saurav Sachidanand".to_string(),
            ])
            .artists(vec!["Tobias Bernard".to_string()])
            .translator_credits(gettext("translator-credits").as_str())
            .build();

        // This can't be added via the builder
        dialog.add_credit_section(gettext("Name by").as_str(), &["Regina Bíró"]);

        dialog.show();
    }

    pub fn run(&self) {
        info!("Fractal ({})", config::APP_ID);
        info!("Version: {} ({})", config::VERSION, config::PROFILE);
        info!("Datadir: {}", config::PKGDATADIR);

        let args: Vec<String> = env::args().collect();
        ApplicationExtManual::run(self, &args);
    }
}

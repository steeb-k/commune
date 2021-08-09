#![doc(
    html_logo_url = "https://gitlab.gnome.org/GNOME/fractal/-/raw/fractal-next/data/icons/org.gnome.FractalNext.svg?inline=false"
)]

mod application;
#[rustfmt::skip]
mod config;

mod components;
mod error;
mod login;
mod secret;
mod session;
mod utils;
mod window;

use self::application::Application;
use self::error::Error;
use self::login::Login;
use self::session::Session;
use self::window::Window;

use config::{GETTEXT_PACKAGE, LOCALEDIR, RESOURCES_FILE};
use gettextrs::*;
use gtk::gdk::Display;
use gtk::gio;
use gtk::IconTheme;
use once_cell::sync::Lazy;

/// The default tokio runtime to be used for async tasks
pub static RUNTIME: Lazy<tokio::runtime::Runtime> =
    Lazy::new(|| tokio::runtime::Runtime::new().unwrap());

fn main() {
    // Initialize logger, debug is carried out via debug!, info!, and warn!.
    tracing_subscriber::fmt::init();

    // Prepare i18n
    setlocale(LocaleCategory::LcAll, "");
    bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR).expect("Invalid argument passed to bindtextdomain");
    textdomain(GETTEXT_PACKAGE).expect("Invalid string passed to textdomain");

    gtk::glib::set_application_name("Fractal");
    gtk::glib::set_prgname(Some("fractal"));

    gtk::init().expect("Unable to start GTK4");
    adw::init();

    let res = gio::Resource::load(RESOURCES_FILE).expect("Could not load gresource file");
    gio::resources_register(&res);

    IconTheme::for_display(&Display::default().unwrap())
        .unwrap()
        .add_resource_path("/org/gnome/FractalNext/icons");

    let app = Application::new();
    app.run();
}

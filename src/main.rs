mod application;
#[rustfmt::skip]
mod config;

mod login;
mod secret;
mod session;
mod window;

use self::application::FrctlApplication;
use self::login::FrctlLogin;
use self::session::FrctlSession;
use self::window::FrctlWindow;

use adw;
use config::{GETTEXT_PACKAGE, LOCALEDIR, RESOURCES_FILE};
use gettextrs::*;
use gtk::gdk::Display;
use gtk::gio;
use gtk::IconTheme;
use once_cell::sync::Lazy;
use tokio;

/// The default tokio runtime to be used for async tasks
pub static RUNTIME: Lazy<tokio::runtime::Runtime> =
    Lazy::new(|| tokio::runtime::Runtime::new().unwrap());

fn main() {
    // Initialize logger, debug is carried out via debug!, info!, and warn!.
    pretty_env_logger::init();

    // Prepare i18n
    setlocale(LocaleCategory::LcAll, "");
    bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR);
    textdomain(GETTEXT_PACKAGE);

    gtk::glib::set_application_name("Fractal");
    gtk::glib::set_prgname(Some("fractal"));

    gtk::init().expect("Unable to start GTK4");
    adw::init();

    let res = gio::Resource::load(RESOURCES_FILE).expect("Could not load gresource file");
    gio::resources_register(&res);

    IconTheme::get_for_display(&Display::get_default().unwrap())
        .unwrap()
        .add_resource_path("/org/gnome/FractalNext/icons");

    let app = FrctlApplication::new();
    app.run();
}

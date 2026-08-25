#![doc(
    html_logo_url = "https://raw.githubusercontent.com/steeb-k/commune/main/data/icons/io.github.steeb_k.Commune.svg",
    html_favicon_url = "https://raw.githubusercontent.com/steeb-k/commune/main/data/icons/io.github.steeb_k.Commune-symbolic.svg"
)]
#![recursion_limit = "256"]
// A Windows GUI application that asks for a console gets one, and it flashes up
// behind the window for as long as the app runs. Development builds keep it,
// because it is where `tracing` writes and where a panic is legible; release
// builds do without.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod account_chooser_dialog;
mod account_switcher;
mod application;
mod components;
#[rustfmt::skip]
mod config;
mod account_settings;
mod contrib;
mod error_page;
mod i18n;
mod identity_verification_view;
mod intent;
mod login;
mod prelude;
mod secret;
mod session;
mod session_list;
mod session_view;
mod system_settings;
mod user_facing_error;
mod utils;
mod window;

use std::sync::LazyLock;

use gettextrs::*;
use gtk::{IconTheme, gdk::Display, gio};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use self::{
    application::*,
    config::*,
    i18n::*,
    utils::{OneshotNotifier, app_bundle},
    window::Window,
};

/// The default tokio runtime to be used for async tasks
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Runtime::new().expect("creating tokio runtime should succeed")
});

/// The notifier to make sure that only one `GtkMediaFile` is played at a single
/// time.
static MEDIA_FILE_NOTIFIER: LazyLock<OneshotNotifier> =
    LazyLock::new(|| OneshotNotifier::new("MEDIA_FILE_NOTIFIER"));

fn main() {
    // Initialize logger, debug is carried out via debug!, info!, warn! and error!.
    // Default to the INFO level for this crate and WARN for everything else.
    // It can be overridden with the RUST_LOG environment variable.
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("commune=info,warn"));

    tracing_subscriber::registry()
        .with(fmt::layer().with_filter(env_filter))
        .init();

    // Find out where our own files are, and tell the libraries we link against
    // where theirs are. This sets environment variables, so it must happen
    // before anything spawns a thread.
    let paths = app_bundle::init();

    // Start listening for notification taps before anything else can finish
    // launching the application, or a tap that launched it is delivered to
    // nobody.
    #[cfg(target_os = "macos")]
    utils::macos_notifications::init();

    // Tell Windows who we are. Nothing to do with notifications works until the
    // process has claimed an application user model ID, and it fails silently
    // rather than complaining, so this happens before anything can try.
    #[cfg(target_os = "windows")]
    {
        utils::windows_app_id::init();
        utils::windows_notifications::init();
        // Before the main loop, so that a Commune started *by* a notification
        // click is already serving the class when Windows calls it.
        utils::windows_toast_activator::init();
    }

    // Prepare i18n
    // Safety: `setlocale` is safe to call because the program is single-threaded.
    unsafe { setlocale(LocaleCategory::LcAll, "") };
    bindtextdomain(GETTEXT_PACKAGE, &paths.localedir)
        .expect("Invalid argument passed to bindtextdomain");
    textdomain(GETTEXT_PACKAGE).expect("Invalid string passed to textdomain");

    gtk::glib::set_application_name("Commune");

    // `Window` is a plain, server-side-decorated `gtk::ApplicationWindow` on
    // Windows, with a native frame subclass standing in for CSD (see
    // `doc/windows.md`). `gtk_window_set_titlebar()` enables CSD
    // unconditionally, so a window with no titlebar still needs telling:
    // left unset, GTK gives an undecorated win32 toplevel its own default
    // `GtkHeaderBar` at realize.
    #[cfg(target_os = "windows")]
    // SAFETY: called before `gtk::init()`, before any other thread exists.
    unsafe {
        std::env::set_var("GTK_CSD", "0");
    }

    // GDK's win32 backend gates DirectComposition-backed rendering (what
    // `GskGLRenderer`/`GskVulkanRenderer` both need there) behind this flag on
    // purpose -- upstream's own comment on `gdk_win32_display_init_dcomp` says
    // it "causes issues with the GL and Vulkan renderers", which is reason
    // enough not to inherit it in a release build sight unseen. Debug builds
    // opt in anyway, so a dependency bump that finally makes this flag mean
    // something gets exercised the moment anyone runs a dev build, rather than
    // silently changing what ships. As of this MSYS2 `gtk4` (4.22.4-1, already
    // MSYS2's newest), the flag is not even recognised -- `GDK_DEBUG=help`
    // does not list it, so this line is inert today; see doc/windows.md's
    // renderer section for the full story. If Commune starts crashing or
    // glitching visually in a *debug* build on Windows with no other
    // explanation after a GTK bump, this is the first thing to suspect. If it
    // instead starts rendering through GL/Vulkan cleanly, that is the signal
    // to test carrying it into release builds too.
    #[cfg(all(target_os = "windows", debug_assertions))]
    // SAFETY: called before `gtk::init()`, before any other thread exists.
    unsafe {
        let value = match std::env::var("GDK_DEBUG") {
            Ok(existing) if !existing.is_empty() => format!("{existing}:dcomp"),
            _ => "dcomp".to_owned(),
        };
        std::env::set_var("GDK_DEBUG", value);
    }

    gtk::init().expect("Could not start GTK4");

    // Now that there are settings to change, make text resolve to the size it
    // is on every other platform.
    #[cfg(target_os = "macos")]
    utils::macos_text_scale::init();

    gst::init().expect("Could not initialize gst");

    #[cfg(target_os = "linux")]
    aperture::init(APP_ID);

    let res = gio::Resource::load(&paths.resources_file).expect("Could not load gresource file");
    gio::resources_register(&res);
    let ui_res =
        gio::Resource::load(&paths.ui_resources_file).expect("Could not load UI gresource file");
    gio::resources_register(&ui_res);

    IconTheme::for_display(&Display::default().unwrap())
        .add_resource_path("/org/gnome/Fractal/icons");

    let app = Application::new();
    app.run(&paths);
}

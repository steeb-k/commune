#![doc(
    html_logo_url = "https://raw.githubusercontent.com/steeb-k/commune/main/data/icons/io.github.steeb_k.Commune.svg",
    html_favicon_url = "https://raw.githubusercontent.com/steeb-k/commune/main/data/icons/io.github.steeb_k.Commune-symbolic.svg"
)]
#![recursion_limit = "256"]
//! Commune, a Matrix client.
//!
//! Everything lives in this library and `src/main.rs` is three lines that call
//! [`run()`], which looks like ceremony on Linux and is the reason the Android
//! port can exist at all.
//!
//! GTK's Android glue does not start a process. It loads a shared object and
//! looks up `main` inside it, so the application has to be something that can
//! be linked into a library rather than something that is already an
//! executable. Cargo will only produce a `staticlib` or a `cdylib` from a
//! **library** target — `crate-type` is not a key a `[[bin]]` has — so a
//! binary-only crate cannot be packaged for Android at all, whatever its
//! symbols look like.
//!
//! Nothing else changes. Every other platform still gets a `commune` binary,
//! built from a `main.rs` that does nothing but call in here.

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
#[cfg(not(target_os = "android"))]
use tracing_subscriber::fmt;
use tracing_subscriber::{EnvFilter, prelude::*};

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

/// The symbol `build-aux/android/stub.c` calls from its `main`.
///
/// GTK's glue looks up `main` in the application's shared object and calls it,
/// and Rust does not export a `main` of its own. Rather than try to make it,
/// the library exports this and a three-line C `main` next to it calls in —
/// which is also how the library gets linked into the shared object in the
/// first place, since Meson performs that link and Cargo only supplies the
/// `staticlib`.
///
/// Returning `0` unconditionally is honest: [`run()`] either panics or comes
/// back after the application has quit normally, so there is no failure left to
/// report by the time this returns.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn commune_main() -> std::ffi::c_int {
    run();
    0
}

/// Set the process up and run the application until it quits.
///
/// This is what used to be `main()`. It must be called on the thread the
/// application will run on, and before anything spawns another one: it sets
/// environment variables and the locale, neither of which is safe to do once a
/// second thread exists.
///
/// # Panics
///
/// If the process cannot become an application at all: GTK failing to
/// initialize, the gresources being absent or unreadable, or gettext rejecting
/// the locale directory. Every one of those means the UI cannot be drawn, so
/// there is nothing to carry on to and failing loudly is the whole of the
/// error handling.
pub fn run() {
    // Initialize logger, debug is carried out via debug!, info!, warn! and error!.
    // Default to the INFO level for this crate and WARN for everything else.
    // It can be overridden with the RUST_LOG environment variable.
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("commune=info,warn"));

    // An Android application has no stdout: anything written there is dropped,
    // so the same subscriber that works everywhere else would log into nothing.
    // `logcat` is the platform's answer, and `adb logcat -s Commune` is how the
    // output is read back.
    #[cfg(target_os = "android")]
    tracing_subscriber::registry()
        .with(paranoid_android::layer("Commune").with_filter(env_filter))
        .init();

    // The same missing stdout swallows panics, and that is much worse than
    // losing log lines. A panic in a tokio worker aborts only that task, so
    // without this the visible symptom is a future that never completes: a
    // spinner that spins for ever, with nothing anywhere to say why. Route
    // panics through `tracing`, which does reach logcat.
    #[cfg(target_os = "android")]
    {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!("PANIC: {info}");
            previous(info);
        }));
    }
    #[cfg(not(target_os = "android"))]
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

    // Prepare i18n
    // Safety: `setlocale` is safe to call because the program is single-threaded.
    unsafe { setlocale(LocaleCategory::LcAll, "") };
    bindtextdomain(GETTEXT_PACKAGE, &paths.localedir)
        .expect("Invalid argument passed to bindtextdomain");
    textdomain(GETTEXT_PACKAGE).expect("Invalid string passed to textdomain");

    gtk::glib::set_application_name("Commune");

    gtk::init().expect("Could not start GTK4");

    // Now that there are settings to change, make text resolve to the size it
    // is on every other platform.
    #[cfg(target_os = "macos")]
    utils::macos_text_scale::init();

    // Capture the Java VM while we are still on the thread GTK gave a `JNIEnv`
    // to. The secret store needs it from a tokio worker, which has no env of
    // its own and cannot borrow this one, so this has to happen here and not
    // where it is used. It needs a display, so it must follow `gtk::init()`.
    #[cfg(target_os = "android")]
    if let Err(error) = utils::android::init() {
        // Not fatal on its own: what fails without it is the secret store, and
        // that reports its own failure with a message about sessions rather
        // than about JNI.
        tracing::error!("Could not reach the Java VM: {error}");
    }

    // GStreamer is not cross-built for Android yet, so there is nothing to
    // initialize there. See `doc/android.md`.
    #[cfg(not(target_os = "android"))]
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

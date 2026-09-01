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
#[cfg(target_os = "android")]
mod android_setup_dialog;
mod application;
mod components;
#[rustfmt::skip]
mod config;
mod account_settings;
mod contrib;
mod core_bridge;
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

/// The default tokio runtime to be used for async tasks.
///
/// The core's, not one of our own. Both crates run Matrix work on tokio, and
/// two runtimes in one process would mean two thread pools and objects
/// dropped under a guard for the runtime they were not made on — which is
/// what `TokioDrop` exists to prevent. There is one runtime; this is it.
pub(crate) use commune_core::RUNTIME;
use gettextrs::*;
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
    //
    // Except on Android, where debug is the default: there is no environment
    // to override with — even the `wrap.<package>` property trick is refused
    // on current emulator images — and the port is still being measured
    // through logcat, which filters by level fine on its own
    // (`adb logcat -s Commune:I`).
    #[cfg(target_os = "android")]
    const DEFAULT_FILTER: &str = "commune=debug,warn";
    #[cfg(not(target_os = "android"))]
    const DEFAULT_FILTER: &str = "commune=info,warn";

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

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
    // SAFETY: called before GTK is started -- which is now `startup()`'s job
    // -- and before any other thread exists.
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
    // SAFETY: called before GTK is started -- which is now `startup()`'s job
    // -- and before any other thread exists.
    unsafe {
        let value = match std::env::var("GDK_DEBUG") {
            Ok(existing) if !existing.is_empty() => format!("{existing}:dcomp"),
            _ => "dcomp".to_owned(),
        };
        std::env::set_var("GDK_DEBUG", value);
    }

    // Nothing above this line takes longer than a few milliseconds, and that is
    // the point. Everything the process actually needs — GTK, GStreamer, both
    // gresources, the icon theme, and on Android the JVM capture and static
    // GStreamer plugin registration — is set up in `ApplicationImpl::startup`,
    // which `GApplication` runs only on the instance that won registration.
    // `gtk::init()` is not called here at all: `GtkApplication` does it in the
    // `startup` our own chains up to, which is the ordinary way to write a GTK
    // application and, at 459ms measured, by far the largest thing a second
    // launch was being charged for. See `doc/startup-registration-race.md`.
    let app = Application::new();
    app.run(&paths);
}

//! Where the app finds its own files at runtime.
//!
//! On Linux the answer is fixed at build time: Meson bakes the install
//! prefix into [`RESOURCES_FILE`], [`UI_RESOURCES_FILE`] and [`LOCALEDIR`],
//! and the app is only ever run from that prefix.
//!
//! Everywhere else that cannot work, for three different reasons: a macOS
//! `.app` has a prefix that cannot be known when it is built, Windows installs
//! per-user rather than to a fixed system prefix, and an Android package has
//! one that is known and wrong. Each arm is below.
//!
//! macOS and Windows are both relocatable — a `.app` gets dragged wherever
//! the user likes, and `bundle.sh`'s folder gets installed per-user — so on
//! both, everything is found relative to the executable instead of trusting
//! what Meson baked in. Windows is the
//! simpler of the two: `bundle.sh` lays out `bin\commune.exe` beside
//! `share\commune\*.gresource` and `share\locale`, which is the same shape
//! `meson install` gives the MSYS2 prefix, so one relative computation
//! serves both a packaged bundle and a plain dev install. `GLib` itself
//! already finds its *own* data — schemas, pixbuf loaders, `GStreamer`
//! plugins, fontconfig — relative to its own DLL automatically on Windows,
//! which is what lets `bundle.sh` call the layout self-relocating; our two
//! gresources are not a `GLib` lookup, so they needed the same treatment
//! explicitly. Without it, the baked-in path only ever resolved on the
//! machine that built the binary — every other machine panicked on start,
//! silently, since a release build has no console for the message to reach.
//!
//! A macOS `.app` is at `Commune.app/Contents/MacOS/commune`, with the
//! payload laid out under `Contents/Resources` as a small Unix prefix:
//!
//! ```text
//! Contents/Resources/share/commune/*.gresource
//! Contents/Resources/share/locale
//! Contents/Resources/share/glib-2.0/schemas/gschemas.compiled
//! Contents/Resources/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache
//! Contents/Resources/lib/gstreamer-1.0
//! Contents/Resources/lib/gio/modules
//! Contents/Resources/libexec/gstreamer-1.0/gst-plugin-scanner
//! Contents/Resources/etc/fonts/fonts.conf
//! ```
//!
//! The libraries themselves live in `Contents/Frameworks` and are found through
//! the install names the bundler rewrites, not through anything here.
//!
//! Our own two gresources and the locale directory are passed to the calls that
//! need them, as [`RuntimePaths`]. The rest belong to libraries we only link
//! against — `GLib`, `GdkPixbuf`, `GStreamer`, fontconfig — which read them
//! from the environment, so [`init()`] sets those variables before any of those
//! libraries are initialized.
//!
//! Doing this in-process rather than from a launcher script is deliberate:
//! `LaunchServices` delivers `matrix:` Apple Events and the Keychain's ACL
//! identity to `CFBundleExecutable`, and a wrapper script would receive both
//! instead of the app.

use std::path::{Path, PathBuf};

use crate::config::{LOCALEDIR, RESOURCES_FILE, UI_RESOURCES_FILE};

/// The paths of the files the app loads at startup.
#[derive(Debug, Clone)]
pub(crate) struct RuntimePaths {
    /// The path of the gresource file with the app's data.
    pub(crate) resources_file: PathBuf,
    /// The path of the gresource file with the app's compiled UI definitions.
    pub(crate) ui_resources_file: PathBuf,
    /// The path of the directory with the app's translations.
    pub(crate) localedir: PathBuf,
}

impl RuntimePaths {
    /// The directory the app's own data was loaded from.
    ///
    /// This is Meson's `PKGDATADIR` outside a bundle, and a directory inside
    /// the bundle within one.
    pub(crate) fn pkgdata_dir(&self) -> &Path {
        self.resources_file.parent().unwrap_or(&self.resources_file)
    }

    /// The paths that Meson baked in at build time.
    fn from_config() -> Self {
        Self {
            resources_file: RESOURCES_FILE.into(),
            ui_resources_file: UI_RESOURCES_FILE.into(),
            localedir: LOCALEDIR.into(),
        }
    }
}

/// Prepare the process for wherever it was launched from, and return the paths
/// it should load its resources from.
///
/// This must be called first thing in `main()`. On macOS it sets environment
/// variables, which is only sound while the process is single-threaded, so
/// nothing that starts a thread — including touching the tokio runtime — can
/// have run yet.
pub(crate) fn init() -> RuntimePaths {
    #[cfg(target_os = "macos")]
    {
        self::macos::init()
    }
    #[cfg(target_os = "android")]
    {
        self::android::init()
    }
    #[cfg(target_os = "windows")]
    {
        self::windows::init()
    }
    #[cfg(not(any(target_os = "macos", target_os = "android", target_os = "windows")))]
    {
        RuntimePaths::from_config()
    }
}

/// Where an Android package keeps the files Meson installed.
///
/// Meson's absolutes are useless here. pixiewood configures the build with
/// `prefix = '/'` and installs into a staging directory, so [`RESOURCES_FILE`]
/// is baked in as `/share/commune/resources.gresource` — which on a device
/// names the root of the filesystem, where nothing of ours has ever been.
///
/// What actually happens is that everything under that staging directory is
/// packed into the APK's `assets/`, and GTK's Java glue extracts it to
/// `Context.getFilesDir()` before calling `main`. The glue then tells `GLib`
/// where that is, so the directory is already known — it only has to be asked
/// for.
///
/// It has to be asked of **`GLib`**, not the environment. The glue calls
/// `g_set_user_dirs()` (`gdk/android/gdkandroidruntime.c:277`), which sets
/// `GLib`'s own idea of the XDG directories and never touches `environ`, so
/// `std::env::var("XDG_DATA_DIRS")` sees nothing at all.
///
/// And it has to be `XDG_DATA_DIRS`, not `XDG_DATA_HOME`. The glue points the
/// two at different places: `XDG_DATA_DIRS` is `getFilesDir()/share`, where the
/// assets were extracted, while `XDG_DATA_HOME` is
/// `getExternalFilesDir(null)/share` — external storage, which never receives
/// them.
///
/// Nothing here sets an environment variable, unlike the macOS arm: `GLib`
/// finds the `GSettings` schemas under `XDG_DATA_DIRS/glib-2.0/schemas` by
/// itself, and that is exactly where pixiewood compiles them to.
#[cfg(target_os = "android")]
mod android {
    use gtk::glib;
    use tracing::{debug, warn};

    use super::RuntimePaths;

    /// The name of the directory holding the application's own data, within a
    /// data directory. This is the last component of Meson's `pkgdatadir`.
    const PKGDATA_NAME: &str = "commune";

    pub(super) fn init() -> RuntimePaths {
        // There is only ever one of these on Android, but taking the first that
        // actually holds our gresource is cheap and self-checking: if the assets
        // were not extracted, the warning below says so, instead of
        // `gio::Resource::load` failing later on a path nobody can account for.
        let data_dirs = glib::system_data_dirs();
        let data_dir = data_dirs
            .iter()
            .find(|dir| dir.join(PKGDATA_NAME).join("resources.gresource").exists());

        let Some(data_dir) = data_dir else {
            warn!(
                "Found no extracted assets in {data_dirs:?}, falling back to the paths Meson \
                 baked in, which do not exist on Android"
            );
            return RuntimePaths::from_config();
        };
        debug!("Running from extracted assets in {}", data_dir.display());

        let pkgdata_dir = data_dir.join(PKGDATA_NAME);
        RuntimePaths {
            resources_file: pkgdata_dir.join("resources.gresource"),
            ui_resources_file: pkgdata_dir.join("ui-resources.gresource"),
            localedir: data_dir.join("locale"),
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        env,
        path::{Path, PathBuf},
    };

    use tracing::debug;

    use super::RuntimePaths;

    /// The directory of a bundle that the current executable is part of, if it
    /// is part of one.
    ///
    /// This is `Commune.app/Contents/Resources`, given an executable at
    /// `Commune.app/Contents/MacOS/commune`.
    fn resources_dir() -> Option<PathBuf> {
        let exe = env::current_exe().ok()?;

        let macos_dir = exe.parent()?;
        if macos_dir.file_name()? != "MacOS" {
            return None;
        }

        let contents_dir = macos_dir.parent()?;
        if contents_dir.file_name()? != "Contents" {
            return None;
        }

        if contents_dir.parent()?.extension()? != "app" {
            return None;
        }

        Some(contents_dir.join("Resources"))
    }

    /// Set the given environment variable to the given path.
    fn set_var(key: &str, path: impl AsRef<Path>) {
        let path = path.as_ref();
        debug!("Setting {key} to {}", path.display());

        // SAFETY: we are called from `init()`, which is documented to be called
        // first thing in `main()`, so the process is still single-threaded and
        // no other thread can be reading the environment.
        unsafe { env::set_var(key, path) };
    }

    /// Point the libraries we link against at the copies of their data inside
    /// the bundle, and return the paths of our own files in it.
    pub(super) fn init() -> RuntimePaths {
        let Some(resources_dir) = resources_dir() else {
            // Not bundled: this is a build installed into a prefix, and Meson
            // already knows where that is.
            return RuntimePaths::from_config();
        };
        debug!("Running from a bundle at {}", resources_dir.display());

        let share_dir = resources_dir.join("share");
        let lib_dir = resources_dir.join("lib");

        // GLib looks up the app's GSettings schemas, which it aborts without.
        set_var(
            "GSETTINGS_SCHEMA_DIR",
            share_dir.join("glib-2.0").join("schemas"),
        );
        // GdkPixbuf loads the image loaders listed in this cache. Without it
        // there is no SVG loader, and every symbolic icon fails to load.
        set_var(
            "GDK_PIXBUF_MODULE_FILE",
            lib_dir
                .join("gdk-pixbuf-2.0")
                .join("2.10.0")
                .join("loaders.cache"),
        );
        // GStreamer needs both its plugins and the helper that introspects
        // them, which it runs as a subprocess.
        set_var("GST_PLUGIN_SYSTEM_PATH_1_0", lib_dir.join("gstreamer-1.0"));
        set_var(
            "GST_PLUGIN_SCANNER_1_0",
            resources_dir
                .join("libexec")
                .join("gstreamer-1.0")
                .join("gst-plugin-scanner"),
        );
        // GIO loads its TLS backend as a module, so HTTPS fails without this.
        set_var("GIO_MODULE_DIR", lib_dir.join("gio").join("modules"));
        // GTK finds the icon themes and its own data through these.
        set_var("GTK_DATA_PREFIX", &resources_dir);
        set_var("GTK_EXE_PREFIX", &resources_dir);

        // Anything that goes looking for XDG data — icon themes above all —
        // should look in the bundle first, but a user with their own icon theme
        // installed should still get it.
        let data_dirs = match env::var_os("XDG_DATA_DIRS") {
            Some(dirs) if !dirs.is_empty() => {
                let mut paths = vec![share_dir.clone()];
                paths.extend(env::split_paths(&dirs));
                env::join_paths(paths).ok()
            }
            _ => None,
        };
        match data_dirs {
            Some(dirs) => set_var("XDG_DATA_DIRS", dirs),
            None => set_var("XDG_DATA_DIRS", &share_dir),
        }

        // fontconfig is only bundled if the system's own configuration turns
        // out not to be enough, so this one is optional.
        let fonts_conf = resources_dir.join("etc").join("fonts").join("fonts.conf");
        if fonts_conf.exists() {
            set_var("FONTCONFIG_FILE", fonts_conf);
        }

        let pkgdata_dir = share_dir.join("commune");
        RuntimePaths {
            resources_file: pkgdata_dir.join("resources.gresource"),
            ui_resources_file: pkgdata_dir.join("ui-resources.gresource"),
            localedir: share_dir.join("locale"),
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::{env, path::PathBuf};

    use super::RuntimePaths;

    /// Find our own two gresources and the locale directory relative to the
    /// running executable, rather than trusting the path Meson baked in at
    /// build time — see the module-level doc comment for why that path only
    /// ever resolves on the machine that built it.
    pub(super) fn init() -> RuntimePaths {
        let Some(root) = install_root() else {
            return RuntimePaths::from_config();
        };

        let share_dir = root.join("share");
        let pkgdata_dir = share_dir.join("commune");
        RuntimePaths {
            resources_file: pkgdata_dir.join("resources.gresource"),
            ui_resources_file: pkgdata_dir.join("ui-resources.gresource"),
            localedir: share_dir.join("locale"),
        }
    }

    /// The directory holding `bin`, `share` and `lib`, given an executable
    /// at `<root>\bin\commune.exe`. The same shape whether that root is
    /// `bundle.sh`'s relocatable folder or a plain `meson install` into an
    /// MSYS2 prefix.
    fn install_root() -> Option<PathBuf> {
        let exe = env::current_exe().ok()?;
        Some(exe.parent()?.parent()?.to_path_buf())
    }
}

//! Where the app finds its own files at runtime.
//!
//! Everywhere except macOS the answer is fixed at build time: Meson bakes the
//! install prefix into [`RESOURCES_FILE`], [`UI_RESOURCES_FILE`] and
//! [`LOCALEDIR`], and the app is only ever run from that prefix.
//!
//! A macOS `.app` is relocatable — the user drags it wherever they like — so
//! nothing about its location can be known when it is built. Everything it
//! needs is therefore found relative to the executable, which is at
//! `Commune.app/Contents/MacOS/commune`, with the payload laid out under
//! `Contents/Resources` as a small Unix prefix:
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
    #[cfg(not(target_os = "macos"))]
    {
        RuntimePaths::from_config()
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

//! Collection of common methods and types.

use std::{
    borrow::Cow,
    cell::{Cell, OnceCell, RefCell},
    fmt, fs,
    io::{self, Write},
    ops::Deref,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    sync::{Arc, LazyLock, Mutex},
};

use adw::prelude::*;
use futures_channel::oneshot;
use futures_util::future::BoxFuture;
use gtk::{gio, glib};
use regex::Regex;
use tempfile::NamedTempFile;
use tracing::error;

#[cfg(target_os = "android")]
pub(crate) mod android;
#[cfg(target_os = "android")]
pub(crate) mod android_notifications;
#[cfg(target_os = "android")]
pub(crate) mod android_push;
#[cfg(target_os = "android")]
pub(crate) mod android_sync_service;
pub(crate) mod app_bundle;
pub(crate) mod expression;
mod expression_list_model;
mod fixed_selection;
mod grouping_list_model;
pub(crate) mod key_bindings;
pub(crate) mod klipy;
mod location;
#[cfg(target_os = "macos")]
pub(crate) mod macos_emoji_spacing;
#[cfg(target_os = "macos")]
pub(crate) mod macos_notifications;
#[cfg(target_os = "macos")]
pub(crate) mod macos_text_scale;
#[cfg(target_os = "macos")]
pub(crate) mod macos_url_events;
mod macros;
pub(crate) mod matrix;
pub(crate) mod media;
pub(crate) mod notifications;
pub(crate) mod password;
mod placeholder_object;
mod single_item_list_model;
pub(crate) mod sourceview;
pub(crate) mod string;
mod template_callbacks;
pub(crate) mod toast;
#[cfg(target_os = "windows")]
pub(crate) mod windows_app_id;
#[cfg(target_os = "windows")]
pub(crate) mod windows_frame;
#[cfg(target_os = "windows")]
pub(crate) mod windows_notifications;
#[cfg(target_os = "windows")]
pub(crate) mod windows_toast_activator;

// Two leaves of Track 3 that moved wholesale, re-exported under the paths
// they already had so that `utils::tls::matrix_client()` and
// `utils::http::fetch()` read as they always did. Neither has a `glib` type
// or a `gettext` call in it, so there was nothing left behind to wrap: the
// application's copies were the core's copies with `pub(crate)` written on
// them. See `doc/track3-convergence.md`.
pub(crate) use commune_core::{http, tls};

pub(crate) use self::{
    expression_list_model::ExpressionListModel,
    fixed_selection::FixedSelection,
    grouping_list_model::*,
    location::{Location, LocationError, LocationExt},
    placeholder_object::PlaceholderObject,
    single_item_list_model::SingleItemListModel,
    template_callbacks::TemplateCallbacks,
};
use crate::{PROFILE, RUNTIME};

/// The type of data.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DataType {
    /// Data that should not be deleted.
    Persistent,
    /// Cache that can be deleted freely.
    Cache,
}

impl DataType {
    /// The path of the directory where data should be stored, depending on this
    /// type.
    #[cfg(not(target_os = "windows"))]
    pub(crate) fn dir_path(self) -> PathBuf {
        let mut path = self.base_dir_path();
        path.push(PROFILE.dir_name().as_ref());

        path
    }

    /// The path of the directory where data should be stored, depending on this
    /// type.
    ///
    /// Windows has one per-user location for both types, `%LOCALAPPDATA%`,
    /// which is where an application keeps state that should not roam to the
    /// user's other machines — and our databases are far too large to roam.
    /// There is no system cache directory to pair it with, so the two types are
    /// told apart by a subdirectory. That puts the profile in the middle of the
    /// path rather than at the end, which is why this does not share the shape
    /// above.
    #[cfg(target_os = "windows")]
    pub(crate) fn dir_path(self) -> PathBuf {
        // `glib::user_data_dir()` is `%LOCALAPPDATA%` on Windows. It is used
        // rather than the variable itself so that a deliberate `XDG_DATA_HOME`
        // still moves the data, which is how the app is told apart from itself
        // in tests.
        let mut path = glib::user_data_dir();
        path.push(PROFILE.dir_name().as_ref());

        match self {
            DataType::Persistent => path.push("data"),
            DataType::Cache => path.push("cache"),
        }

        path
    }

    /// The path of the platform directory that holds data of this type.
    #[cfg(not(any(target_os = "macos", target_os = "android", target_os = "windows")))]
    fn base_dir_path(self) -> PathBuf {
        match self {
            DataType::Persistent => glib::user_data_dir(),
            DataType::Cache => glib::user_cache_dir(),
        }
    }

    /// The path of the platform directory that holds data of this type.
    ///
    /// `GLib`'s XDG answers are wrong here in both directions, and the
    /// persistent one is wrong in a way that matters.
    ///
    /// GTK's Android glue sets `XDG_DATA_HOME` — what `glib::user_data_dir()`
    /// returns — to `Context.getExternalFilesDir(null)/share`. That is
    /// *external* storage: not readable by other ordinary applications under
    /// scoped storage, but exposed over USB/MTP, reachable by anything holding
    /// `MANAGE_EXTERNAL_STORAGE`, and possibly on removable media. The session
    /// databases and the secret store both sit under this directory, so leaving
    /// it there would put the message history and the passphrase that encrypts
    /// it somewhere the application sandbox does not reach.
    ///
    /// The glue sets nothing at all for the cache, so `glib::user_cache_dir()`
    /// falls back to `$HOME/.cache`, and an Android process has no useful
    /// `HOME`.
    ///
    /// What the glue does give us is `XDG_DATA_DIRS`, set to
    /// `Context.getFilesDir()/share` — internal storage, owned by this
    /// application's UID. Everything here is derived from that, because asking
    /// Android directly needs a `Context`, and a `Context` needs a realized
    /// toplevel, which does not exist when the first session is restored.
    ///
    /// **Persistent data may not live in `getFilesDir()` itself**, which is
    /// where it was first put and where it was destroyed by every new build.
    /// That directory is GTK's, not ours: the glue extracts the application's
    /// assets into it, and decides whether to do so by comparing a fingerprint
    /// file against the one in the APK. When they differ — which is to say on
    /// every build — `SystemFilesystem.doWriteResources()` calls
    /// `cleanDirectory(getFilesDir())` first, and that recurses and deletes
    /// everything it finds. Sessions, the secret store, the SDK's databases and
    /// the message history all sat under it, so installing a new build silently
    /// logged the account out and threw away its data.
    ///
    /// So persistent data goes to `<data>/no_backup` — `getNoBackupFilesDir()`,
    /// a sibling of `files` and `cache` that the glue never looks at. It is the
    /// right place on its own merits too: `patch-manifest.sh` already forces
    /// `allowBackup="false"` because the databases are sealed with a Keystore
    /// key that cannot leave the device, and this is the directory Android
    /// provides for exactly that.
    ///
    /// The cache stays at `getCacheDir()`, which the glue does not touch.
    #[cfg(target_os = "android")]
    fn base_dir_path(self) -> PathBuf {
        // `getFilesDir()`, `getCacheDir()` and `getNoBackupFilesDir()` are all
        // siblings on Android: `<data>/files`, `<data>/cache`,
        // `<data>/no_backup`.
        let data_dir = android_files_dir()
            .parent()
            .expect("the Android files directory should have a parent")
            .to_owned();

        match self {
            DataType::Persistent => data_dir.join("no_backup"),
            DataType::Cache => data_dir.join("cache"),
        }
    }

    /// The path of the platform directory that holds data of this type.
    ///
    /// `GLib` follows the XDG base directory specification everywhere, so it
    /// would put our data in `~/.local/share` and `~/.cache` on macOS. Use the
    /// directories macOS actually expects instead, which is also where a user
    /// looking for the app's data would go.
    #[cfg(target_os = "macos")]
    fn base_dir_path(self) -> PathBuf {
        let mut path = glib::home_dir();
        path.push("Library");

        match self {
            DataType::Persistent => path.push("Application Support"),
            DataType::Cache => path.push("Caches"),
        }

        path
    }
}

/// The path of `Context.getFilesDir()`, derived from what GTK's Android glue
/// told `GLib`.
///
/// # Panics
///
/// If `XDG_DATA_DIRS` is not the single `<files>/share` entry the glue sets.
///
/// Carrying on with `glib::user_data_dir()` instead would be possible and is
/// exactly what must not happen: that path is external storage, so a quiet
/// fallback would put the databases and the secret store back where this code
/// exists to move them from. A panic is loud and obvious; the alternative is
/// silent and wrong.
#[cfg(target_os = "android")]
fn android_files_dir() -> PathBuf {
    let data_dirs = glib::system_data_dirs();

    let share_dir = data_dirs
        .first()
        .expect("XDG_DATA_DIRS should be set by GTK's Android glue");

    assert!(
        share_dir.file_name().is_some_and(|name| name == "share"),
        "expected XDG_DATA_DIRS to be `<files>/share`, got {}",
        share_dir.display()
    );

    share_dir
        .parent()
        .expect("`<files>/share` should have a parent")
        .to_owned()
}

/// Repair a file handed over by GTK's broken macOS pasteboard encoding.
///
/// GTK 4.22's macOS backend percent-encodes the whole `file://…` string it
/// builds for a file that is dropped on the window or read from the
/// clipboard, so the scheme's own colon comes out as `%3A` and GIO, unable
/// to parse a scheme, hands us a dummy file with no path. The rest of the
/// string *is* correctly encoded, so putting the colon back yields exactly
/// the URI a fixed GTK produces. Upstream fixed it on `main` in `2a8a2895`;
/// no 4.22 release carries the fix. Once one does, this never triggers: a
/// healthy local file has a path.
#[cfg(target_os = "macos")]
pub(crate) fn repair_pasteboard_file(file: gio::File) -> gio::File {
    match file.uri().strip_prefix("file%3A//") {
        Some(rest) if file.path().is_none() => gio::File::for_uri(&format!("file://{rest}")),
        _ => file,
    }
}

/// Replace variables in the given string with the given dictionary.
///
/// The expected format to replace is `{name}`, where `name` is the first string
/// in the dictionary entry tuple.
pub(crate) fn freplace<'a>(s: &'a str, args: &[(&str, &str)]) -> Cow<'a, str> {
    let mut s = Cow::Borrowed(s);

    for (k, v) in args {
        s = Cow::Owned(s.replace(&format!("{{{k}}}"), v));
    }

    s
}

/// Regex that matches a string that only includes emojis.
pub(crate) static EMOJI_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        ^
        [\p{White_Space}\p{Emoji_Component}]*
        [\p{Emoji}--\p{Decimal_Number}]+
        [\p{White_Space}\p{Emoji}\p{Emoji_Component}--\p{Decimal_Number}]*
        $
        # That string is made of at least one emoji, except digits, possibly more,
        # possibly with modifiers, possibly with spaces, but nothing else
        ",
    )
    .unwrap()
});

/// Inner to manage a bound object.
#[derive(Debug)]
struct BoundObjectInner<T: ObjectType> {
    obj: T,
    signal_handler_ids: Vec<glib::SignalHandlerId>,
}

/// Wrapper to manage a bound object.
///
/// This keeps a strong reference to the object.
#[derive(Debug)]
pub struct BoundObject<T: ObjectType> {
    inner: RefCell<Option<BoundObjectInner<T>>>,
}

impl<T: ObjectType> BoundObject<T> {
    /// Creates a new empty `BoundObject`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the given object and signal handlers IDs.
    ///
    /// Calls `disconnect_signals` first to drop the previous strong reference
    /// and disconnect the previous signal handlers.
    pub(crate) fn set(&self, obj: T, signal_handler_ids: Vec<glib::SignalHandlerId>) {
        self.disconnect_signals();

        let inner = BoundObjectInner {
            obj,
            signal_handler_ids,
        };

        self.inner.replace(Some(inner));
    }

    /// Get the object, if any.
    pub fn obj(&self) -> Option<T> {
        self.inner.borrow().as_ref().map(|inner| inner.obj.clone())
    }

    /// Disconnect the signal handlers and drop the strong reference.
    pub fn disconnect_signals(&self) {
        if let Some(inner) = self.inner.take() {
            for signal_handler_id in inner.signal_handler_ids {
                inner.obj.disconnect(signal_handler_id);
            }
        }
    }
}

impl<T: ObjectType> Default for BoundObject<T> {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl<T: ObjectType> Drop for BoundObject<T> {
    fn drop(&mut self) {
        self.disconnect_signals();
    }
}

impl<T: IsA<glib::Object> + glib::HasParamSpec> glib::property::Property for BoundObject<T> {
    type Value = Option<T>;
}

impl<T: IsA<glib::Object>> glib::property::PropertyGet for BoundObject<T> {
    type Value = Option<T>;

    fn get<R, F: Fn(&Self::Value) -> R>(&self, f: F) -> R {
        f(&self.obj())
    }
}

/// Wrapper to manage a bound object.
///
/// This keeps a weak reference to the object.
#[derive(Debug)]
pub struct BoundObjectWeakRef<T: ObjectType> {
    weak_obj: glib::WeakRef<T>,
    signal_handler_ids: RefCell<Vec<glib::SignalHandlerId>>,
}

impl<T: ObjectType> BoundObjectWeakRef<T> {
    /// Creates a new empty `BoundObjectWeakRef`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the given object and signal handlers IDs.
    ///
    /// Calls `disconnect_signals` first to remove the previous weak reference
    /// and disconnect the previous signal handlers.
    pub(crate) fn set(&self, obj: &T, signal_handler_ids: Vec<glib::SignalHandlerId>) {
        self.disconnect_signals();

        self.weak_obj.set(Some(obj));
        self.signal_handler_ids.replace(signal_handler_ids);
    }

    /// Get a strong reference to the object.
    pub fn obj(&self) -> Option<T> {
        self.weak_obj.upgrade()
    }

    /// Disconnect the signal handlers and drop the weak reference.
    pub fn disconnect_signals(&self) {
        let signal_handler_ids = self.signal_handler_ids.take();

        if let Some(obj) = self.weak_obj.upgrade() {
            for signal_handler_id in signal_handler_ids {
                obj.disconnect(signal_handler_id);
            }
        }

        self.weak_obj.set(None);
    }
}

impl<T: ObjectType> Default for BoundObjectWeakRef<T> {
    fn default() -> Self {
        Self {
            weak_obj: Default::default(),
            signal_handler_ids: Default::default(),
        }
    }
}

impl<T: ObjectType> Drop for BoundObjectWeakRef<T> {
    fn drop(&mut self) {
        self.disconnect_signals();
    }
}

impl<T: IsA<glib::Object> + glib::HasParamSpec> glib::property::Property for BoundObjectWeakRef<T> {
    type Value = Option<T>;
}

impl<T: IsA<glib::Object>> glib::property::PropertyGet for BoundObjectWeakRef<T> {
    type Value = Option<T>;

    fn get<R, F: Fn(&Self::Value) -> R>(&self, f: F) -> R {
        f(&self.obj())
    }
}

/// Wrapper to manage a bound construct-only object.
///
/// This keeps a strong reference to the object.
#[derive(Debug)]
pub struct BoundConstructOnlyObject<T: ObjectType> {
    obj: OnceCell<T>,
    signal_handler_ids: RefCell<Vec<glib::SignalHandlerId>>,
}

impl<T: ObjectType> BoundConstructOnlyObject<T> {
    /// Creates a new empty `BoundConstructOnlyObject`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the given object and signal handlers IDs.
    ///
    /// Panics if the object was already set.
    pub(crate) fn set(&self, obj: T, signal_handler_ids: Vec<glib::SignalHandlerId>) {
        self.obj.set(obj).unwrap();
        self.signal_handler_ids.replace(signal_handler_ids);
    }

    /// Get a strong reference to the object.
    ///
    /// Panics if the object has not been set yet.
    pub fn obj(&self) -> &T {
        self.obj.get().unwrap()
    }
}

impl<T: ObjectType> Default for BoundConstructOnlyObject<T> {
    fn default() -> Self {
        Self {
            obj: Default::default(),
            signal_handler_ids: Default::default(),
        }
    }
}

impl<T: ObjectType> Drop for BoundConstructOnlyObject<T> {
    fn drop(&mut self) {
        let signal_handler_ids = self.signal_handler_ids.take();

        if let Some(obj) = self.obj.get() {
            for signal_handler_id in signal_handler_ids {
                obj.disconnect(signal_handler_id);
            }
        }
    }
}

impl<T: IsA<glib::Object> + glib::HasParamSpec> glib::property::Property
    for BoundConstructOnlyObject<T>
{
    type Value = T;
}

impl<T: IsA<glib::Object>> glib::property::PropertyGet for BoundConstructOnlyObject<T> {
    type Value = T;

    fn get<R, F: Fn(&Self::Value) -> R>(&self, f: F) -> R {
        f(self.obj())
    }
}

/// Helper type to keep track of ongoing async actions that can succeed in
/// different functions.
///
/// This type can only have one strong reference and many weak references.
///
/// The strong reference should be dropped in the first function where the
/// action succeeds. Then other functions can drop the weak references when
/// they can't be upgraded.
#[derive(Debug)]
pub struct OngoingAsyncAction<T> {
    strong: Rc<AsyncAction<T>>,
}

impl<T> OngoingAsyncAction<T> {
    /// Create a new async action that sets the given value.
    ///
    /// Returns both a strong and a weak reference.
    pub(crate) fn set(value: T) -> (Self, WeakOngoingAsyncAction<T>) {
        let strong = Rc::new(AsyncAction::Set(value));
        let weak = Rc::downgrade(&strong);
        (Self { strong }, WeakOngoingAsyncAction { weak })
    }

    /// Create a new async action that removes a value.
    ///
    /// Returns both a strong and a weak reference.
    pub(crate) fn remove() -> (Self, WeakOngoingAsyncAction<T>) {
        let strong = Rc::new(AsyncAction::Remove);
        let weak = Rc::downgrade(&strong);
        (Self { strong }, WeakOngoingAsyncAction { weak })
    }

    /// Get the inner value, if any.
    pub(crate) fn as_value(&self) -> Option<&T> {
        self.strong.as_value()
    }
}

/// A weak reference to an `OngoingAsyncAction`.
#[derive(Debug, Clone)]
pub struct WeakOngoingAsyncAction<T> {
    weak: Weak<AsyncAction<T>>,
}

impl<T> WeakOngoingAsyncAction<T> {
    /// Whether this async action is still ongoing (i.e. whether the strong
    /// reference still exists).
    pub fn is_ongoing(&self) -> bool {
        self.weak.strong_count() > 0
    }
}

/// An async action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AsyncAction<T> {
    /// An async action is ongoing to set this value.
    Set(T),

    /// An async action is ongoing to remove a value.
    Remove,
}

impl<T> AsyncAction<T> {
    /// Get the inner value, if any.
    pub fn as_value(&self) -> Option<&T> {
        match self {
            Self::Set(value) => Some(value),
            Self::Remove => None,
        }
    }
}

/// A wrapper that requires the tokio runtime to be running when dropped.
#[derive(Debug, Clone)]
pub struct TokioDrop<T>(Option<T>);

impl<T> TokioDrop<T> {
    /// Create a new `TokioDrop` wrapping the given type.
    pub fn new(value: T) -> Self {
        Self(Some(value))
    }
}

impl<T> Deref for TokioDrop<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("TokioDrop should always contain a value")
    }
}

impl<T> From<T> for TokioDrop<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> Drop for TokioDrop<T> {
    fn drop(&mut self) {
        let _guard = RUNTIME.enter();

        if let Some(value) = self.0.take() {
            drop(value);
        }
    }
}

/// The state of a resource that can be loaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, glib::Enum)]
#[enum_type(name = "LoadingState")]
pub enum LoadingState {
    /// It hasn't been loaded yet.
    #[default]
    Initial,
    /// It is currently loading.
    Loading,
    /// It has been fully loaded.
    Ready,
    /// An error occurred while loading it.
    Error,
}

impl From<commune_core::utils::LoadingState> for LoadingState {
    fn from(state: commune_core::utils::LoadingState) -> Self {
        use commune_core::utils::LoadingState as Core;

        match state {
            Core::Initial => Self::Initial,
            Core::Loading => Self::Loading,
            Core::Ready => Self::Ready,
            Core::Error => Self::Error,
        }
    }
}

/// Convert the given checked `bool` to a `GtkAccessibleTristate`.
pub(crate) fn bool_to_accessible_tristate(checked: bool) -> gtk::AccessibleTristate {
    if checked {
        gtk::AccessibleTristate::True
    } else {
        gtk::AccessibleTristate::False
    }
}

/// The path of the given file on the local filesystem, if it has one.
///
/// Use this instead of `gio::File::path()` everywhere the path is about to be
/// opened, read, or handed to something that is not GIO.
///
/// GTK's Android backend answers `g_file_get_path()` for a `content://` URI
/// with `Uri.getPath()`, so a file the picker returned reports a plausible
/// absolute path -- `/document/video:1000000034` -- that no filesystem has.
/// GIO's contract is that a file with no local path returns `NULL`, and every
/// `if let Some(path)` guard here was written against that contract, so the lie
/// defeats all of them at once and the failure surfaces far from the picker
/// that caused it. Checking that the path exists restores the contract without
/// diverging from upstream GTK, and is correct everywhere else too, because a
/// stale path is stale on every platform.
///
/// This is a test for *reading*. A file the user has just chosen as a save
/// destination does not exist yet, so this returns `None` for it; check the
/// parent directory instead in that case.
///
/// See `doc/android-attachments-plan.md` for why this is a helper here rather
/// than a patch to GTK.
pub(crate) fn local_path(file: &gio::File) -> Option<PathBuf> {
    file.path().filter(|path| path.exists())
}

/// A wrapper around several sources of files.
#[derive(Debug, Clone)]
pub enum File {
    /// A `GFile`.
    Gio(gio::File),
    /// A temporary file.
    ///
    /// When all strong references to this file are destroyed, the file will be
    /// destroyed too.
    Temp(Arc<NamedTempFile>),
}

impl File {
    /// Get a `GFile` for this file.
    pub(crate) fn as_gfile(&self) -> gio::File {
        match self {
            Self::Gio(file) => file.clone(),
            Self::Temp(file) => gio::File::for_path(file.path()),
        }
    }
}

impl From<gio::File> for File {
    fn from(value: gio::File) -> Self {
        Self::Gio(value)
    }
}

impl From<NamedTempFile> for File {
    fn from(value: NamedTempFile) -> Self {
        Self::Temp(value.into())
    }
}

/// The directory where to put temporary files.
///
/// Not `glib::user_runtime_dir()` on Android. GTK's glue sets `XDG_DATA_DIRS`,
/// `XDG_DATA_HOME`, `XDG_CONFIG_DIRS` and `XDG_CONFIG_HOME`, and nothing else,
/// so that call falls through to `glib::user_cache_dir()` and then to
/// `$HOME/.cache` — the fallback [`DataType::base_dir_path`] already describes,
/// reached here by a path that had not been changed with it.
///
/// The symptom was not an error message. Every media file the viewer opened
/// failed with `ENOENT`, the viewer took its empty state, and a picture opened
/// from a room showed as a black screen.
static TMP_DIR: LazyLock<Box<Path>> = LazyLock::new(|| {
    #[cfg(target_os = "android")]
    let dir = DataType::Cache.dir_path().join("tmp");

    #[cfg(not(target_os = "android"))]
    let dir = {
        let mut dir = glib::user_runtime_dir();
        dir.push(PROFILE.dir_name().as_ref());
        dir
    };

    dir.into_boxed_path()
});

/// Ensure the temporary file directory exists, and return it.
///
/// `create_dir_all`, not `create_dir`: the parent is not guaranteed to exist.
/// On Android nothing has created the cache directory when the first
/// attachment is opened, and `create_dir` fails with `ENOENT` rather than
/// creating it.
fn ensure_tmp_dir() -> Result<&'static Path, std::io::Error> {
    let dir = TMP_DIR.as_ref();
    if !dir.exists()
        && let Err(error) = fs::create_dir_all(dir)
        && !matches!(error.kind(), io::ErrorKind::AlreadyExists)
    {
        return Err(error);
    }

    Ok(dir)
}

/// Save the given data to a temporary file.
///
/// When all strong references to the returned file are destroyed, the file will
/// be destroyed too.
pub(crate) async fn save_data_to_tmp_file(data: Vec<u8>) -> Result<File, std::io::Error> {
    RUNTIME
        .spawn_blocking(move || {
            let mut file = NamedTempFile::new_in(ensure_tmp_dir()?)?;
            file.write_all(&data)?;

            Ok(file.into())
        })
        .await
        .expect("task was not aborted")
}

/// Create an empty temporary file and return its path, with the file closed.
///
/// Use this instead of [`save_data_to_tmp_file`] when the caller hands the
/// path to something outside GIO that opens the file itself — matrix-sdk's
/// key export writes to its path with `std::fs::File::create`, and on Windows
/// that fails while this process still holds the same file open, which a live
/// `NamedTempFile` does. The file is deleted when the returned `TempPath` is
/// dropped, the same as [`File::Temp`].
pub(crate) async fn tmp_file_path() -> Result<tempfile::TempPath, std::io::Error> {
    RUNTIME
        .spawn_blocking(|| Ok(NamedTempFile::new_in(ensure_tmp_dir()?)?.into_temp_path()))
        .await
        .expect("task was not aborted")
}

/// A counted reference.
///
/// Can be used to perform some actions when the count is 0 or non-zero.
pub struct CountedRef(Rc<InnerCountedRef>);

struct InnerCountedRef {
    /// The count of the reference
    count: Cell<usize>,
    /// The function to call when the count decreases to zero.
    on_zero: Box<dyn Fn()>,
    /// The function to call when the count increases from zero.
    on_non_zero: Box<dyn Fn()>,
}

impl CountedRef {
    /// Construct a counted reference.
    pub(crate) fn new<F1, F2>(on_zero: F1, on_non_zero: F2) -> Self
    where
        F1: Fn() + 'static,
        F2: Fn() + 'static,
    {
        Self(
            InnerCountedRef {
                count: Default::default(),
                on_zero: Box::new(on_zero),
                on_non_zero: Box::new(on_non_zero),
            }
            .into(),
        )
    }

    /// The current count of the reference.
    pub fn count(&self) -> usize {
        self.0.count.get()
    }
}

impl fmt::Debug for CountedRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CountedRef")
            .field("count", &self.count())
            .finish_non_exhaustive()
    }
}

impl Clone for CountedRef {
    fn clone(&self) -> Self {
        let count = self.count();
        self.0.count.set(count.saturating_add(1));

        if count == 0 {
            (self.0.on_non_zero)();
        }

        Self(self.0.clone())
    }
}

impl Drop for CountedRef {
    fn drop(&mut self) {
        let count = self.count();
        self.0.count.set(count.saturating_sub(1));

        if count == 1 {
            (self.0.on_zero)();
        }
    }
}

/// Extensions trait for types with a `child` property.
pub(crate) trait ChildPropertyExt {
    /// The child of this widget, is any.
    fn child_property(&self) -> Option<gtk::Widget>;

    /// Set the child of this widget.
    fn set_child_property(&self, child: Option<&impl IsA<gtk::Widget>>);

    /// Get the child if it is of the proper type, or construct it with the
    /// given function and set is as the child of this widget before returning
    /// it.
    fn child_or_else<W>(&self, f: impl FnOnce() -> W) -> W
    where
        W: IsA<gtk::Widget>,
    {
        if let Some(child) = self.child_property().and_downcast() {
            child
        } else {
            let child = f();
            self.set_child_property(Some(&child));
            child
        }
    }

    /// Get the child if it is of the proper type, or construct it with its
    /// `Default` implementation and set is as the child of this widget before
    /// returning it.
    fn child_or_default<W>(&self) -> W
    where
        W: IsA<gtk::Widget> + Default,
    {
        self.child_or_else(Default::default)
    }
}

impl<W> ChildPropertyExt for W
where
    W: IsABin,
{
    fn child_property(&self) -> Option<gtk::Widget> {
        self.child()
    }

    fn set_child_property(&self, child: Option<&impl IsA<gtk::Widget>>) {
        self.set_child(child);
    }
}

impl ChildPropertyExt for gtk::ListItem {
    fn child_property(&self) -> Option<gtk::Widget> {
        self.child()
    }

    fn set_child_property(&self, child: Option<&impl IsA<gtk::Widget>>) {
        self.set_child(child);
    }
}

/// Helper trait to implement for widgets that subclass `AdwBin`, to be able to
/// use the `ChildPropertyExt` trait.
///
/// This trait is to circumvent conflicts in Rust's type system, where if we try
/// to implement `ChildPropertyExt for W where W: IsA<adw::Bin>` it complains
/// that the other external types that implement `ChildPropertyExt` might
/// implement `IsA<adw::Bin>` in the future… So instead of reimplementing
/// `ChildPropertyExt` for every type where we need it, which requires to
/// implement two methods, we only implement this which requires nothing.
pub(crate) trait IsABin: IsA<adw::Bin> {}

impl IsABin for adw::Bin {}

/// Resample the given slice to the given length, using linear interpolation.
///
/// Returns the slice as-is if it is of the correct length. Returns a `Vec` of
/// zeroes if the slice is empty.
pub(crate) fn resample_slice(slice: &[f32], new_len: usize) -> Cow<'_, [f32]> {
    let len = slice.len();

    if len == new_len {
        // The slice has the correct length, return it.
        return Cow::Borrowed(slice);
    }

    if new_len == 0 {
        // We do not need values, return an empty slice.
        return Cow::Borrowed(&[]);
    }

    if len <= 1
        || slice
            .iter()
            .all(|value| (*value - slice[0]).abs() < 0.000_001)
    {
        // There is a single value so we do not need to interpolate, return a `Vec`
        // containing that value.
        let value = slice.first().copied().unwrap_or_default();
        return Cow::Owned(std::iter::repeat_n(value, new_len).collect());
    }

    // We need to interpolate the values.
    let mut result = Vec::with_capacity(new_len);
    let ratio = (len - 1) as f32 / (new_len - 1) as f32;

    for i in 0..new_len {
        let position_abs = i as f32 * ratio;
        let position_before = position_abs.floor();
        let position_after = position_abs.ceil();
        let position_rel = position_abs % 1.0;

        // We are sure that the positions are positive.
        #[allow(clippy::cast_sign_loss)]
        let value_before = slice[position_before as usize];
        #[allow(clippy::cast_sign_loss)]
        let value_after = slice[(position_after as usize).min(slice.len().saturating_sub(1))];

        let value = (1.0 - position_rel) * value_before + position_rel * value_after;
        result.push(value);
    }

    Cow::Owned(result)
}

/// A helper type to wait for a notification that can occur only one time.
///
/// [`OneshotNotifier::listen()`] must be called to initialize it and get a
/// receiver. The receiver must then be `.await`ed and the future will resolve
/// when it is notified.
///
/// The receiver will receive a signal the first time that
/// [`OneshotNotifier::notify_value()`] is called. Further calls to this
/// function will be noops until a new receiver is created.The value to return
/// must implement `Default`, as this is the value that will be sent to the
/// receiver when the notifier is dropped before a value is notified.
///
/// This notifier can be cloned freely and moved between threads.
///
/// It is also possible to share this notifier between tasks to make sure that a
/// single task is running at a time. If [`OneshotNotifier::listen()`] is called
/// while there is already a receiver waiting, it will be notified as if the
/// notifier was dropped.
#[derive(Debug, Clone)]
pub(crate) struct OneshotNotifier<T = ()> {
    /// The context used to identify the notifier in logs.
    context: &'static str,
    /// The sender for the notification signal.
    sender: Arc<Mutex<Option<oneshot::Sender<T>>>>,
}

impl<T> OneshotNotifier<T> {
    /// Get a new `OneshotNotifier` for the given context.
    pub(crate) fn new(context: &'static str) -> Self {
        Self {
            sender: Default::default(),
            context,
        }
    }
}

impl<T> OneshotNotifier<T>
where
    T: Default + Send + 'static,
{
    /// Initialize this `OneshotNotifier` and get a receiver.
    pub(crate) fn listen(&self) -> OneshotNotifierReceiver<T> {
        let (sender, receiver) = oneshot::channel();

        match self.sender.lock() {
            Ok(mut guard) => *guard = Some(sender),
            Err(error) => {
                error!(
                    context = self.context,
                    "Failed to lock oneshot notifier: {error}"
                );
            }
        }

        OneshotNotifierReceiver(receiver)
    }

    /// Notify the receiver with the given value, if any receiver is still
    /// listening.
    pub(crate) fn notify_value(&self, value: T) {
        match self.sender.lock() {
            Ok(mut guard) => {
                if let Some(sender) = guard.take() {
                    let _ = sender.send(value);
                }
            }
            Err(error) => {
                error!(
                    context = self.context,
                    "Failed to lock oneshot notifier: {error}"
                );
            }
        }
    }

    /// Notify the receiver with the default value, if any receiver is still
    /// listening.
    pub(crate) fn notify(&self) {
        self.notify_value(T::default());
    }
}

/// A notification receiver associated to a [`OneshotNotifier`].
///
/// This should be `.await`ed to wait for a notification.
#[derive(Debug)]
pub(crate) struct OneshotNotifierReceiver<T = ()>(oneshot::Receiver<T>);

impl<T> IntoFuture for OneshotNotifierReceiver<T>
where
    T: Default + Send + 'static,
{
    type Output = T;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.0.await.unwrap_or_default() })
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn a_mangled_pasteboard_uri_gets_its_scheme_back() {
        // The exact string GTK 4.22's macOS backend produces for
        // "/tmp/test file ü.png" dropped from the Finder.
        let broken = gio::File::for_uri("file%3A///tmp/test%20file%20u%CC%88.png");
        assert!(broken.path().is_none());

        let repaired = repair_pasteboard_file(broken);
        assert_eq!(
            repaired.path().as_deref(),
            Some(Path::new("/tmp/test file u\u{308}.png"))
        );
    }

    #[test]
    fn a_healthy_file_is_left_alone() {
        let file = gio::File::for_path("/tmp/plain.png");
        let same = repair_pasteboard_file(file.clone());
        assert_eq!(same.uri(), file.uri());
    }
}

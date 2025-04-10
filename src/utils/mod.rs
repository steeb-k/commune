//! Collection of common methods and types.

use std::{
    borrow::Cow,
    cell::{Cell, OnceCell, RefCell},
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    sync::{Arc, LazyLock},
};

use futures_util::{
    future::{self, Either, Future},
    pin_mut,
};
use gtk::{gio, glib, prelude::*};
use regex::Regex;
use tempfile::NamedTempFile;

pub(crate) mod expression;
mod expression_list_model;
mod grouping_list_model;
pub(crate) mod key_bindings;
mod location;
mod macros;
pub(crate) mod matrix;
pub(crate) mod media;
pub(crate) mod notifications;
mod placeholder_object;
mod single_item_list_model;
pub(crate) mod sourceview;
pub(crate) mod string;
mod template_callbacks;
pub(crate) mod toast;

pub(crate) use self::{
    expression_list_model::ExpressionListModel,
    location::{Location, LocationError, LocationExt},
    placeholder_object::PlaceholderObject,
    single_item_list_model::SingleItemListModel,
    template_callbacks::TemplateCallbacks,
};
use crate::{PROFILE, RUNTIME};

/// The path of the directory where data should be stored, depending on its
/// type.
pub(crate) fn data_dir_path(data_type: DataType) -> PathBuf {
    let mut path = match data_type {
        DataType::Persistent => glib::user_data_dir(),
        DataType::Cache => glib::user_cache_dir(),
    };
    path.push(PROFILE.dir_name().as_ref());

    path
}

/// The type of data.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DataType {
    /// Data that should not be deleted.
    Persistent,
    /// Cache that can be deleted freely.
    Cache,
}

pub(crate) enum TimeoutFuture {
    Timeout,
}

/// Executes the given future with the given timeout.
///
/// If the future didn't resolve before the timeout was reached, this returns
/// an `Err(TimeoutFuture)`.
pub(crate) async fn timeout_future<T>(
    timeout: std::time::Duration,
    fut: impl Future<Output = T>,
) -> Result<T, TimeoutFuture> {
    let timeout = glib::timeout_future(timeout);
    pin_mut!(fut);

    match future::select(fut, timeout).await {
        Either::Left((x, _)) => Ok(x),
        Either::Right(_) => Err(TimeoutFuture::Timeout),
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

/// A type that requires the tokio runtime to be running when dropped.
///
/// This is basically usable as a [`OnceCell`].
#[derive(Debug, Clone)]
pub struct TokioDrop<T>(OnceCell<T>);

impl<T> TokioDrop<T> {
    /// Create a new empty `TokioDrop`;
    pub fn new() -> Self {
        Self::default()
    }

    /// Gets a reference to the underlying value.
    ///
    /// Returns `None` if the cell is empty.
    pub fn get(&self) -> Option<&T> {
        self.0.get()
    }

    /// Sets the contents of this cell to `value`.
    ///
    /// Returns `Ok(())` if the cell was empty and `Err(value)` if it was full.
    pub(crate) fn set(&self, value: T) -> Result<(), T> {
        self.0.set(value)
    }
}

impl<T> Default for TokioDrop<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T> Drop for TokioDrop<T> {
    fn drop(&mut self) {
        let _guard = RUNTIME.enter();

        if let Some(inner) = self.0.take() {
            drop(inner);
        }
    }
}

impl<T: glib::property::Property> glib::property::Property for TokioDrop<T> {
    type Value = T::Value;
}

impl<T> glib::property::PropertyGet for TokioDrop<T> {
    type Value = T;

    fn get<R, F: Fn(&Self::Value) -> R>(&self, f: F) -> R {
        f(self.get().unwrap())
    }
}

impl<T> glib::property::PropertySet for TokioDrop<T> {
    type SetValue = T;

    fn set(&self, v: Self::SetValue) {
        assert!(
            self.set(v).is_ok(),
            "TokioDrop value was already initialized"
        );
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

/// Convert the given checked `bool` to a `GtkAccessibleTristate`.
pub(crate) fn bool_to_accessible_tristate(checked: bool) -> gtk::AccessibleTristate {
    if checked {
        gtk::AccessibleTristate::True
    } else {
        gtk::AccessibleTristate::False
    }
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
    /// The path to the file.
    pub(crate) fn path(&self) -> Option<PathBuf> {
        match self {
            Self::Gio(file) => file.path(),
            Self::Temp(file) => Some(file.path().to_owned()),
        }
    }

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
static TMP_DIR: LazyLock<Box<Path>> = LazyLock::new(|| {
    let mut dir = glib::user_runtime_dir();
    dir.push(PROFILE.dir_name().as_ref());
    dir.into_boxed_path()
});

/// Save the given data to a temporary file.
///
/// When all strong references to the returned file are destroyed, the file will
/// be destroyed too.
pub(crate) async fn save_data_to_tmp_file(data: Vec<u8>) -> Result<File, std::io::Error> {
    RUNTIME
        .spawn_blocking(move || {
            let dir = TMP_DIR.as_ref();
            if !dir.exists() {
                if let Err(error) = fs::create_dir(dir) {
                    if !matches!(error.kind(), io::ErrorKind::AlreadyExists) {
                        return Err(error);
                    }
                }
            }
            let mut file = NamedTempFile::new_in(dir)?;
            file.write_all(&data)?;

            Ok(file.into())
        })
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

//! Small shared helpers, lifted from the application's `utils` as they are
//! needed.

use std::ops::Deref;

use crate::RUNTIME;

/// A type that should be dropped from inside the tokio runtime.
///
/// Some SDK types spawn tasks or take async locks when they drop. In the
/// application the drop could happen on the GTK thread; in the core it can
/// happen on whatever thread the FFI caller releases the last reference
/// from. Entering the runtime first keeps either case sound.
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

impl<T: std::fmt::Debug> std::fmt::Debug for TokioDrop<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The state of a resource that can be loaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
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

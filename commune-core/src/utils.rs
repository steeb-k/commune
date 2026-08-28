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

/// Common extensions for mutable strings.
///
/// The portable subset of the application's `utils::string::StrMutExt` —
/// the Pango-markup half stayed behind.
pub trait StrMutExt {
    /// Remove the whitespaces at the end of the string.
    fn truncate_end_whitespaces(&mut self);

    /// Remove the NUL bytes from the string.
    ///
    /// NUL is not renderable and some platform string types would truncate
    /// at the first NUL byte.
    fn strip_nul(&mut self);

    /// Remove unnecessary or problematic characters from the string.
    fn clean_string(&mut self) {
        self.strip_nul();
        self.truncate_end_whitespaces();
    }
}

impl StrMutExt for String {
    fn truncate_end_whitespaces(&mut self) {
        if self.is_empty() {
            return;
        }

        let new_len = self
            .char_indices()
            .rfind(|(_, c)| !c.is_whitespace())
            .map(|(idx, c)| {
                // We have the position of the last non-whitespace character, so the last
                // whitespace character is the character after it.
                idx + c.len_utf8()
            })
            // 0 means that there are only whitespaces in the string.
            .unwrap_or_default();

        self.truncate(new_len);
    }

    fn strip_nul(&mut self) {
        self.retain(|c| c != '\0');
    }
}

/// Extensions to `Option<String>`.
pub trait OptionStringExt: Sized {
    /// Remove unnecessary or problematic characters from the string.
    ///
    /// If the final string is empty, replaces it with `None`.
    fn clean_string(&mut self);

    /// Remove unnecessary or problematic characters from the string.
    ///
    /// If the final string is empty, replaces it with `None`.
    #[must_use]
    fn into_clean_string(mut self) -> Self {
        self.clean_string();
        self
    }
}

impl OptionStringExt for Option<String> {
    fn clean_string(&mut self) {
        self.take_if(|s| {
            s.clean_string();
            s.is_empty()
        });
    }
}

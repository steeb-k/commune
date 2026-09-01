//! The core's named settings, kept in the application's `GSettings`.
//!
//! The core asks its embedder where settings live, and for the desktop the
//! answer has to be `GSettings`: the session list's order and every session's
//! own settings are already in the `sessions` key of
//! `io.github.steeb_k.Commune`, put there by every version of this
//! application so far. Letting the core fall back to its own JSON file would
//! not break anything visibly — it would quietly start from defaults, and an
//! upgrade would look like the sidebar forgetting the order of the accounts.
//!
//! **The awkward part is threads.** `SettingsStore` is `Send + Sync`, because
//! the core reaches it from wherever its work happens — `SessionList`'s
//! restore is `async` and runs on a tokio worker. `gio::Settings` is a
//! `GObject` and is neither. The two cannot simply be put together.
//!
//! So the values are mirrored in memory. Reads are served from the mirror and
//! never touch `GSettings` at all; writes update the mirror and then hand the
//! real write to the main context with `invoke()`, which is callable from any
//! thread and is the one primitive that makes this safe. Nothing blocks a
//! tokio worker on the main loop, which the obvious alternative — a
//! round-trip per call — would do, and would deadlock the moment the main
//! loop was itself waiting on that task.
//!
//! The cost is that a `set` is not durable the instant it returns. It was
//! never durable in that sense: `GSettings` batches its own writes too.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use commune_core::settings::SettingsStore;
use gtk::{gio, glib, prelude::*};
use tracing::warn;

/// A [`SettingsStore`] over the application's `GSettings`.
#[derive(Debug)]
pub(crate) struct GSettingsStore {
    /// The string-valued settings, mirrored so that reads need no main
    /// context.
    values: Mutex<HashMap<String, String>>,
}

impl GSettingsStore {
    /// Mirror the string-valued keys of the given settings.
    ///
    /// Must be called on the main thread, which is the only place a
    /// `gio::Settings` may be touched; `Application::startup` is where.
    pub(crate) fn new(settings: &gio::Settings) -> Arc<Self> {
        let mut values = HashMap::new();

        for key in settings
            .settings_schema()
            .map(|schema| schema.list_keys())
            .unwrap_or_default()
        {
            let value = settings.value(&key);

            // Only the string keys: those are the ones the core can ask for,
            // and a `GVariant` of any other type has no `String` to mirror.
            if let Some(string) = value.get::<String>() {
                values.insert(key.to_string(), string);
            }
        }

        let store = Arc::new(Self {
            values: Mutex::new(values),
        });

        // Keep the mirror honest. Nothing else should be writing the keys the
        // core reads — but "should" is not a guarantee, the application still
        // writes some of its own settings directly, and a mirror that can go
        // stale is a mirror that will. `changed` fires on the main context,
        // which is where a `gio::Settings` belongs anyway.
        settings.connect_changed(
            None,
            glib::clone!(
                #[weak]
                store,
                move |settings, key| {
                    if let Some(string) = settings.value(key).get::<String>() {
                        store
                            .values
                            .lock()
                            .expect("settings mirror is not poisoned")
                            .insert(key.to_owned(), string);
                    }
                }
            ),
        );

        store
    }
}

impl SettingsStore for GSettingsStore {
    fn get(&self, key: &str) -> Option<String> {
        self.values
            .lock()
            .expect("settings mirror is not poisoned")
            .get(key)
            .cloned()
    }

    fn set(&self, key: &str, value: &str) {
        self.values
            .lock()
            .expect("settings mirror is not poisoned")
            .insert(key.to_owned(), value.to_owned());

        let key = key.to_owned();
        let value = value.to_owned();

        // `invoke` is safe from any thread and runs the closure on the main
        // context, where a `gio::Settings` may exist. It is constructed in
        // there rather than captured for the same reason the mirror exists at
        // all: the object cannot cross a thread boundary.
        glib::MainContext::default().invoke(move || {
            let settings = gio::Settings::new(crate::APP_ID);

            if let Err(error) = settings.set_string(&key, &value) {
                warn!("Could not save the {key} setting: {error}");
            }
        });
    }
}

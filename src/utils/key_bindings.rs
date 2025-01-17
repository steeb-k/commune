//! Helpers to add key bindings to widgets.

use gtk::{gdk, subclass::prelude::*};

/// List of keys that activate a widget.
// Copied from GtkButton's source code.
pub(crate) const ACTIVATE_KEYS: &[gdk::Key] = &[
    gdk::Key::space,
    gdk::Key::KP_Space,
    gdk::Key::Return,
    gdk::Key::ISO_Enter,
    gdk::Key::KP_Enter,
];

/// Activate the given action when one of the [`ACTIVATE_KEYS`] binding is
/// triggered.
pub(crate) fn add_activate_bindings<T: WidgetClassExt>(klass: &mut T, action: &str) {
    for key in ACTIVATE_KEYS {
        klass.add_binding_action(*key, gdk::ModifierType::empty(), action);
    }
}

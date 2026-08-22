//! Helpers to add key bindings to widgets.

use gtk::{gdk, subclass::prelude::*};

/// The modifier for the platform's primary accelerator.
///
/// This is the Rust counterpart of GTK's `<Primary>` accelerator name: Command
/// on macOS, Control everywhere else.
#[expect(
    dead_code,
    reason = "the sweep replacing the hardcoded CONTROL_MASK bindings is M3 of doc/macos-plan.md"
)]
pub(crate) const PRIMARY_MASK: gdk::ModifierType = if cfg!(target_os = "macos") {
    gdk::ModifierType::META_MASK
} else {
    gdk::ModifierType::CONTROL_MASK
};

/// List of keys that activate a widget.
// Copied from GtkButton's source code.
const ACTIVATE_KEYS: &[gdk::Key] = &[
    gdk::Key::space,
    gdk::Key::KP_Space,
    gdk::Key::Return,
    gdk::Key::ISO_Enter,
    gdk::Key::KP_Enter,
];

/// Add key bindings to the given class to trigger the given action to activate
/// a widget.
pub(crate) fn add_activate_bindings<T: WidgetClassExt>(klass: &mut T, action: &str) {
    for key in ACTIVATE_KEYS {
        klass.add_binding_action(*key, gdk::ModifierType::empty(), action);
    }
}

/// List of key and modifier combos that trigger a context menu to appear.
const CONTEXT_MENU_BINDINGS: &[(gdk::Key, gdk::ModifierType)] = &[
    (gdk::Key::F10, gdk::ModifierType::SHIFT_MASK),
    (gdk::Key::Menu, gdk::ModifierType::empty()),
];

/// Add key bindings to the given class to trigger the given action to show a
/// context menu.
pub(crate) fn add_context_menu_bindings<T: WidgetClassExt>(klass: &mut T, action: &str) {
    for (key, modifier) in CONTEXT_MENU_BINDINGS {
        klass.add_binding_action(*key, *modifier, action);
    }
}

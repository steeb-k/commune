//! Drawing the state of a password the user is choosing.
//!
//! Three pages ask somebody to invent a password — changing it, signing up and
//! resetting it — and all three say the same things about it in the same
//! widgets. The rules themselves are [`validate_password`], which is about the
//! Matrix specification rather than about GTK; these two functions are the
//! part that draws them.

use adw::prelude::*;
use gettextrs::gettext;

use super::matrix::validate_password;

/// Draw how good the password in `entry` is.
///
/// The meter fills in five steps, and the first rule the password fails is
/// named in `label` behind `revealer`. Returns whether the password passes
/// every rule.
pub(crate) fn draw_password_validity(
    entry: &adw::PasswordEntryRow,
    progress: &gtk::LevelBar,
    revealer: &gtk::Revealer,
    label: &gtk::Label,
) -> bool {
    let password = entry.text();

    if password.is_empty() {
        revealer.set_reveal_child(false);
        entry.remove_css_class("success");
        entry.remove_css_class("warning");
        progress.set_value(0.0);
        progress.remove_css_class("success");
        progress.remove_css_class("warning");
        return false;
    }

    let validity = validate_password(&password);

    progress.set_value(f64::from(validity.progress) / 20.0);

    if validity.progress == 100 {
        revealer.set_reveal_child(false);
        entry.add_css_class("success");
        entry.remove_css_class("warning");
        progress.add_css_class("success");
        progress.remove_css_class("warning");
        return true;
    }

    entry.remove_css_class("success");
    entry.add_css_class("warning");
    progress.remove_css_class("success");
    progress.add_css_class("warning");

    // Only the first missing rule is named. A list of five complaints about a
    // half-typed password is noise.
    if !validity.has_length {
        label.set_label(&gettext("Password must be at least 8 characters long"));
    } else if !validity.has_lowercase {
        label.set_label(&gettext(
            "Password must have at least one lower-case letter",
        ));
    } else if !validity.has_uppercase {
        label.set_label(&gettext(
            "Password must have at least one upper-case letter",
        ));
    } else if !validity.has_number {
        label.set_label(&gettext("Password must have at least one digit"));
    } else if !validity.has_symbol {
        label.set_label(&gettext("Password must have at least one symbol"));
    }

    revealer.set_reveal_child(true);

    false
}

/// Draw whether the confirmation in `entry` matches `password`.
///
/// Returns whether the two match. An empty confirmation says nothing rather
/// than complaining about what has not been typed yet, and does not match.
pub(crate) fn draw_password_confirmation(
    password: &str,
    entry: &adw::PasswordEntryRow,
    revealer: &gtk::Revealer,
    label: &gtk::Label,
) -> bool {
    let confirmation = entry.text();

    if confirmation.is_empty() {
        revealer.set_reveal_child(false);
        entry.remove_css_class("success");
        entry.remove_css_class("warning");
        return false;
    }

    if password == confirmation {
        revealer.set_reveal_child(false);
        entry.add_css_class("success");
        entry.remove_css_class("warning");
        return true;
    }

    entry.remove_css_class("success");
    entry.add_css_class("warning");
    label.set_label(&gettext("Passwords do not match"));
    revealer.set_reveal_child(true);

    false
}

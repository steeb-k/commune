//! A document font libadwaita can put in a stylesheet, on macOS.
//!
//! libadwaita sets `--document-font-family` on the root of the stylesheet
//! from the platform's document font, and where there is no such setting —
//! macOS has none — it falls back to GTK's own `gtk-font-name`. On macOS that
//! is the system font, which the backend reports as `.AppleSystemUIFont`,
//! and libadwaita writes the family into the CSS unquoted. A family that
//! starts with a full stop is not a CSS identifier, so every widget styled
//! with the `document` class — every message body in the timeline — logs a
//! "Theme parser error" as its style is computed, in a run that goes past a
//! thousand lines in an ordinary session.
//!
//! The variable is only read, never the font itself, so the fix is to define
//! it again, quoted, from a provider that outranks libadwaita's. The family
//! stays what it was.

use gtk::{glib, pango};
use tracing::{debug, warn};

/// The provider holding the definition, kept so a font change can replace it.
static PROVIDER: std::sync::LazyLock<glib::thread_guard::ThreadGuard<gtk::CssProvider>> =
    std::sync::LazyLock::new(|| glib::thread_guard::ThreadGuard::new(gtk::CssProvider::new()));

/// Define the document font family from GTK's font, quoted.
///
/// Must be called after `gtk::init()`, which is where the settings this reads
/// come from.
pub(crate) fn init() {
    let Some(settings) = gtk::Settings::default() else {
        warn!("Could not set the document font: there are no settings to read it from");
        return;
    };
    let Some(display) = gtk::gdk::Display::default() else {
        warn!("Could not set the document font: there is no display to style");
        return;
    };

    gtk::style_context_add_provider_for_display(
        &display,
        PROVIDER.get_ref(),
        // libadwaita defines the variable at theme priority; this has to
        // come later in the cascade.
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    update(&settings);
    settings.connect_gtk_font_name_notify(update);
}

/// Load the definition for the current `gtk-font-name`.
fn update(settings: &gtk::Settings) {
    let Some(font_name) = settings.gtk_font_name() else {
        return;
    };
    let description = pango::FontDescription::from_string(&font_name);
    let Some(family) = description.family() else {
        debug!("GTK's font name {font_name:?} names no family; leaving the document font alone");
        return;
    };

    // A CSS string: double quotes and backslashes inside it are escaped.
    let escaped = family.replace('\\', "\\\\").replace('"', "\\\"");
    PROVIDER.get_ref().load_from_string(&format!(
        ":root {{ --document-font-family: \"{escaped}\"; }}"
    ));
    debug!("Set the document font family to {family:?}");
}

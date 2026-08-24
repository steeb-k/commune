//! The sound a call makes before somebody picks it up.
//!
//! The freedesktop sound theme names both of them — `phone-incoming-call` for
//! a call arriving and `phone-outgoing-calling` for the ringback of one going
//! out — so this plays the theme the desktop is already using rather than a
//! sound of our own. A theme that names neither is silent, which is the same
//! answer as a machine with no speakers: worth logging, not worth inventing a
//! beep for.

use std::path::PathBuf;

use gst::prelude::*;
use gtk::{gio, gio::prelude::SettingsExt, glib};
use tracing::{debug, warn};

/// The sound theme event for a call coming in.
const INCOMING_SOUND: &str = "phone-incoming-call";

/// The sound theme event for a call going out.
const OUTGOING_SOUND: &str = "phone-outgoing-calling";

/// The theme every freedesktop system has, and the one the specification says
/// to fall back to.
const FALLBACK_THEME: &str = "freedesktop";

/// The settings that say whether the desktop makes sounds at all.
const SOUND_SCHEMA: &str = "org.gnome.desktop.sound";

/// A sound that repeats until it is dropped.
#[derive(Debug)]
pub(crate) struct Ringtone {
    /// The player, stopped when this is dropped.
    playbin: gst::Element,
    /// The bus watch that starts the sound again when it ends.
    _bus_guard: Option<gst::bus::BusWatchGuard>,
}

impl Ringtone {
    /// Ring for a call that is coming in.
    pub(crate) fn incoming() -> Option<Self> {
        Self::play(INCOMING_SOUND)
    }

    /// Ring back for a call that is going out.
    pub(crate) fn outgoing() -> Option<Self> {
        Self::play(OUTGOING_SOUND)
    }

    /// Start the given sound theme event, on a loop.
    fn play(event: &str) -> Option<Self> {
        if !event_sounds_enabled() {
            debug!("Not ringing: the desktop has event sounds switched off");
            return None;
        }

        let path = sound_file(event)?;
        let uri = glib::filename_to_uri(&path, None).ok()?;

        let playbin = gst::ElementFactory::make("playbin3")
            .property("uri", uri.as_str())
            .build()
            .ok()?;

        // The theme's files are one ring each, not a ringing telephone. What
        // makes it a ringing telephone is playing it again when it ends.
        let bus_guard = playbin.bus().and_then(|bus| {
            let player = playbin.clone();
            bus.add_watch_local(move |_, message| {
                match message.view() {
                    gst::MessageView::Eos(_) => {
                        let _ = player.seek_simple(gst::SeekFlags::FLUSH, gst::ClockTime::ZERO);
                    }
                    gst::MessageView::Error(error) => {
                        warn!("Could not play the ringtone: {}", error.error());
                    }
                    _ => {}
                }

                glib::ControlFlow::Continue
            })
            .ok()
        });

        if playbin.set_state(gst::State::Playing).is_err() {
            warn!("Could not start the ringtone");
            return None;
        }

        debug!("Ringing with {}", path.display());

        Some(Self {
            playbin,
            _bus_guard: bus_guard,
        })
    }
}

impl Drop for Ringtone {
    fn drop(&mut self) {
        if self.playbin.set_state(gst::State::Null).is_err() {
            warn!("Could not stop the ringtone");
        }
    }
}

/// Whether the desktop wants to be heard.
///
/// A person who switched event sounds off switched off the sound a telephone
/// makes as well: the sound theme is where both of them live, and this is the
/// switch above it. The notification still arrives.
fn event_sounds_enabled() -> bool {
    let Some(settings) = sound_settings() else {
        // No schema, no opinion. Every other desktop rings.
        return true;
    };

    settings.boolean("event-sounds")
}

/// The name of the sound theme in use.
fn sound_theme() -> String {
    sound_settings()
        .map(|settings| settings.string("theme-name").to_string())
        .filter(|theme| !theme.is_empty())
        .unwrap_or_else(|| FALLBACK_THEME.to_owned())
}

/// The desktop's sound settings, if this desktop has any.
fn sound_settings() -> Option<gio::Settings> {
    // Constructing a `gio::Settings` for a schema that is not installed aborts
    // the process, so the schema is looked up first.
    gio::SettingsSchemaSource::default()?.lookup(SOUND_SCHEMA, true)?;

    Some(gio::Settings::new(SOUND_SCHEMA))
}

/// Find the file for a sound theme event.
///
/// The XDG sound theme specification's layout, and only the part of it that is
/// ever used in practice: `<data dir>/sounds/<theme>/stereo/<event>.<ext>`.
/// Theme inheritance is not walked; the theme itself and then `freedesktop`
/// is what every implementation ends up doing anyway.
fn sound_file(event: &str) -> Option<PathBuf> {
    let theme = sound_theme();
    let mut themes = vec![theme.as_str()];

    if theme != FALLBACK_THEME {
        themes.push(FALLBACK_THEME);
    }

    let mut dirs = vec![glib::user_data_dir()];
    dirs.extend(glib::system_data_dirs());

    for dir in &dirs {
        for theme in &themes {
            for extension in ["oga", "ogg", "wav"] {
                let path = dir
                    .join("sounds")
                    .join(theme)
                    .join("stereo")
                    .join(format!("{event}.{extension}"));

                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }

    debug!("No sound theme has an event called {event}; the call will be silent");

    None
}

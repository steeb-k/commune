//! The application's seam onto [`commune_core::secret`].
//!
//! The backends themselves — the Linux keyring and Secret Portal, the macOS
//! Keychain, the Windows Credential Manager, the sealed files on Android, and
//! the `SecretFile` that encrypts the tokens next to the databases — used to
//! live under `src/secret/`, transcribed a second time into the core. They
//! are the core's now, and what is left here is the part that cannot be:
//! `glib`, and `gettext`.
//!
//! **The wrapper.** [`StoredSession`] is a construct-only `GObject` property
//! on `SessionInfo`, so it has to be a `glib::Boxed` type, and the core's
//! struct cannot derive one — the core has no `glib`, and the orphan rule
//! forbids deriving a foreign trait for a foreign type here. So the
//! application keeps a newtype with the derive on it and a `Deref` through
//! to the core's, and every field access reads exactly as it did.
//!
//! **The strings.** The core returns [`SecretError`] as a value, never as a
//! sentence — that is what [`commune_core::secret::KeyringError`] is for —
//! and the `UserFacingError` implementation below is where each one becomes
//! English that `gettext` can translate. The other direction is the label the
//! keyring shows for a stored session, which the application hands to the
//! core already translated at `config::init()` time; see
//! `Application::startup`.

use std::{fmt, ops::Deref};

pub(crate) use commune_core::secret::{SESSION_ID_LENGTH, Secret, SecretError, SecretExt};
use gettextrs::gettext;
use gtk::glib;
use matrix_sdk::Client;

use crate::{prelude::*, utils::matrix::ClientSetupError};

/// A session, as stored in the secret service.
///
/// A newtype around [`commune_core::secret::StoredSession`], for the reason
/// this module's documentation gives. It dereferences to the core's struct,
/// so the fields and the `&self` methods are reached as if it were one.
#[derive(Clone, glib::Boxed)]
#[boxed_type(name = "StoredSession")]
pub struct StoredSession(commune_core::secret::StoredSession);

impl StoredSession {
    /// Construct a `StoredSession` from the session of the given Matrix
    /// client.
    ///
    /// Returns an error if we failed to generate a unique session ID for the
    /// new session.
    pub(crate) async fn new(client: &Client) -> Result<Self, ClientSetupError> {
        commune_core::secret::StoredSession::new(client)
            .await
            .map(Self)
    }

    /// The core's session inside this wrapper.
    ///
    /// Needed where the core takes one by value and `Deref` cannot help.
    pub(crate) fn into_inner(self) -> commune_core::secret::StoredSession {
        self.0
    }
}

impl Deref for StoredSession {
    type Target = commune_core::secret::StoredSession;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<commune_core::secret::StoredSession> for StoredSession {
    fn from(value: commune_core::secret::StoredSession) -> Self {
        Self(value)
    }
}

impl fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl UserFacingError for SecretError {
    fn to_user_facing(&self) -> String {
        match self {
            // Already a sentence when it reaches us: it is whatever the
            // platform said, and there is nothing better to say about it.
            SecretError::Service(error) => error.clone(),
            #[cfg(target_os = "linux")]
            SecretError::Keyring(error) => error.to_user_facing(),
        }
    }
}

#[cfg(target_os = "linux")]
impl UserFacingError for commune_core::secret::KeyringError {
    fn to_user_facing(&self) -> String {
        use commune_core::secret::KeyringError;

        match self {
            KeyringError::CorruptedFile => gettext("The secret storage file is corrupted."),
            KeyringError::NoFileLocation => {
                gettext("Could not access the secret storage file location.")
            }
            KeyringError::FileIo => {
                gettext("An unexpected error occurred when accessing the secret storage file.")
            }
            KeyringError::FileChanged => {
                gettext("The secret storage file has been changed by another process.")
            }
            KeyringError::PortalCancelled => gettext(
                "The request to the Flatpak Secret Portal was cancelled. Make sure to accept any prompt asking to access it.",
            ),
            KeyringError::PortalNotAvailable => gettext(
                "The Flatpak Secret Portal is not available. Make sure xdg-desktop-portal is installed, and it is at least at version 1.5.0.",
            ),
            KeyringError::Portal => gettext(
                "An unexpected error occurred when interacting with the D-Bus Secret Portal backend.",
            ),
            KeyringError::PortalWeakKey => {
                gettext("The Flatpak Secret Portal provided a key that is too weak to be secure.")
            }
            KeyringError::Locked => gettext("The collection or item is locked."),
            KeyringError::ItemDeleted => gettext("The item was deleted."),
            KeyringError::Service => gettext(
                "An unexpected error occurred when interacting with the D-Bus Secret Service.",
            ),
            KeyringError::NoServiceSession => {
                gettext("The D-Bus Secret Service session does not exist.")
            }
            KeyringError::NoSuchObject => gettext("The collection or item does not exist."),
            KeyringError::ServiceDismissed => gettext(
                "The request to the D-Bus Secret Service was cancelled. Make sure to accept any prompt asking to access it.",
            ),
            KeyringError::NoDefaultCollection => gettext(
                "Could not access the default collection. Make sure a keyring was created and set as default.",
            ),
        }
    }
}

/// The label the platform's secret backend shows for a stored session.
///
/// Handed to the core at startup as a template, because the core is where the
/// session is written and `gettext` is not something it can reach. The
/// substitution is the core's; this only says what the sentence is.
pub(crate) fn credential_label_template() -> String {
    // Translators: Do NOT translate the content between '{' and '}', this is a
    // variable name.
    gettext("Commune: Matrix credentials for {user_id}")
}

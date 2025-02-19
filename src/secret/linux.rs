//! Linux API to store the data of a session, using the Secret Service or Secret
//! portal.

use std::{collections::HashMap, path::Path};

use gettextrs::gettext;
use oo7::{Item, Keyring};
use ruma::UserId;
use thiserror::Error;
use tokio::fs;
use tracing::{debug, error, info};
use url::Url;

use super::{Secret, SecretError, StoredSession, SESSION_ID_LENGTH};
use crate::{gettext_f, prelude::*, spawn_tokio, utils::matrix, APP_ID, PROFILE};

/// The current version of the stored session.
pub const CURRENT_VERSION: u8 = 6;
/// The minimum supported version for the stored sessions.
///
/// Currently, this matches the version when Fractal 5 was released.
pub const MIN_SUPPORTED_VERSION: u8 = 4;

/// Keys used in the Linux secret backend.
mod keys {
    /// The attribute for the schema in the Secret Service.
    pub(super) const XDG_SCHEMA: &str = "xdg:schema";
    /// The attribute for the profile of the app.
    pub(super) const PROFILE: &str = "profile";
    /// The attribute for the version of the stored session.
    pub(super) const VERSION: &str = "version";
    /// The attribute for the URL of the homeserver.
    pub(super) const HOMESERVER: &str = "homeserver";
    /// The attribute for the user ID.
    pub(super) const USER: &str = "user";
    /// The attribute for the device ID.
    pub(super) const DEVICE_ID: &str = "device-id";
    /// The deprecated attribute for the database path.
    pub(super) const DB_PATH: &str = "db-path";
    /// The attribute for the session ID.
    pub(super) const ID: &str = "id";
}

/// Retrieves all sessions stored in the secret backend.
pub async fn restore_sessions() -> Result<Vec<StoredSession>, SecretError> {
    match restore_sessions_inner().await {
        Ok(sessions) => Ok(sessions),
        Err(error) => {
            error!("Could not restore previous sessions: {error}");
            Err(error.into())
        }
    }
}

async fn restore_sessions_inner() -> Result<Vec<StoredSession>, oo7::Error> {
    let keyring = Keyring::new().await?;

    keyring.unlock().await?;

    let items = keyring
        .search_items(&HashMap::from([
            (keys::XDG_SCHEMA, APP_ID),
            (keys::PROFILE, PROFILE.as_str()),
        ]))
        .await?;

    let mut sessions = Vec::with_capacity(items.len());

    for item in items {
        item.unlock().await?;

        match StoredSession::try_from_secret_item(item).await {
            Ok(session) => sessions.push(session),
            Err(LinuxSecretError::OldVersion {
                version,
                mut session,
                attributes,
            }) => {
                if version < MIN_SUPPORTED_VERSION {
                    info!(
                        "Found old session for user {} with version {version} that is no longer supported, removing…",
                        session.user_id
                    );

                    // Try to log it out.
                    log_out_session(session.clone()).await;

                    // Delete the session from the secret backend.
                    delete_session(&session).await;

                    // Delete the session data folders.
                    spawn_tokio!(async move {
                        if let Err(error) = fs::remove_dir_all(session.data_path()).await {
                            error!("Could not remove session database: {error}");
                        }

                        if version >= 6 {
                            if let Err(error) = fs::remove_dir_all(session.cache_path()).await {
                                error!("Could not remove session cache: {error}");
                            }
                        }
                    })
                    .await
                    .unwrap();

                    continue;
                }

                info!(
                    "Found session {} for user {} with old version {}, applying migrations…",
                    session.id, session.user_id, version,
                );
                session.apply_migrations(version, attributes).await;

                sessions.push(session);
            }
            Err(LinuxSecretError::Field(LinuxSecretFieldError::Invalid)) => {
                // We already log the specific errors for this.
            }
            Err(error) => {
                error!("Could not restore previous session: {error}");
            }
        }
    }

    Ok(sessions)
}

/// Write the given session to the secret backend.
///
/// Note that this overwrites any previously stored session with the same
/// attributes.
pub async fn store_session(session: StoredSession) -> Result<(), SecretError> {
    match store_session_inner(session).await {
        Ok(()) => Ok(()),
        Err(error) => {
            error!("Could not store session: {error}");
            Err(error.into())
        }
    }
}

async fn store_session_inner(session: StoredSession) -> Result<(), oo7::Error> {
    let keyring = Keyring::new().await?;

    let attributes = session.attributes();
    let secret = serde_json::to_string(&session.secret).unwrap();

    keyring
        .create_item(
            &gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                "Fractal: Matrix credentials for {user_id}",
                &[("user_id", session.user_id.as_str())],
            ),
            &attributes,
            secret,
            true,
        )
        .await?;

    Ok(())
}

/// Delete the given session from the secret backend.
pub async fn delete_session(session: &StoredSession) {
    let attributes = session.attributes();

    spawn_tokio!(async move {
        if let Err(error) = delete_item_with_attributes(&attributes).await {
            error!("Could not delete session data from secret backend: {error}");
        }
    })
    .await
    .unwrap();
}

/// Create a client and log out the given session.
async fn log_out_session(session: StoredSession) {
    debug!("Logging out session");
    spawn_tokio!(async move {
        match matrix::client_with_stored_session(session).await {
            Ok(client) => {
                if let Err(error) = client.matrix_auth().logout().await {
                    error!("Could not log out session: {error}");
                }
            }
            Err(error) => {
                error!("Could not build client to log out session: {error}");
            }
        }
    })
    .await
    .unwrap();
}

impl StoredSession {
    /// Build self from a secret.
    async fn try_from_secret_item(item: Item) -> Result<Self, LinuxSecretError> {
        let attributes = item.attributes().await?;

        let version = parse_attribute(&attributes, keys::VERSION, str::parse::<u8>)?;
        if version > CURRENT_VERSION {
            return Err(LinuxSecretError::UnsupportedVersion(version));
        }

        let homeserver = parse_attribute(&attributes, keys::HOMESERVER, Url::parse)?;
        let user_id = parse_attribute(&attributes, keys::USER, |s| UserId::parse(s))?;
        let device_id = get_attribute(&attributes, keys::DEVICE_ID)?.as_str().into();
        let id = if version <= 5 {
            let string = get_attribute(&attributes, keys::DB_PATH)?;
            Path::new(string)
                .iter()
                .next_back()
                .and_then(|s| s.to_str())
                .expect("Session ID in db-path should be valid UTF-8")
                .to_owned()
        } else {
            get_attribute(&attributes, keys::ID)?.clone()
        };
        let secret = match item.secret().await {
            Ok(secret) => {
                if version <= 4 {
                    match rmp_serde::from_slice::<Secret>(&secret) {
                        Ok(secret) => secret,
                        Err(error) => {
                            error!("Could not parse secret in stored session: {error}");
                            return Err(LinuxSecretFieldError::Invalid.into());
                        }
                    }
                } else {
                    match serde_json::from_slice(&secret) {
                        Ok(secret) => secret,
                        Err(error) => {
                            error!("Could not parse secret in stored session: {error:?}");
                            return Err(LinuxSecretFieldError::Invalid.into());
                        }
                    }
                }
            }
            Err(error) => {
                error!("Could not get secret in stored session: {error}");
                return Err(LinuxSecretFieldError::Invalid.into());
            }
        };

        let session = Self {
            homeserver,
            user_id,
            device_id,
            id,
            secret,
        };

        if version < CURRENT_VERSION {
            Err(LinuxSecretError::OldVersion {
                version,
                session,
                attributes,
            })
        } else {
            Ok(session)
        }
    }

    /// Get the attributes from `self`.
    fn attributes(&self) -> HashMap<&'static str, String> {
        HashMap::from([
            (keys::HOMESERVER, self.homeserver.to_string()),
            (keys::USER, self.user_id.to_string()),
            (keys::DEVICE_ID, self.device_id.to_string()),
            (keys::ID, self.id.clone()),
            (keys::VERSION, CURRENT_VERSION.to_string()),
            (keys::PROFILE, PROFILE.to_string()),
            (keys::XDG_SCHEMA, APP_ID.to_owned()),
        ])
    }

    /// Migrate this session to the current version.
    async fn apply_migrations(&mut self, from_version: u8, attributes: HashMap<String, String>) {
        if from_version < 6 {
            // Version 5 changes the serialization of the secret from MessagePack to JSON.
            // Version 6 truncates sessions IDs, changing the path of the databases, and
            // removes the `db-path` attribute to replace it with the `id` attribute.
            // They both remove and add again the item in the secret backend so we merged
            // the migrations.
            info!("Migrating to version 6…");

            // Keep the old state of the session.
            let old_path = self.data_path();

            // Truncate the session ID.
            self.id.truncate(SESSION_ID_LENGTH);
            let new_path = self.data_path();

            let clone = self.clone();
            spawn_tokio!(async move {
                debug!(
                    "Renaming databases directory to: {}",
                    new_path.to_string_lossy()
                );
                if let Err(error) = fs::rename(old_path, new_path).await {
                    error!("Could not rename databases directory: {error}");
                }

                // Changing an attribute in an item creates a new item in oo7 because of a bug,
                // so we need to delete it and create a new one.
                if let Err(error) = delete_item_with_attributes(&attributes).await {
                    error!("Could not remove outdated session: {error}");
                }

                if let Err(error) = store_session_inner(clone).await {
                    error!("Could not store updated session: {error}");
                }
            })
            .await
            .unwrap();
        }
    }
}

/// Get the attribute with the given key in the given map.
fn get_attribute<'a>(
    attributes: &'a HashMap<String, String>,
    key: &'static str,
) -> Result<&'a String, LinuxSecretFieldError> {
    attributes
        .get(key)
        .ok_or(LinuxSecretFieldError::Missing(key))
}

/// Parse the attribute with the given key, using the given parsing function in
/// the given map.
fn parse_attribute<F, V, E>(
    attributes: &HashMap<String, String>,
    key: &'static str,
    parse: F,
) -> Result<V, LinuxSecretFieldError>
where
    F: FnOnce(&str) -> Result<V, E>,
    E: std::fmt::Display,
{
    let string = get_attribute(attributes, key)?;
    match parse(string) {
        Ok(value) => Ok(value),
        Err(error) => {
            error!("Could not parse {key} in stored session: {error}");
            Err(LinuxSecretFieldError::Invalid)
        }
    }
}

/// Any error that can happen when retrieving an attribute from the secret
/// backends on Linux.
#[derive(Debug, Error)]
pub enum LinuxSecretFieldError {
    /// An attribute is missing.
    ///
    /// This should only happen if for some reason we get an item from a
    /// different application.
    #[error("Could not find {0} in stored session")]
    Missing(&'static str),

    /// An invalid attribute was found.
    ///
    /// This should only happen if for some reason we get an item from a
    /// different application.
    #[error("Invalid field in stored session")]
    Invalid,
}

/// Remove the item with the given attributes from the secret backend.
async fn delete_item_with_attributes(
    attributes: &impl oo7::AsAttributes,
) -> Result<(), oo7::Error> {
    let keyring = Keyring::new().await?;
    keyring.delete(attributes).await?;

    Ok(())
}

/// Any error that can happen when interacting with the secret backends on
/// Linux.
#[derive(Debug, Error)]
// Complains about StoredSession in OldVersion, but we need it.
#[allow(clippy::large_enum_variant)]
pub enum LinuxSecretError {
    /// A session with an unsupported version was found.
    #[error("Session found with unsupported version {0}")]
    UnsupportedVersion(u8),

    /// A session with an old version was found.
    #[error("Session found with old version")]
    OldVersion {
        /// The version that was found.
        version: u8,
        /// The session that was found.
        session: StoredSession,
        /// The attributes of the secret item for the session.
        ///
        /// We use it to update the secret item because, if we use the `Item`
        /// directly, the Secret portal API returns errors saying that the file
        /// has changed after the first item was modified.
        attributes: HashMap<String, String>,
    },

    /// An error occurred while retrieving a field of the session.
    ///
    /// This should only happen if for some reason we get an item from a
    /// different application.
    #[error(transparent)]
    Field(#[from] LinuxSecretFieldError),

    /// An error occurred while interacting with the secret backend.
    #[error(transparent)]
    Oo7(#[from] oo7::Error),
}

impl From<oo7::Error> for SecretError {
    fn from(value: oo7::Error) -> Self {
        Self::Service(value.to_user_facing())
    }
}

impl UserFacingError for oo7::Error {
    fn to_user_facing(&self) -> String {
        match self {
            oo7::Error::File(error) => error.to_user_facing(),
            oo7::Error::DBus(error) => error.to_user_facing(),
        }
    }
}

impl UserFacingError for oo7::file::Error {
    fn to_user_facing(&self) -> String {
        use oo7::file::Error;

        match self {
            Error::FileHeaderMismatch(_) |
            Error::VersionMismatch(_) |
            Error::NoData |
            Error::MacError |
            Error::HashedAttributeMac(_) |
            Error::GVariantDeserialization(_) |
            Error::SaltSizeMismatch(_, _) |
            Error::ChecksumMismatch |
            Error::AlgorithmMismatch(_) |
            Error::IncorrectSecret |
            Error::Crypto(_) |
            Error::Utf8(_) => gettext(
                "The secret storage file is corrupted.",
            ),
            Error::NoParentDir(_) |
            Error::NoDataDir => gettext(
                "Could not access the secret storage file location.",
            ),
            Error::Io(_) => gettext(
                "An unexpected error occurred when accessing the secret storage file.",
            ),
            Error::TargetFileChanged(_) => gettext(
                "The secret storage file has been changed by another process.",
            ),
            Error::Portal(ashpd::Error::Portal(ashpd::PortalError::Cancelled(_))) => gettext(
                "The request to the Flatpak Secret Portal was cancelled. Make sure to accept any prompt asking to access it.",
            ),
            Error::Portal(ashpd::Error::PortalNotFound(_)) => gettext(
                "The Flatpak Secret Portal is not available. Make sure xdg-desktop-portal is installed, and it is at least at version 1.5.0.",
            ),
            Error::Portal(_) => gettext(
                "An unexpected error occurred when interacting with the D-Bus Secret Portal backend.",
            ),
            Error::WeakKey(_) => gettext(
                "The Flatpak Secret Portal provided a key that is too weak to be secure.",
            ),
            // Can only occur when using the `replace_item_index` or `delete_item_index` methods.
            Error::InvalidItemIndex(_) => unreachable!(),
        }
    }
}

impl UserFacingError for oo7::dbus::Error {
    fn to_user_facing(&self) -> String {
        use oo7::dbus::{Error, ServiceError};

        match self {
            Error::Deleted => gettext(
                "The item was deleted.",
            ),
            Error::Service(s) => match s {
                ServiceError::ZBus(_) => gettext(
                    "An unexpected error occurred when interacting with the D-Bus Secret Service.",
                ),
                ServiceError::IsLocked(_) => gettext(
                    "The collection or item is locked.",
                ),
                ServiceError::NoSession(_) => gettext(
                    "The D-Bus Secret Service session does not exist.",
                ),
                ServiceError::NoSuchObject(_) => gettext(
                    "The collection or item does not exist.",
                ),
            },
            Error::Dismissed => gettext(
                "The request to the D-Bus Secret Service was cancelled. Make sure to accept any prompt asking to access it.",
            ),
            Error::NotFound(_) => gettext(
                "Could not access the default collection. Make sure a keyring was created and set as default.",
            ),
            Error::ZBus(_) |
            Error::Utf8(_) |
            Error::Crypto(_) |
            Error::IO(_) => gettext(
                "An unexpected error occurred when interacting with the D-Bus Secret Service.",
            ),
        }
    }
}

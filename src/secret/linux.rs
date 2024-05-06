//! Linux API to store the data of a session, using the Secret Service or Secret
//! portal.

use std::{collections::HashMap, fs, path::PathBuf};

use gettextrs::gettext;
use oo7::{Item, Keyring};
use ruma::{OwnedDeviceId, UserId};
use thiserror::Error;
use tracing::{debug, error, info};
use url::Url;

use super::{Secret, SecretError, StoredSession, DATA_PATH};
use crate::{gettext_f, prelude::*, spawn_tokio, utils::matrix, APP_ID, PROFILE};

/// The current version of the stored session.
pub const CURRENT_VERSION: u8 = 5;
/// The attribute to identify the schema in the Secret Service.
const SCHEMA_ATTRIBUTE: &str = "xdg:schema";

/// Retrieves all sessions stored to the `SecretService`.
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
        .search_items(&HashMap::from([(SCHEMA_ATTRIBUTE, APP_ID)]))
        .await?;

    let mut sessions = Vec::with_capacity(items.len());

    for item in items {
        item.unlock().await?;

        match StoredSession::try_from_secret_item(item).await {
            Ok(session) => sessions.push(session),
            Err(LinuxSecretError::OldVersion {
                version,
                item,
                mut session,
            }) => {
                if version == 0 {
                    info!(
                        "Found old session for user {} with sled store, removing…",
                        session.user_id
                    );

                    // Try to log it out.
                    log_out_session(session.clone()).await;

                    session.delete().await;
                    continue;
                }

                info!(
                    "Found session {} for user {} with old version {}, applying migrations…",
                    session.id(),
                    session.user_id,
                    version,
                );
                session.apply_migrations(version, item).await;

                sessions.push(session);
            }
            Err(LinuxSecretError::WrongProfile) => {}
            Err(error) => {
                error!("Could not restore previous session: {error}");
            }
        }
    }

    Ok(sessions)
}

/// Write the given session to the `SecretService`, overwriting any previously
/// stored session with the same attributes.
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

/// Delete the given session from the secret service.
pub async fn delete_session(session: StoredSession) {
    spawn_tokio!(async move {
        if let Err(error) = session.delete_from_secret_service().await {
            error!("Could not delete session data from Secret Service: {error}");
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
                error!("Could not build client to log out session: {error}")
            }
        }
    })
    .await
    .unwrap();
}

impl StoredSession {
    /// Build self from a secret.
    async fn try_from_secret_item(item: Item) -> Result<Self, LinuxSecretError> {
        let attr = item.attributes().await?;

        let version = match attr.get("version") {
            Some(string) => match string.parse::<u8>() {
                Ok(version) => version,
                Err(error) => {
                    error!("Could not parse 'version' attribute in stored session: {error}");
                    return Err(LinuxSecretError::Invalid(gettext(
                        "Malformed version in stored session",
                    )));
                }
            },
            None => 0,
        };
        if version > CURRENT_VERSION {
            return Err(LinuxSecretError::UnsupportedVersion {
                version,
                item,
                attributes: attr,
            });
        }

        // TODO: Remove this and request profile in Keyring::search_items when we remove
        // migration.
        match attr.get("profile") {
            // Ignore the item if it's for another profile.
            Some(profile) if *profile != PROFILE.as_str() => {
                return Err(LinuxSecretError::WrongProfile)
            }
            // It's an error if the version is at least 2 but there is no profile.
            // Versions older than 2 will be migrated.
            None if version >= 2 => {
                return Err(LinuxSecretError::Invalid(gettext(
                    "Could not find profile in stored session",
                )));
            }
            // No issue for other cases.
            _ => {}
        };

        let homeserver = match attr.get("homeserver") {
            Some(string) => match Url::parse(string) {
                Ok(homeserver) => homeserver,
                Err(error) => {
                    error!("Could not parse 'homeserver' attribute in stored session: {error}");
                    return Err(LinuxSecretError::Invalid(gettext(
                        "Malformed homeserver in stored session",
                    )));
                }
            },
            None => {
                return Err(LinuxSecretError::Invalid(gettext(
                    "Could not find homeserver in stored session",
                )));
            }
        };
        let user_id = match attr.get("user") {
            Some(string) => match UserId::parse(string.as_str()) {
                Ok(user_id) => user_id,
                Err(error) => {
                    error!("Could not parse 'user' attribute in stored session: {error}");
                    return Err(LinuxSecretError::Invalid(gettext(
                        "Malformed user ID in stored session",
                    )));
                }
            },
            None => {
                return Err(LinuxSecretError::Invalid(gettext(
                    "Could not find user ID in stored session",
                )));
            }
        };
        let device_id = match attr.get("device-id") {
            Some(string) => OwnedDeviceId::from(string.as_str()),
            None => {
                return Err(LinuxSecretError::Invalid(gettext(
                    "Could not find device ID in stored session",
                )));
            }
        };
        let path = match attr.get("db-path") {
            Some(string) => PathBuf::from(string),
            None => {
                return Err(LinuxSecretError::Invalid(gettext(
                    "Could not find database path in stored session",
                )));
            }
        };
        let secret = match item.secret().await {
            Ok(secret) => {
                if version <= 4 {
                    match rmp_serde::from_slice::<Secret>(&secret) {
                        Ok(secret) => secret,
                        Err(error) => {
                            error!("Could not parse secret in stored session: {error}");
                            return Err(LinuxSecretError::Invalid(gettext(
                                "Malformed secret in stored session",
                            )));
                        }
                    }
                } else {
                    match serde_json::from_slice(&secret) {
                        Ok(secret) => secret,
                        Err(error) => {
                            error!("Could not parse secret in stored session: {error:?}");
                            return Err(LinuxSecretError::Invalid(gettext(
                                "Malformed secret in stored session",
                            )));
                        }
                    }
                }
            }
            Err(error) => {
                error!("Could not get secret in stored session: {error}");
                return Err(LinuxSecretError::Invalid(gettext(
                    "Could not get secret in stored session",
                )));
            }
        };

        let session = Self {
            homeserver,
            user_id,
            device_id,
            path,
            secret,
        };

        if version < CURRENT_VERSION {
            Err(LinuxSecretError::OldVersion {
                version,
                item,
                session,
            })
        } else {
            Ok(session)
        }
    }

    /// Get the attributes from `self`.
    fn attributes(&self) -> HashMap<&str, String> {
        HashMap::from([
            ("homeserver", self.homeserver.to_string()),
            ("user", self.user_id.to_string()),
            ("device-id", self.device_id.to_string()),
            ("db-path", self.path.to_str().unwrap().to_owned()),
            ("version", CURRENT_VERSION.to_string()),
            ("profile", PROFILE.to_string()),
            (SCHEMA_ATTRIBUTE, APP_ID.to_owned()),
        ])
    }

    /// Remove this session from the `SecretService`
    async fn delete_from_secret_service(&self) -> Result<(), SecretError> {
        let keyring = Keyring::new().await?;
        keyring.delete(&self.attributes()).await?;

        Ok(())
    }

    /// Migrate this session to the current version.
    async fn apply_migrations(&mut self, current_version: u8, item: Item) {
        if current_version < 4 {
            info!("Migrating to version 4…");

            let target_path = DATA_PATH.join(self.id());

            if self.path != target_path {
                debug!("Moving database to: {}", target_path.to_string_lossy());

                if let Err(error) = fs::create_dir_all(&target_path) {
                    error!("Could not create new directory: {error}");
                }

                if let Err(error) = fs::rename(&self.path, &target_path) {
                    error!("Could not move database: {error}");
                }

                self.path = target_path;
            }
        }

        info!("Migrating to version 5…");

        let clone = self.clone();
        spawn_tokio!(async move {
            if let Err(error) = item.delete().await {
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

/// Any error that can happen when interacting with the Secret Service on Linux.
#[derive(Debug, Error)]
pub enum LinuxSecretError {
    /// A session with an unsupported version was found.
    #[error("Session found with unsupported version {version}")]
    UnsupportedVersion {
        version: u8,
        item: Item,
        attributes: HashMap<String, String>,
    },

    /// A session with an old version was found.
    #[error("Session found with old version")]
    OldVersion {
        version: u8,
        item: Item,
        session: StoredSession,
    },

    /// An invalid session was found.
    ///
    /// This should only happen if for some reason we get an item from a
    /// different application.
    #[error("Invalid session: {0}")]
    Invalid(String),

    /// An error occurred interacting with the secret service.
    #[error(transparent)]
    Oo7(#[from] oo7::Error),

    /// Trying to restore a session with the wrong profile.
    #[error("Session found for wrong profile")]
    WrongProfile,
}

impl From<oo7::Error> for SecretError {
    fn from(value: oo7::Error) -> Self {
        Self::Service(value.to_user_facing())
    }
}

impl UserFacingError for oo7::Error {
    fn to_user_facing(&self) -> String {
        match self {
            oo7::Error::Portal(error) => error.to_user_facing(),
            oo7::Error::DBus(error) => error.to_user_facing(),
        }
    }
}

impl UserFacingError for oo7::portal::Error {
    fn to_user_facing(&self) -> String {
        use oo7::portal::Error;

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
            Error::Utf8(_) => gettext(
                "The secret storage file is corrupted.",
            ),
            Error::NoParentDir(_) |
            Error::NoDataDir => gettext(
                "Could not access the secret storage file location.",
            ),
            Error::Io(_) => gettext(
                "An unknown error occurred when accessing the secret storage file.",
            ),
            Error::TargetFileChanged(_) => gettext(
                "The secret storage file has been changed by another process.",
            ),
            Error::PortalBus(_) => gettext(
                "An unknown error occurred when interacting with the D-Bus Secret Portal backend.",
            ),
            Error::CancelledPortalRequest => gettext(
                "The request to the Flatpak Secret Portal was cancelled. Make sure to accept any prompt asking to access it.",
            ),
            Error::PortalNotAvailable => gettext(
                "The Flatpak Secret Portal is not available. Make sure xdg-desktop-portal is installed, and it is at least at version 1.5.0.",
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
                    "An unknown error occurred when interacting with the D-Bus Secret Service.",
                ),
                ServiceError::IsLocked => gettext(
                    "The collection or item is locked.",
                ),
                ServiceError::NoSession => gettext(
                    "The D-Bus Secret Service session does not exist.",
                ),
                ServiceError::NoSuchObject => gettext(
                    "The collection or item does not exist.",
                ),
            },
            Error::Dismissed => gettext(
                "The request to the D-Bus Secret Service was cancelled. Make sure to accept any prompt asking to access it.",
            ),
            Error::NotFound(_) => gettext(
                "Could not access the default collection. Make sure a keyring was created and set as default.",
            ),
            Error::Zbus(_) |
            Error::IO(_) => gettext(
                "An unknown error occurred when interacting with the D-Bus Secret Service.",
            ),
        }
    }
}

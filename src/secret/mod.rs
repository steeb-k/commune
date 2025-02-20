//! API to store the data of a session in a secret store on the system.

use std::{fmt, path::PathBuf};

use gtk::glib;
use matrix_sdk::{authentication::matrix::MatrixSessionTokens, AuthSession, SessionMeta};
use rand::{
    distr::{Alphanumeric, SampleString},
    rng,
};
use ruma::{OwnedDeviceId, OwnedUserId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs;
use tracing::{debug, error};
use url::Url;
use zeroize::Zeroizing;

mod file;
#[cfg(target_os = "linux")]
mod linux;

use self::file::SecretFile;
use crate::{
    prelude::*,
    spawn_tokio,
    utils::{data_dir_path, matrix::ClientSetupError, DataType},
};

/// The length of a session ID, in chars or bytes as the string is ASCII.
pub(crate) const SESSION_ID_LENGTH: usize = 8;
/// The length of a passphrase, in chars or bytes as the string is ASCII.
pub(crate) const PASSPHRASE_LENGTH: usize = 30;

cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        /// The secret API.
        pub(crate) type Secret = linux::LinuxSecret;
    } else {
        /// The secret API.
        pub(crate) type Secret = unimplemented::UnimplementedSecret;
    }
}

/// Trait implemented by secret backends.
pub(crate) trait SecretExt {
    /// Retrieves all sessions stored in the secret backend.
    async fn restore_sessions() -> Result<Vec<StoredSession>, SecretError>;

    /// Store the given session into the secret backend, overwriting any
    /// previously stored session with the same attributes.
    async fn store_session(session: StoredSession) -> Result<(), SecretError>;

    /// Delete the given session from the secret backend.
    async fn delete_session(session: &StoredSession);
}

/// The fallback `Secret` API, to use on platforms where it is unimplemented.
#[cfg(not(target_os = "linux"))]
mod unimplemented {
    #[derive(Debug)]
    pub(crate) struct UnimplementedSecret;

    impl SecretExt for UnimplementedSecret {
        async fn restore_sessions() -> Result<Vec<StoredSession>, SecretError> {
            unimplemented!()
        }

        async fn store_session(session: StoredSession) -> Result<(), SecretError> {
            unimplemented!()
        }

        async fn delete_session(session: &StoredSession) {
            unimplemented!()
        }
    }
}

/// Any error that can happen when interacting with the secret service.
#[derive(Debug, Error)]
pub(crate) enum SecretError {
    /// An error occurred interacting with the secret service.
    #[error("Service error: {0}")]
    Service(String),
}

impl UserFacingError for SecretError {
    fn to_user_facing(&self) -> String {
        match self {
            SecretError::Service(error) => error.clone(),
        }
    }
}

/// A session, as stored in the secret service.
#[derive(Clone, glib::Boxed)]
#[boxed_type(name = "StoredSession")]
pub struct StoredSession {
    /// The URL of the homeserver where the account lives.
    pub homeserver: Url,
    /// The unique identifier of the user.
    pub user_id: OwnedUserId,
    /// The unique identifier of the session on the homeserver.
    pub device_id: OwnedDeviceId,
    /// The unique local identifier of the session.
    ///
    /// This is the name of the directories where the session data lives.
    pub id: String,
    /// The passphrase used to encrypt the local databases.
    pub passphrase: Zeroizing<String>,
}

impl fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredSession")
            .field("homeserver", &self.homeserver)
            .field("user_id", &self.user_id)
            .field("device_id", &self.device_id)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl StoredSession {
    /// Construct a `StoredSession` from the given SDK user session data.
    ///
    /// Returns an error if we failed to generate a unique session ID for the
    /// new session.
    pub(crate) async fn new(
        homeserver: Url,
        meta: SessionMeta,
        tokens: SessionTokens,
    ) -> Result<Self, ClientSetupError> {
        let SessionMeta { user_id, device_id } = meta;

        // Generate a unique random session ID.
        let mut id = None;
        let data_path = data_dir_path(DataType::Persistent);

        // Try 10 times, so we do not have an infinite loop.
        for _ in 0..10 {
            let generated = Alphanumeric.sample_string(&mut rng(), SESSION_ID_LENGTH);

            // Make sure that the ID is not already in use.
            let path = data_path.join(&generated);
            if !path.exists() {
                id = Some(generated);
                break;
            }
        }

        let Some(id) = id else {
            return Err(ClientSetupError::NoSessionId);
        };

        let passphrase = Alphanumeric.sample_string(&mut rng(), PASSPHRASE_LENGTH);

        let session = Self {
            homeserver,
            user_id,
            device_id,
            id,
            passphrase: passphrase.into(),
        };

        session.store_tokens(tokens).await;

        Ok(session)
    }

    /// The path where the persistent data of this session lives.
    pub(crate) fn data_path(&self) -> PathBuf {
        let mut path = data_dir_path(DataType::Persistent);
        path.push(&self.id);
        path
    }

    /// The path where the cached data of this session lives.
    pub(crate) fn cache_path(&self) -> PathBuf {
        let mut path = data_dir_path(DataType::Cache);
        path.push(&self.id);
        path
    }

    /// Delete this session from the system.
    pub(crate) async fn delete(self) {
        debug!(
            "Removing stored session {} for Matrix user {}…",
            self.id, self.user_id,
        );

        Secret::delete_session(&self).await;

        spawn_tokio!(async move {
            if let Err(error) = fs::remove_dir_all(self.data_path()).await {
                error!("Could not remove session database: {error}");
            }
            if let Err(error) = fs::remove_dir_all(self.cache_path()).await {
                error!("Could not remove session cache: {error}");
            }
        })
        .await
        .expect("task was not aborted");
    }

    /// The path to the files containing the session tokens.
    fn tokens_path(&self) -> PathBuf {
        let mut path = self.data_path();
        path.push("tokens");
        path
    }

    /// Load the tokens of this session.
    pub(crate) async fn load_tokens(&self) -> Option<SessionTokens> {
        let tokens_path = self.tokens_path();
        let passphrase = self.passphrase.clone();

        let handle = spawn_tokio!(async move { SecretFile::read(&tokens_path, &passphrase).await });

        match handle.await.expect("task was not aborted") {
            Ok(tokens) => Some(tokens),
            Err(error) => {
                error!("Could not load session tokens: {error}");
                None
            }
        }
    }

    /// Store the tokens of this session.
    pub(crate) async fn store_tokens(&self, tokens: SessionTokens) {
        let tokens_path = self.tokens_path();
        let passphrase = self.passphrase.clone();

        let handle =
            spawn_tokio!(
                async move { SecretFile::write(&tokens_path, &passphrase, &tokens).await }
            );

        if let Err(error) = handle.await.expect("task was not aborted") {
            error!("Could not store session tokens: {error}");
        }
    }
}

/// The tokens of a user session.
#[derive(Serialize, Deserialize)]
pub(crate) struct SessionTokens {
    pub(crate) access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) refresh_token: Option<String>,
}

impl From<MatrixSessionTokens> for SessionTokens {
    fn from(value: MatrixSessionTokens) -> Self {
        let MatrixSessionTokens {
            access_token,
            refresh_token,
        } = value;
        SessionTokens {
            access_token,
            refresh_token,
        }
    }
}

impl From<SessionTokens> for MatrixSessionTokens {
    fn from(value: SessionTokens) -> Self {
        let SessionTokens {
            access_token,
            refresh_token,
        } = value;
        MatrixSessionTokens {
            access_token,
            refresh_token,
        }
    }
}

impl From<AuthSession> for SessionTokens {
    fn from(value: AuthSession) -> Self {
        Self {
            access_token: value.access_token().to_owned(),
            refresh_token: value.get_refresh_token().map(ToOwned::to_owned),
        }
    }
}

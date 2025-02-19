//! API to store the data of a session in a secret store on the system.

use std::{fmt, path::PathBuf};

use gtk::glib;
use matrix_sdk::{
    authentication::matrix::{MatrixSession, MatrixSessionTokens},
    SessionMeta,
};
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

#[cfg(target_os = "linux")]
mod linux;

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
    /// The secrets of the session.
    pub secret: SecretData,
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
    /// Construct a `StoredSession` from the given login data.
    ///
    /// Returns an error if we failed to generate a unique session ID for the
    /// new session.
    pub(crate) fn with_login_data(
        homeserver: Url,
        data: MatrixSession,
    ) -> Result<Self, ClientSetupError> {
        let MatrixSession {
            meta: SessionMeta { user_id, device_id },
            tokens: MatrixSessionTokens { access_token, .. },
        } = data;

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

        let secret = SecretData {
            access_token,
            passphrase,
        };

        Ok(Self {
            homeserver,
            user_id,
            device_id,
            id,
            secret,
        })
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
}

/// Secret data that can be stored in the secret backend.
#[derive(Clone, Deserialize, Serialize)]
pub struct SecretData {
    /// The access token to provide to the homeserver for authentication.
    pub access_token: String,
    /// The passphrase used to encrypt the local databases.
    pub passphrase: String,
}

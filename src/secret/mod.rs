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
#[cfg(not(target_os = "linux"))]
mod unimplemented;

#[cfg(target_os = "linux")]
use self::linux::delete_session;
#[cfg(target_os = "linux")]
pub use self::linux::{restore_sessions, store_session};
#[cfg(not(target_os = "linux"))]
use self::unimplemented::delete_session;
#[cfg(not(target_os = "linux"))]
pub use self::unimplemented::{restore_sessions, store_session};
use crate::{
    prelude::*,
    spawn_tokio,
    utils::{data_dir_path, matrix::ClientSetupError, DataType},
};

/// The length of a session ID, in chars or bytes as the string is ASCII.
pub const SESSION_ID_LENGTH: usize = 8;
/// The length of a passphrase, in chars or bytes as the string is ASCII.
pub const PASSPHRASE_LENGTH: usize = 30;

/// Any error that can happen when interacting with the secret service.
#[derive(Debug, Error)]
pub enum SecretError {
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
    pub secret: Secret,
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
    pub fn with_login_data(homeserver: Url, data: MatrixSession) -> Result<Self, ClientSetupError> {
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

        let secret = Secret {
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
    pub fn data_path(&self) -> PathBuf {
        let mut path = data_dir_path(DataType::Persistent);
        path.push(&self.id);
        path
    }

    /// The path where the cached data of this session lives.
    pub fn cache_path(&self) -> PathBuf {
        let mut path = data_dir_path(DataType::Cache);
        path.push(&self.id);
        path
    }

    /// Delete this session from the system.
    pub async fn delete(self) {
        debug!(
            "Removing stored session {} for Matrix user {}…",
            self.id, self.user_id,
        );

        delete_session(&self).await;

        spawn_tokio!(async move {
            if let Err(error) = fs::remove_dir_all(self.data_path()).await {
                error!("Could not remove session database: {error}");
            }
            if let Err(error) = fs::remove_dir_all(self.cache_path()).await {
                error!("Could not remove session cache: {error}");
            }
        })
        .await
        .unwrap();
    }
}

/// A `Secret` that can be stored in the `SecretService`.
#[derive(Clone, Deserialize, Serialize)]
pub struct Secret {
    /// The access token to provide to the homeserver for authentication.
    pub access_token: String,
    /// The passphrase used to encrypt the local databases.
    pub passphrase: String,
}

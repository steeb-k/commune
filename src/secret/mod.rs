//! API to store the data of a session in a secret store on the system.

use std::{ffi::OsStr, fmt, path::PathBuf};

use gtk::glib;
use matrix_sdk::{
    matrix_auth::{MatrixSession, MatrixSessionTokens},
    SessionMeta,
};
use once_cell::sync::Lazy;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
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
use crate::{application::AppProfile, prelude::*, spawn_tokio, PROFILE};

/// The path where the database should be stored.
static DATA_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let dir_name = match PROFILE {
        AppProfile::Stable => "fractal".to_owned(),
        _ => format!("fractal-{PROFILE}"),
    };

    glib::user_data_dir().join(dir_name)
});

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
    pub homeserver: Url,
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub path: PathBuf,
    pub secret: Secret,
}

impl fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredSession")
            .field("homeserver", &self.homeserver)
            .field("user_id", &self.user_id)
            .field("device_id", &self.device_id)
            .field("path", &self.path)
            .finish()
    }
}

impl StoredSession {
    /// Construct a `StoredSession` from the given login data.
    pub fn with_login_data(homeserver: Url, data: MatrixSession) -> Self {
        let MatrixSession {
            meta: SessionMeta { user_id, device_id },
            tokens: MatrixSessionTokens { access_token, .. },
        } = data;

        let path = DATA_PATH.join(glib::uuid_string_random().as_str());

        let passphrase = thread_rng()
            .sample_iter(Alphanumeric)
            .take(30)
            .map(char::from)
            .collect();

        let secret = Secret {
            access_token,
            passphrase,
        };

        Self {
            homeserver,
            user_id,
            device_id,
            path,
            secret,
        }
    }

    /// Split this `StoredSession` into parts.
    pub fn into_parts(self) -> (Url, PathBuf, String, MatrixSession) {
        let Self {
            homeserver,
            user_id,
            device_id,
            path,
            secret: Secret {
                access_token,
                passphrase,
            },
        } = self;

        let data = MatrixSession {
            meta: SessionMeta { user_id, device_id },
            tokens: MatrixSessionTokens {
                access_token,
                refresh_token: None,
            },
        };

        (homeserver, path, passphrase, data)
    }

    /// The unique ID for this `StoredSession`.
    ///
    /// This is the name of the folder where the DB is stored.
    pub fn id(&self) -> &str {
        self.path
            .iter()
            .next_back()
            .and_then(OsStr::to_str)
            .unwrap()
    }

    /// Delete this session from the system.
    pub async fn delete(self) {
        debug!(
            "Removing stored session {} for Matrix user {}…",
            self.id(),
            self.user_id,
        );

        delete_session(self.clone()).await;

        spawn_tokio!(async move {
            if let Err(error) = fs::remove_dir_all(self.path).await {
                error!("Failed to remove session database: {error}");
            }
        })
        .await
        .unwrap();
    }
}

/// A `Secret` that can be stored in the `SecretService`.
#[derive(Clone, Deserialize, Serialize)]
pub struct Secret {
    pub access_token: String,
    pub passphrase: String,
}

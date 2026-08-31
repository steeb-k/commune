//! API to store the data of a session in a secret store on the system.

use std::{fmt, path::PathBuf};

use matrix_sdk::{Client, SessionMeta, SessionTokens, authentication::oauth::ClientId};
use rand::{
    distr::{Alphanumeric, SampleString},
    rng,
};
use ruma::{OwnedDeviceId, OwnedUserId};
use thiserror::Error;
use tokio::fs;
use tracing::{debug, error};
use url::Url;
use zeroize::Zeroizing;

#[cfg(target_os = "android")]
mod android;
mod file;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use self::file::SecretFile;
use crate::{UserFacingError, matrix::ClientSetupError, paths::DataType, spawn_tokio};

/// The length of a session ID, in chars or bytes as the string is ASCII.
pub const SESSION_ID_LENGTH: usize = 8;
/// The length of a passphrase, in chars or bytes as the string is ASCII.
pub(crate) const PASSPHRASE_LENGTH: usize = 30;

cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        /// The secret API.
        pub type Secret = linux::LinuxSecret;
    } else if #[cfg(target_os = "macos")] {
        /// The secret API.
        pub type Secret = macos::MacosSecret;
    } else if #[cfg(target_os = "android")] {
        /// The secret API.
        pub type Secret = android::AndroidSecret;
    } else if #[cfg(target_os = "windows")] {
        /// The secret API.
        pub type Secret = windows::WindowsSecret;
    } else {
        /// The secret API.
        pub type Secret = unimplemented::UnimplementedSecret;
    }
}

/// Trait implemented by secret backends.
///
/// Only this crate implements it — the allow is the lint's own suggestion
/// for that arrangement.
#[allow(async_fn_in_trait)]
pub trait SecretExt {
    /// Retrieves all sessions stored in the secret backend.
    async fn restore_sessions() -> Result<Vec<StoredSession>, SecretError>;

    /// Store the given session into the secret backend, overwriting any
    /// previously stored session with the same attributes.
    async fn store_session(session: StoredSession) -> Result<(), SecretError>;

    /// Delete the given session from the secret backend.
    async fn delete_session(session: &StoredSession);
}

/// The fallback `Secret` API, to use on platforms where it is unimplemented.
#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    target_os = "windows"
)))]
mod unimplemented {
    use super::*;

    #[derive(Debug)]
    pub struct UnimplementedSecret;

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
pub enum SecretError {
    /// An error occurred interacting with the secret service.
    ///
    /// The message comes from the platform and is not something this crate
    /// can say anything better about, so it is carried as it stands.
    #[error("Service error: {0}")]
    Service(String),
    /// The Linux keyring or Secret Portal refused, in one of the ways worth
    /// telling them apart.
    #[cfg(target_os = "linux")]
    #[error("Keyring error: {0}")]
    Keyring(#[from] KeyringError),
}

impl UserFacingError for SecretError {
    fn to_user_facing(&self) -> String {
        match self {
            SecretError::Service(error) => error.clone(),
            #[cfg(target_os = "linux")]
            SecretError::Keyring(error) => error.to_user_facing(),
        }
    }
}

/// What went wrong with the Linux secret backend, as one of the cases the
/// user can be told apart.
///
/// `oo7`'s own errors carry far more detail than a person can act on, and the
/// application has always collapsed them into these fifteen sentences. They
/// stay a value rather than a string because the sentence belongs to whatever
/// is drawing it: the GTK application runs each of these through `gettext`,
/// and the core's own rendering below is the English fallback for an embedder
/// with no translations. Turning them into a string here is what the first
/// transcription did, and it is how the translations were lost.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum KeyringError {
    /// The keyring file cannot be read as a keyring file.
    #[error("The secret storage file is corrupted")]
    CorruptedFile,
    /// The directory the keyring file should live in is not reachable.
    #[error("Could not access the secret storage file location")]
    NoFileLocation,
    /// Reading or writing the keyring file failed.
    #[error("Could not access the secret storage file")]
    FileIo,
    /// Another process wrote the keyring file underneath us.
    #[error("The secret storage file has been changed by another process")]
    FileChanged,
    /// The user dismissed the Flatpak Secret Portal's prompt.
    #[error("The request to the Flatpak Secret Portal was cancelled")]
    PortalCancelled,
    /// There is no Secret Portal on the bus.
    #[error("The Flatpak Secret Portal is not available")]
    PortalNotAvailable,
    /// The Secret Portal failed in some other way.
    #[error("The Flatpak Secret Portal failed")]
    Portal,
    /// The Secret Portal handed back a key that is not strong enough.
    #[error("The Flatpak Secret Portal provided a key that is too weak to be secure")]
    PortalWeakKey,
    /// The collection or item is locked.
    #[error("The collection or item is locked")]
    Locked,
    /// The item was deleted while we were using it.
    #[error("The item was deleted")]
    ItemDeleted,
    /// The D-Bus Secret Service failed in some other way.
    #[error("The D-Bus Secret Service failed")]
    Service,
    /// The Secret Service session does not exist.
    #[error("The D-Bus Secret Service session does not exist")]
    NoServiceSession,
    /// The collection or item does not exist.
    #[error("The collection or item does not exist")]
    NoSuchObject,
    /// The user dismissed the Secret Service's prompt.
    #[error("The request to the D-Bus Secret Service was cancelled")]
    ServiceDismissed,
    /// There is no default collection to store the session in.
    #[error("Could not access the default collection")]
    NoDefaultCollection,
}

#[cfg(target_os = "linux")]
impl UserFacingError for KeyringError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::CorruptedFile => String::from("The secret storage file is corrupted."),
            Self::NoFileLocation => {
                String::from("Could not access the secret storage file location.")
            }
            Self::FileIo => {
                String::from("An unexpected error occurred when accessing the secret storage file.")
            }
            Self::FileChanged => {
                String::from("The secret storage file has been changed by another process.")
            }
            Self::PortalCancelled => String::from(
                "The request to the Flatpak Secret Portal was cancelled. Make sure to accept any prompt asking to access it.",
            ),
            Self::PortalNotAvailable => String::from(
                "The Flatpak Secret Portal is not available. Make sure xdg-desktop-portal is installed, and it is at least at version 1.5.0.",
            ),
            Self::Portal => String::from(
                "An unexpected error occurred when interacting with the D-Bus Secret Portal backend.",
            ),
            Self::PortalWeakKey => String::from(
                "The Flatpak Secret Portal provided a key that is too weak to be secure.",
            ),
            Self::Locked => String::from("The collection or item is locked."),
            Self::ItemDeleted => String::from("The item was deleted."),
            Self::Service => String::from(
                "An unexpected error occurred when interacting with the D-Bus Secret Service.",
            ),
            Self::NoServiceSession => {
                String::from("The D-Bus Secret Service session does not exist.")
            }
            Self::NoSuchObject => String::from("The collection or item does not exist."),
            Self::ServiceDismissed => String::from(
                "The request to the D-Bus Secret Service was cancelled. Make sure to accept any prompt asking to access it.",
            ),
            Self::NoDefaultCollection => String::from(
                "Could not access the default collection. Make sure a keyring was created and set as default.",
            ),
        }
    }
}

/// A session, as stored in the secret service.
#[derive(Clone)]
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
    /// The unique identifier of the client with the homeserver.
    pub client_id: Option<ClientId>,
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
    /// Construct a `StoredSession` from the session of the given Matrix client.
    ///
    /// Returns an error if we failed to generate a unique session ID for the
    /// new session.
    pub async fn new(client: &Client) -> Result<Self, ClientSetupError> {
        // Generate a unique random session ID.
        let mut id = None;
        let data_path = DataType::Persistent.dir_path();

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

        let homeserver = client.homeserver();
        let SessionMeta { user_id, device_id } = client
            .session_meta()
            .expect("logged-in client should have session meta")
            .clone();
        let tokens = client
            .session_tokens()
            .expect("logged-in client should have session tokens")
            .clone();
        let client_id = client.oauth().client_id().cloned();

        let passphrase = Alphanumeric.sample_string(&mut rng(), PASSPHRASE_LENGTH);

        let session = Self {
            homeserver,
            user_id,
            device_id,
            id,
            client_id,
            passphrase: passphrase.into(),
        };

        session.create_data_dir().await;
        session.store_tokens(tokens).await;

        Ok(session)
    }

    /// The path where the persistent data of this session lives.
    #[must_use]
    pub fn data_path(&self) -> PathBuf {
        let mut path = DataType::Persistent.dir_path();
        path.push(&self.id);
        path
    }

    /// Create the directory where the persistent data of this session will
    /// live.
    async fn create_data_dir(&self) {
        let data_path = self.data_path();

        spawn_tokio!(async move {
            if let Err(error) = fs::create_dir_all(data_path).await {
                error!("Could not create session data directory: {error}");
            }
        })
        .await
        .expect("task was not aborted");
    }

    /// The path where the cached data of this session lives.
    #[must_use]
    pub fn cache_path(&self) -> PathBuf {
        let mut path = DataType::Cache.dir_path();
        path.push(&self.id);
        path
    }

    /// Delete this session from the system.
    pub async fn delete(self) {
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
    pub async fn load_tokens(&self) -> Option<SessionTokens> {
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
    pub async fn store_tokens(&self, tokens: SessionTokens) {
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

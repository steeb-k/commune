//! Secret backend using a file in the application's private storage.
//!
//! # This is a placeholder, not a secure store
//!
//! Android's real answer for this is the Keystore, reached through JNI, and
//! that is what a build handed to anyone must use. This backend exists so that
//! the port can log in and be exercised before the JNI plumbing exists; it
//! writes the session — including the passphrase that encrypts the local
//! databases — as plain JSON.
//!
//! What it does rely on is the application sandbox: everything under
//! `Context.getFilesDir()` is owned by this application's UID and is not
//! readable by other applications. On a device that has not been rooted, and
//! with `android:allowBackup` disabled, that is a meaningful boundary. It is
//! not equivalent to hardware-backed key storage, it does not survive a rooted
//! device, and it is not what the Keystore would give us.
//!
//! Replacing this is tracked as the first task of S5 in `doc/android-plan.md`.
//! The `SecretExt` surface is the seam: only this file changes.
//!
//! # Layout
//!
//! One file per session, `secrets.d/<session id>.json`, next to the per-session
//! data directories rather than inside them, so that enumerating sessions does
//! not mean walking every session's database. The payload is the same
//! `version: 1` document the macOS backend stores in the Keychain. It is
//! duplicated rather than shared because the two backends should be free to
//! diverge.

use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use matrix_sdk::authentication::oauth::ClientId;
use serde::{Deserialize, Serialize};
use tracing::{error, warn};
use zeroize::{Zeroize, Zeroizing};

use super::{SecretError, SecretExt, StoredSession};
use crate::{spawn_tokio, utils::DataType};

/// The version of the payload format that this version of the application
/// writes.
const CURRENT_VERSION: u8 = 1;

/// The permissions of the directory holding the session files: only this
/// application's UID may traverse it.
const DIR_MODE: u32 = 0o700;

/// The permissions of a session file.
const FILE_MODE: u32 = 0o600;

pub(crate) struct AndroidSecret;

impl SecretExt for AndroidSecret {
    async fn restore_sessions() -> Result<Vec<StoredSession>, SecretError> {
        let handle = spawn_tokio!(async move { restore_sessions_inner() });

        match handle.await.expect("task was not aborted") {
            Ok(sessions) => Ok(sessions),
            Err(error) => {
                error!("Could not restore previous sessions: secret error: {error}");
                Err(error)
            }
        }
    }

    async fn store_session(session: StoredSession) -> Result<(), SecretError> {
        let handle = spawn_tokio!(async move { store_session_inner(&session) });

        match handle.await.expect("task was not aborted") {
            Ok(()) => Ok(()),
            Err(error) => {
                error!("Could not store session: secret error: {error}");
                Err(error)
            }
        }
    }

    async fn delete_session(session: &StoredSession) {
        let path = session_path(&session.id);

        spawn_tokio!(async move {
            match fs::remove_file(&path) {
                Ok(()) => {}
                // A session that was never stored is already in the state the
                // caller asked for.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => error!("Could not delete stored session: {error}"),
            }
        })
        .await
        .expect("task was not aborted");
    }
}

/// The payload stored for a session.
///
/// This mirrors the macOS backend's document so that the two can be compared
/// when one of them misbehaves.
#[derive(Debug, Serialize, Deserialize)]
struct FileSecret {
    /// The version of this payload, so that a future format is recognised
    /// rather than misread.
    version: u8,
    /// The URL of the homeserver where the account lives.
    homeserver: String,
    /// The unique identifier of the user.
    user_id: String,
    /// The unique identifier of the session on the homeserver.
    device_id: String,
    /// The unique local identifier of the session.
    id: String,
    /// The unique identifier of the client with the homeserver.
    client_id: Option<String>,
    /// The passphrase used to encrypt the local databases.
    passphrase: String,
}

/// The directory holding one file per stored session.
fn secrets_dir() -> PathBuf {
    // The session IDs that name the sibling directories are 8 alphanumeric
    // characters, so a name carrying a dot cannot collide with one.
    DataType::Persistent.dir_path().join("secrets.d")
}

/// The path of the file holding the session with the given ID.
fn session_path(id: &str) -> PathBuf {
    secrets_dir().join(format!("{id}.json"))
}

/// Retrieve every session stored on disk.
fn restore_sessions_inner() -> Result<Vec<StoredSession>, SecretError> {
    let dir = secrets_dir();

    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        // No directory means no sessions have ever been stored, which is not an
        // error: it is what a first launch looks like.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(SecretError::Service(error.to_string())),
    };

    let mut sessions = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warn!("Ignoring unreadable entry in the session directory: {error}");
                continue;
            }
        };

        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }

        match session_from_file(&path) {
            Ok(session) => sessions.push(session),
            // One unreadable file must not stop the others from being restored,
            // or a single bad file locks the user out of every account.
            Err(error) => warn!("Ignoring stored session {}: {error}", path.display()),
        }
    }

    Ok(sessions)
}

/// Read a [`StoredSession`] from the file at the given path.
fn session_from_file(path: &Path) -> Result<StoredSession, String> {
    let data = Zeroizing::new(
        fs::read(path).map_err(|error| format!("could not read session data: {error}"))?,
    );

    let secret: FileSecret =
        serde_json::from_slice(&data).map_err(|error| format!("invalid session data: {error}"))?;

    if secret.version > CURRENT_VERSION {
        return Err(format!(
            "session was stored by a newer version of the application (payload version {}, we \
             support up to {CURRENT_VERSION})",
            secret.version
        ));
    }

    Ok(StoredSession {
        homeserver: secret
            .homeserver
            .parse()
            .map_err(|error| format!("invalid homeserver URL: {error}"))?,
        user_id: secret
            .user_id
            .try_into()
            .map_err(|error| format!("invalid user ID: {error}"))?,
        device_id: secret.device_id.into(),
        id: secret.id,
        client_id: secret.client_id.map(ClientId::new),
        passphrase: Zeroizing::new(secret.passphrase),
    })
}

/// Store the given session, replacing any file that already holds the same
/// session ID.
fn store_session_inner(session: &StoredSession) -> Result<(), SecretError> {
    let dir = secrets_dir();
    fs::create_dir_all(&dir).map_err(|error| SecretError::Service(error.to_string()))?;
    // `create_dir_all` respects the umask, so the mode is set explicitly rather
    // than assumed.
    fs::set_permissions(&dir, fs::Permissions::from_mode(DIR_MODE))
        .map_err(|error| SecretError::Service(error.to_string()))?;

    let secret = FileSecret {
        version: CURRENT_VERSION,
        homeserver: session.homeserver.to_string(),
        user_id: session.user_id.to_string(),
        device_id: session.device_id.to_string(),
        id: session.id.clone(),
        client_id: session.client_id.as_ref().map(|id| id.as_str().to_owned()),
        passphrase: session.passphrase.as_str().to_owned(),
    };

    let mut payload =
        serde_json::to_vec(&secret).map_err(|error| SecretError::Service(error.to_string()))?;
    let result = write_session_file(&dir, &session.id, &payload);
    payload.zeroize();

    result
}

/// Write the payload for the given session ID, atomically.
///
/// The file is written under a temporary name and renamed into place, so that
/// an interrupted write cannot leave a half-written session behind — losing the
/// passphrase would mean losing the account's local data.
fn write_session_file(dir: &Path, id: &str, payload: &[u8]) -> Result<(), SecretError> {
    let final_path = dir.join(format!("{id}.json"));
    let temp_path = dir.join(format!("{id}.json.tmp"));

    let write = || -> io::Result<()> {
        fs::write(&temp_path, payload)?;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(FILE_MODE))?;
        fs::rename(&temp_path, &final_path)
    };

    if let Err(error) = write() {
        // Leaving a temporary file behind would be read as a session on the
        // next launch if it were ever named like one; it is not, but it is
        // still rubbish in the user's storage.
        let _ = fs::remove_file(&temp_path);
        return Err(SecretError::Service(error.to_string()));
    }

    Ok(())
}

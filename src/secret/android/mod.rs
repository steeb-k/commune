//! Secret backend storing each session sealed with an Android Keystore key.
//!
//! # What protects the session
//!
//! Three things, and it is worth being precise about which does what.
//!
//! **The Keystore key.** The payload is encrypted with AES-256-GCM using a key
//! generated inside the Android Keystore and never handed to us — see
//! [`keystore`]. It cannot be read out of the device, and it cannot be used by
//! anything that is not this application's UID. A file copied off a rooted
//! phone is ciphertext, and the key is not in the copy.
//!
//! **The application sandbox.** The files sit under `Context.getFilesDir()`,
//! owned by this application's UID. That is what
//! [`DataType::Persistent`] resolves to on Android, though only because
//! `crate::utils` was made to derive it: GTK's glue points `GLib`'s
//! `XDG_DATA_HOME` at `Context.getExternalFilesDir(null)`, which is external
//! storage, and using it would put these files somewhere USB and
//! `MANAGE_EXTERNAL_STORAGE` can reach.
//!
//! **`allowBackup` being off.** pixiewood leaves Android's default of `true`,
//! which would let `adb backup` and the system's cloud backup carry the files
//! away. `build-aux/android/patch-manifest.sh` turns it off after every
//! `pixiewood generate`. Even with it on the ciphertext would be useless
//! without the key, but there is no reason to hand it out.
//!
//! # What does not
//!
//! The key is not bound to the user being present: no
//! `setUserAuthenticationRequired`, so anything running as this UID can decrypt
//! without a lock-screen prompt. That is deliberate for now — Commune restores
//! sessions at startup, before there is a window to prompt over — and it is the
//! obvious next tightening if sessions ever become worth an unlock.
//!
//! Hardware backing is not guaranteed either. The Keystore uses secure hardware
//! where the device has it and falls back to software otherwise, and nothing
//! here refuses the software case.
//!
//! # Layout
//!
//! One file per session, `secrets.d/<session id>.sealed`, next to the
//! per-session data directories rather than inside them, so that enumerating
//! sessions does not mean walking every session's database. Inside is the
//! [`keystore`] envelope wrapping the same `version: 1` JSON document the macOS
//! backend stores in the Keychain.
//!
//! The document is duplicated from the macOS backend rather than shared,
//! because the two should be free to diverge.

mod keystore;

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

/// The extension of a stored session file.
///
/// Not `.json`: what is on disk is a [`keystore`] envelope, and calling it JSON
/// would invite someone to open it and wonder why it is not.
const FILE_EXTENSION: &str = "sealed";

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
    secrets_dir().join(format!("{id}.{FILE_EXTENSION}"))
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
        if path
            .extension()
            .is_none_or(|extension| extension != FILE_EXTENSION)
        {
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
    let sealed = fs::read(path).map_err(|error| format!("could not read session data: {error}"))?;

    // A failure here is not the same as a malformed file: it also happens when
    // the Keystore key is gone, which is what the user sees after clearing the
    // app's data or restoring to a different device. Either way this session
    // cannot be recovered, and the caller drops it with a warning.
    let data = keystore::decrypt(&sealed)
        .map_err(|error| format!("could not decrypt session data: {error}"))?;

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

    let mut plaintext =
        serde_json::to_vec(&secret).map_err(|error| SecretError::Service(error.to_string()))?;
    let sealed = keystore::encrypt(&plaintext);
    plaintext.zeroize();

    let sealed = sealed.map_err(|error| SecretError::Service(error.to_string()))?;

    write_session_file(&dir, &session.id, &sealed)
}

/// Write the payload for the given session ID, atomically.
///
/// The file is written under a temporary name and renamed into place, so that
/// an interrupted write cannot leave a half-written session behind — losing the
/// passphrase would mean losing the account's local data.
fn write_session_file(dir: &Path, id: &str, payload: &[u8]) -> Result<(), SecretError> {
    let final_path = dir.join(format!("{id}.{FILE_EXTENSION}"));
    let temp_path = dir.join(format!("{id}.{FILE_EXTENSION}.tmp"));

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

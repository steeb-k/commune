//! Secret backend using the Windows Credential Manager.
//!
//! Sessions are stored as generic credentials, one per session, with a target
//! name of `{APP_ID}/{session id}`. The application ID already carries the
//! profile (`…Commune` and `…Commune.Devel` are different prefixes), so a
//! development build never sees a stable build's sessions.
//!
//! Like the macOS Keychain, and unlike the Secret Service on Linux, the
//! Credential Manager cannot be searched on free-form attributes: enumeration
//! matches on the target name and nothing else. So the session's metadata does
//! not go into attributes, it is serialised into the credential blob next to
//! the passphrase. The session ID lives in the target name, which is what makes
//! a credential addressable for deletion.

use std::slice;

use matrix_sdk::authentication::oauth::ClientId;
use serde::{Deserialize, Serialize};
use tracing::{error, warn};
use windows::{
    Win32::{
        Foundation::{ERROR_NOT_FOUND, FILETIME},
        Security::Credentials::{
            CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW,
            CredEnumerateW, CredFree, CredWriteW,
        },
    },
    core::{HSTRING, PCWSTR, PWSTR},
};
use zeroize::{Zeroize, Zeroizing};

use super::{SecretError, SecretExt, StoredSession};
use crate::{APP_ID, spawn_tokio};

/// The version of the payload format that this version of the application
/// writes.
const CURRENT_VERSION: u8 = 1;

/// The largest credential blob the Credential Manager accepts, in bytes.
///
/// This is `CRED_MAX_CREDENTIAL_BLOB_SIZE` from `wincred.h`. Our payload is
/// nowhere near it — a few hundred bytes — but the check before writing is
/// worth having, because the failure it prevents is a silently lost account.
const MAX_BLOB_SIZE: usize = 5 * 512;

pub(crate) struct WindowsSecret;

impl SecretExt for WindowsSecret {
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
        let target = target_name(&session.id);

        spawn_tokio!(async move {
            // SAFETY: `target` outlives the call, and `CredDeleteW` only reads
            // the string it is given.
            let result = unsafe { CredDeleteW(&HSTRING::from(target), CRED_TYPE_GENERIC, None) };

            if let Err(error) = result {
                error!("Could not delete session data from the Credential Manager: {error}");
            }
        })
        .await
        .expect("task was not aborted");
    }
}

/// The payload stored in the Credential Manager for a session.
///
/// The Credential Manager cannot be searched on custom attributes, so
/// everything needed to reconstruct a [`StoredSession`] travels inside the
/// blob. The whole serialised payload is zeroized after it is handed over,
/// which is why the passphrase is a plain [`String`] here.
#[derive(Debug, Serialize, Deserialize)]
struct CredentialSecret {
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

/// The target name that identifies the credential of the session with the given
/// ID.
fn target_name(session_id: &str) -> String {
    format!("{APP_ID}/{session_id}")
}

/// Retrieve all the sessions stored in the Credential Manager.
fn restore_sessions_inner() -> Result<Vec<StoredSession>, SecretError> {
    let filter = HSTRING::from(format!("{APP_ID}/*"));
    let mut count = 0u32;
    let mut credentials = std::ptr::null_mut();

    // SAFETY: `filter` outlives the call, and the two out-parameters are valid
    // for writes of their own types. On success the buffer `credentials` points
    // at belongs to us until `CredFree`, which the guard below performs on
    // every path out.
    let result = unsafe {
        CredEnumerateW(
            PCWSTR(filter.as_ptr()),
            None,
            &raw mut count,
            &raw mut credentials,
        )
    };

    if let Err(error) = result {
        // An empty store is reported as an error rather than as a count of
        // zero, and it is the ordinary state of a machine that has never logged
        // in.
        if error.code() == ERROR_NOT_FOUND.to_hresult() {
            return Ok(Vec::new());
        }

        return Err(SecretError::Service(error.message()));
    }

    // From here on the buffer must be freed however this function ends.
    let _guard = CredentialsGuard(credentials);

    // SAFETY: `CredEnumerateW` returned success, so `credentials` points at
    // `count` valid pointers to `CREDENTIALW`.
    let entries = unsafe { slice::from_raw_parts(credentials, count as usize) };

    let mut sessions = Vec::with_capacity(entries.len());

    for entry in entries {
        // SAFETY: every pointer in the array points at a credential that the
        // guard keeps alive for the rest of this function.
        let credential = unsafe { &**entry };

        // SAFETY: `CredentialBlob` points at `CredentialBlobSize` bytes, or is
        // null when the size is zero.
        let data = Zeroizing::new(if credential.CredentialBlobSize == 0 {
            Vec::new()
        } else {
            unsafe {
                slice::from_raw_parts(
                    credential.CredentialBlob,
                    credential.CredentialBlobSize as usize,
                )
            }
            .to_vec()
        });

        // SAFETY: `TargetName` is a null-terminated wide string for as long as
        // the credential lives.
        let target = unsafe { credential.TargetName.to_string() }
            .unwrap_or_else(|_| "<unreadable target name>".to_owned());

        match session_from_secret(&data) {
            Ok(session) => sessions.push(session),
            // One unreadable credential must not stop the others from being
            // restored, or a single bad entry locks the user out of every
            // account.
            Err(error) => warn!("Ignoring credential {target}: {error}"),
        }
    }

    Ok(sessions)
}

/// The buffer returned by `CredEnumerateW`, freed when it goes out of scope.
struct CredentialsGuard(*mut *mut CREDENTIALW);

impl Drop for CredentialsGuard {
    fn drop(&mut self) {
        // SAFETY: the pointer came from a successful `CredEnumerateW` and is
        // freed exactly once, here.
        unsafe { CredFree(self.0.cast()) };
    }
}

/// Deserialize a [`StoredSession`] from the given credential blob.
fn session_from_secret(data: &[u8]) -> Result<StoredSession, String> {
    let secret: CredentialSecret =
        serde_json::from_slice(data).map_err(|error| format!("invalid session data: {error}"))?;

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

/// Store the given session in the Credential Manager, replacing any credential
/// that already holds the same session ID.
fn store_session_inner(session: &StoredSession) -> Result<(), SecretError> {
    let secret = CredentialSecret {
        version: CURRENT_VERSION,
        homeserver: session.homeserver.to_string(),
        user_id: session.user_id.to_string(),
        device_id: session.device_id.to_string(),
        id: session.id.clone(),
        client_id: session.client_id.as_ref().map(|id| id.as_str().to_owned()),
        passphrase: session.passphrase.as_str().to_owned(),
    };

    let mut payload = Zeroizing::new(
        serde_json::to_vec(&secret).map_err(|error| SecretError::Service(error.to_string()))?,
    );

    if payload.len() > MAX_BLOB_SIZE {
        return Err(SecretError::Service(format!(
            "session data is {} bytes, which is more than the {MAX_BLOB_SIZE} the Credential \
             Manager accepts",
            payload.len()
        )));
    }

    let target = HSTRING::from(target_name(&session.id));
    let user_name = HSTRING::from(session.user_id.as_str());
    // The Credential Manager control panel shows the comment, so it is the one
    // place a user can see what this credential belongs to. It is deliberately
    // not translated: it is read by whoever is looking at Windows' own list of
    // credentials, in among entries written by other applications in English.
    let comment = HSTRING::from(format!(
        "Commune: Matrix credentials for {}",
        session.user_id
    ));

    // `CREDENTIALW` declares its strings mutable because the same structure is
    // used for reading, where Windows owns them. `CredWriteW` only reads, so
    // handing it pointers into our own strings is sound.
    let credential = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_ptr().cast_mut()),
        Comment: PWSTR(comment.as_ptr().cast_mut()),
        LastWritten: FILETIME::default(),
        CredentialBlobSize: payload.len() as u32,
        CredentialBlob: payload.as_mut_ptr(),
        // The credential belongs to this user on this machine and survives a
        // reboot. "Local machine" names the scope it roams to, not who can read
        // it: it stays in this user's own store either way.
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(user_name.as_ptr().cast_mut()),
    };

    // SAFETY: every pointer in `credential` refers to a local that outlives the
    // call, and the blob is `CredentialBlobSize` bytes long. `CredWriteW` reads
    // the structure and does not retain it.
    //
    // This adds the credential, or replaces it if the target name already
    // exists, which is the overwriting behaviour that `SecretExt` asks for.
    let result = unsafe { CredWriteW(&raw const credential, 0) };

    // The blob was handed to Windows by pointer, so it could not be a
    // `Zeroizing` temporary; wipe it now that the call has returned.
    payload.zeroize();

    result.map_err(|error| SecretError::Service(error.message()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RUNTIME;

    /// Store a session, read it back, and delete it.
    ///
    /// This writes to the real Credential Manager, because the API has no
    /// notion of a store to test against — there is only the user's own. What
    /// keeps that acceptable is that the session ID is one nothing else will
    /// generate, that every assertion is about that one credential and ignores
    /// whatever else is in the store, and that the test deletes what it wrote.
    /// A credential left behind by a run that panicked is recognisable by its
    /// name and safe to remove.
    ///
    /// It is worth the intrusion: this module is the only place in the
    /// application that hands raw pointers to the operating system, and
    /// everything it protects — every account on the machine — is lost if it is
    /// wrong.
    #[test]
    fn a_session_survives_a_round_trip() {
        let id = format!("test-{}", std::process::id());
        let session = StoredSession {
            homeserver: "https://example.org".parse().expect("URL is valid"),
            user_id: "@alice:example.org".try_into().expect("user ID is valid"),
            device_id: "ADEVICEID".into(),
            id: id.clone(),
            client_id: Some(ClientId::new("a-client-id".to_owned())),
            passphrase: Zeroizing::new("a-passphrase".to_owned()),
        };

        RUNTIME
            .block_on(WindowsSecret::store_session(session.clone()))
            .expect("storing the session should succeed");

        let restored = RUNTIME
            .block_on(WindowsSecret::restore_sessions())
            .expect("restoring sessions should succeed")
            .into_iter()
            .find(|restored| restored.id == id);

        // Delete before asserting, so that a mismatch does not also leave the
        // credential behind.
        RUNTIME.block_on(WindowsSecret::delete_session(&session));

        let restored = restored.expect("the stored session should be found again");
        assert_eq!(restored.homeserver, session.homeserver);
        assert_eq!(restored.user_id, session.user_id);
        assert_eq!(restored.device_id, session.device_id);
        assert_eq!(restored.client_id, session.client_id);
        assert_eq!(restored.passphrase.as_str(), session.passphrase.as_str());

        let after_delete = RUNTIME
            .block_on(WindowsSecret::restore_sessions())
            .expect("restoring sessions should succeed")
            .into_iter()
            .any(|restored| restored.id == id);
        assert!(!after_delete, "the session should be gone once deleted");
    }
}

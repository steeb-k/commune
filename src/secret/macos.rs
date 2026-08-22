//! Secret backend using the macOS Keychain.
//!
//! Sessions are stored as generic password items, one per session, all under
//! the service name [`APP_ID`]. The application ID already carries the profile
//! (`…Commune` and `…Commune.Devel` are different services), so a development
//! build never sees a stable build's sessions.
//!
//! Unlike the Secret Service on Linux, the Keychain cannot be searched on
//! free-form attributes: the only fields we could query are the service and the
//! account. So the session's metadata does not go into item attributes, it is
//! serialised into the secret next to the passphrase. The account is the
//! session ID, which is what makes an item addressable for deletion.

use matrix_sdk::authentication::oauth::ClientId;
use security_framework::{
    item::{ItemClass, ItemSearchOptions, Limit},
    passwords::{delete_generic_password, get_generic_password, set_generic_password_options},
    passwords_options::PasswordOptions,
};
use serde::{Deserialize, Serialize};
use tracing::{error, warn};
use zeroize::Zeroizing;

use super::{SecretError, SecretExt, StoredSession};
use crate::{APP_ID, gettext_f, spawn_tokio};

/// The version of the payload format that this version of the application
/// writes.
const CURRENT_VERSION: u8 = 1;

/// The `errSecItemNotFound` status, returned by a search that matched nothing.
///
/// This is `security_framework_sys::base::errSecItemNotFound`, inlined so that
/// we do not have to depend on the `-sys` crate for a single constant.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

/// The raw name of the `kSecAttrAccount` attribute, as it appears in the
/// dictionary a Keychain search returns.
const ACCOUNT_ATTRIBUTE: &str = "acct";

pub(crate) struct MacosSecret;

impl SecretExt for MacosSecret {
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
        let id = session.id.clone();

        spawn_tokio!(async move {
            if let Err(error) = delete_generic_password(APP_ID, &id) {
                error!("Could not delete session data from the Keychain: {error}");
            }
        })
        .await
        .expect("task was not aborted");
    }
}

/// The payload stored in the Keychain for a session.
///
/// The Keychain cannot be searched on custom attributes, so everything needed
/// to reconstruct a [`StoredSession`] travels inside the secret. The whole
/// serialised payload is zeroized after it is handed to the Keychain, which is
/// why the passphrase is a plain [`String`] here.
#[derive(Debug, Serialize, Deserialize)]
struct KeychainSecret {
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

/// Retrieve all the sessions stored in the Keychain.
///
/// This takes two passes on purpose. `SecItemCopyMatching` rejects a query that
/// asks for the secret data of more than one item — `kSecReturnData` together
/// with `kSecMatchLimitAll` fails with `errSecParam` — so the first pass lists
/// the accounts and the second fetches each secret by name.
fn restore_sessions_inner() -> Result<Vec<StoredSession>, SecretError> {
    let results = match ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(APP_ID)
        .load_attributes(true)
        .limit(Limit::All)
        .search()
    {
        Ok(results) => results,
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => return Ok(Vec::new()),
        Err(error) => return Err(SecretError::Service(error.to_string())),
    };

    let mut sessions = Vec::with_capacity(results.len());

    for result in results {
        let Some(attributes) = result.simplify_dict() else {
            warn!("Ignoring Keychain item without attributes");
            continue;
        };
        // `acct` is the raw name of `kSecAttrAccount`, which is where
        // `store_session_inner` puts the session ID.
        let Some(account) = attributes.get(ACCOUNT_ATTRIBUTE) else {
            warn!("Ignoring Keychain item without an account");
            continue;
        };

        let data = match get_generic_password(APP_ID, account) {
            Ok(data) => Zeroizing::new(data),
            Err(error) => {
                warn!("Could not read the secret of Keychain item {account}: {error}");
                continue;
            }
        };

        match session_from_secret(&data) {
            Ok(session) => sessions.push(session),
            // One unreadable item must not stop the others from being restored,
            // or a single bad item locks the user out of every account.
            Err(error) => warn!("Ignoring Keychain item {account}: {error}"),
        }
    }

    Ok(sessions)
}

/// Deserialize a [`StoredSession`] from the given secret payload.
fn session_from_secret(data: &[u8]) -> Result<StoredSession, String> {
    let secret: KeychainSecret =
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

/// Store the given session in the Keychain, replacing any item that already
/// holds the same session ID.
fn store_session_inner(session: &StoredSession) -> Result<(), SecretError> {
    let secret = KeychainSecret {
        version: CURRENT_VERSION,
        homeserver: session.homeserver.to_string(),
        user_id: session.user_id.to_string(),
        device_id: session.device_id.to_string(),
        id: session.id.clone(),
        client_id: session.client_id.as_ref().map(|id| id.as_str().to_owned()),
        passphrase: session.passphrase.as_str().to_owned(),
    };

    let payload = Zeroizing::new(
        serde_json::to_vec(&secret).map_err(|error| SecretError::Service(error.to_string()))?,
    );

    let mut options = PasswordOptions::new_generic_password(APP_ID, &session.id);
    options.set_label(&gettext_f(
        // Translators: Do NOT translate the content between '{' and '}', this is a
        // variable name.
        "Commune: Matrix credentials for {user_id}",
        &[("user_id", session.user_id.as_str())],
    ));

    // This adds the item, or updates it if the service and account pair already
    // exists, which is the overwriting behaviour that `SecretExt` asks for.
    set_generic_password_options(&payload, options)
        .map_err(|error| SecretError::Service(error.to_string()))?;

    Ok(())
}

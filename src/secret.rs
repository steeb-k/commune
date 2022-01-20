use std::{path::PathBuf, string::FromUtf8Error};

use gettextrs::gettext;
use matrix_sdk::ruma::identifiers::{DeviceId, UserId};
use secret_service::{EncryptionType, SecretService};
use serde::{Deserialize, Serialize};
use serde_json::error::Error as JsonError;
use url::Url;

use crate::config::APP_ID;

#[derive(Debug, Clone)]
pub struct StoredSession {
    pub homeserver: Url,
    pub user_id: Box<UserId>,
    pub device_id: Box<DeviceId>,
    pub path: PathBuf,
    pub secret: Secret,
}

/// A possible error value when converting a `Secret` from a UTF-8 byte vector.
pub enum FromUtf8SecretError {
    Str(FromUtf8Error),
    Json(JsonError),
}

impl From<FromUtf8Error> for FromUtf8SecretError {
    fn from(err: FromUtf8Error) -> Self {
        Self::Str(err)
    }
}

impl From<JsonError> for FromUtf8SecretError {
    fn from(err: JsonError) -> Self {
        Self::Json(err)
    }
}

/// A `Secret` that can be stored in the `SecretService`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Secret {
    pub access_token: String,
    pub passphrase: String,
}

impl Secret {
    /// Returns a byte vec of this `Secret`’s contents.
    pub fn as_bytes(&self) -> Vec<u8> {
        serde_json::to_string(self).unwrap().as_bytes().to_vec()
    }

    /// Converts a vector of bytes to a `Secret`.
    pub fn from_utf8(vec: Vec<u8>) -> Result<Self, FromUtf8SecretError> {
        let s = String::from_utf8(vec)?;
        Ok(serde_json::from_str(&s)?)
    }
}

/// Retrieves all sessions stored to the `SecretService`
pub fn restore_sessions() -> Result<Vec<StoredSession>, secret_service::Error> {
    let ss = SecretService::new(EncryptionType::Dh)?;
    let collection = get_default_collection_unlocked(&ss)?;

    // Sessions that contain or produce errors are ignored.
    // TODO: Return error for corrupt sessions

    let res = collection
        .search_items([("xdg:schema", APP_ID)].into())?
        .iter()
        .filter_map(|item| {
            let attr = item.get_attributes().ok()?;

            let homeserver = Url::parse(attr.get("homeserver")?).ok()?;
            let user_id = UserId::parse(attr.get("user")?.as_str()).ok()?;
            let device_id = <&DeviceId>::from(attr.get("device-id")?.as_str()).to_owned();
            let path = PathBuf::from(attr.get("db-path")?);
            let secret = Secret::from_utf8(item.get_secret().ok()?).ok()?;

            Some(StoredSession {
                homeserver,
                path,
                user_id,
                device_id,
                secret,
            })
        })
        .collect();

    Ok(res)
}

/// Writes a session to the `SecretService`, overwriting any previously stored
/// session with the same `homeserver`, `username` and `device-id`.
pub fn store_session(session: &StoredSession) -> Result<(), secret_service::Error> {
    let ss = SecretService::new(EncryptionType::Dh)?;
    let collection = get_default_collection_unlocked(&ss)?;

    let attributes = [
        ("xdg:schema", APP_ID),
        ("homeserver", session.homeserver.as_str()),
        ("user", session.user_id.as_str()),
        ("device-id", session.device_id.as_str()),
        ("db-path", session.path.to_str().unwrap()),
    ]
    .into();

    collection.create_item(
        // Translators: The parameter is a Matrix User ID
        &gettext!("Fractal: Matrix credentials for {}", session.user_id),
        attributes,
        &session.secret.as_bytes(),
        true,
        "application/json",
    )?;

    Ok(())
}

/// Removes a session from the `SecretService`
pub fn remove_session(session: &StoredSession) -> Result<(), secret_service::Error> {
    let ss = SecretService::new(EncryptionType::Dh)?;
    let collection = get_default_collection_unlocked(&ss)?;

    let attributes = [
        ("xdg:schema", APP_ID),
        ("homeserver", session.homeserver.as_str()),
        ("user", session.user_id.as_str()),
        ("device-id", session.device_id.as_str()),
        ("db-path", session.path.to_str().unwrap()),
    ]
    .into();

    let items = collection.search_items(attributes)?;

    for item in items {
        item.delete()?;
    }

    Ok(())
}

fn get_default_collection_unlocked<'a>(
    secret_service: &'a SecretService,
) -> Result<secret_service::Collection<'a>, secret_service::Error> {
    let collection = match secret_service.get_default_collection() {
        Ok(col) => col,
        Err(secret_service::Error::NoResult) => {
            secret_service.create_collection("default", "default")?
        }
        Err(error) => return Err(error),
    };

    collection.unlock()?;

    Ok(collection)
}

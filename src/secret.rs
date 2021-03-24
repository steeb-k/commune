use secret_service::EncryptionType;
use secret_service::SecretService;
use std::collections::HashMap;

/// Retrives all sessions stored to the `SecretService`
pub fn restore_sessions() -> Result<Vec<(String, matrix_sdk::Session)>, secret_service::Error> {
    use std::convert::TryInto;

    let ss = SecretService::new(EncryptionType::Dh)?;
    let collection = ss.get_default_collection()?;

    // Sessions that contain or produce errors are ignored.
    // TODO: Return error for corrupt sessions
    let res = collection
        .get_all_items()?
        .iter()
        .filter_map(|item| {
            let attr = item.get_attributes().ok()?;
            if let (Some(homeserver), Some(access_token), Some(user_id), Some(device_id)) = (
                attr.get("homeserver"),
                String::from_utf8(item.get_secret().ok()?).ok(),
                attr.get("user-id")
                    .and_then(|s| s.to_string().try_into().ok()),
                attr.get("device-id")
                    .and_then(|s| Some(s.to_string().into())),
            ) {
                let session = matrix_sdk::Session {
                    access_token,
                    user_id,
                    device_id,
                };
                Some((homeserver.to_string(), session))
            } else {
                None
            }
        })
        .collect();

    Ok(res)
}

/// Writes a sessions to the `SecretService`, overwriting any previously stored session with the
/// same `homeserver`, `username` and `device-id`.
pub fn store_session(
    homeserver: &str,
    session: matrix_sdk::Session,
) -> Result<(), secret_service::Error> {
    let ss = SecretService::new(EncryptionType::Dh)?;
    let collection = ss.get_default_collection()?;

    let attributes: HashMap<&str, &str> = [
        ("user-id", session.user_id.as_str()),
        ("homeserver", homeserver),
        ("device-id", session.device_id.as_str()),
    ]
    .iter()
    .cloned()
    .collect();

    collection.create_item(
        "Fractal",
        attributes,
        session.access_token.as_bytes(),
        true,
        "text/plain",
    )?;

    Ok(())
}

//! Default implementation of the secret API for unsuppoted platforms.

/// Retrieves all sessions stored to the `SecretService`
pub async fn restore_sessions() -> Result<Vec<StoredSession>, SecretError> {
    unimplemented!()
}

/// Write the given session to the `SecretService`, overwriting any previously
/// stored session with the same attributes.
pub async fn store_session(session: StoredSession) -> Result<(), SecretError> {
    unimplemented!()
}

/// Delete the given session from the secret service.
pub async fn delete_session(session: StoredSession) {
    unimplemented!()
}

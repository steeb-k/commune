//! A user that can only be described by asking the homeserver, i.e. one
//! that sync will not update.
//!
//! The value half of the application's `RemoteUser`
//! (`src/session/remote/user.rs`): the profile the homeserver hands back,
//! read one field at a time so that a field the homeserver mangled costs
//! only that field. What stayed in the application is the `User` object it
//! extends and the staleness timer, which is the cache's.

use ruma::{
    OwnedMxcUri, UserId,
    api::client::profile::{AvatarUrl, DisplayName},
};
use tracing::error;

use crate::{UserFacingError, session::Session, spawn_tokio};

/// What can go wrong while asking for a user's profile.
#[derive(Debug, thiserror::Error)]
pub enum RemoteUserError {
    /// The homeserver could not answer.
    ///
    /// Boxed because `matrix_sdk::Error` is large enough that carrying it
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl UserFacingError for RemoteUserError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// The profile of a user, as the homeserver describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteUserProfile {
    /// The display name of the user, if they set one.
    pub display_name: Option<String>,
    /// The avatar of the user, if they set one.
    pub avatar_url: Option<OwnedMxcUri>,
}

impl Session {
    /// Ask the homeserver for the profile of the user with the given ID.
    ///
    /// A field that cannot be read is logged and left empty rather than
    /// failing the whole profile, as the application does.
    pub async fn remote_user_profile(
        &self,
        user_id: &UserId,
    ) -> Result<RemoteUserProfile, RemoteUserError> {
        let client = self.client();
        let user_id_clone = user_id.to_owned();
        let handle =
            spawn_tokio!(
                async move { client.account().fetch_user_profile_of(&user_id_clone).await }
            );

        let response = handle
            .await
            .expect("task was not aborted")
            .map_err(|fetch_error| {
                error!(%user_id, "Could not load user profile: {fetch_error}");
                RemoteUserError::Server(Box::new(fetch_error))
            })?;

        let display_name = match response.get_static::<DisplayName>() {
            Ok(display_name) => display_name,
            Err(read_error) => {
                error!(%user_id, "Could not deserialize user display name: {read_error}");
                None
            }
        };

        let avatar_url = match response.get_static::<AvatarUrl>() {
            Ok(avatar_url) => avatar_url,
            Err(read_error) => {
                error!(%user_id, "Could not deserialize user avatar URL: {read_error}");
                None
            }
        };

        Ok(RemoteUserProfile {
            display_name,
            avatar_url,
        })
    }
}

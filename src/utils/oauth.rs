//! Helper methods for OAuth-aware compatibility, to avoid pulling in all the
//! dependencies of the `experimental-oidc` SDK feature only to use a few
//! methods.

use std::error::Error;

use matrix_sdk::{reqwest::StatusCode, Client};
use ruma::{api::client::discovery::get_authentication_issuer, OwnedDeviceId};
use serde::Deserialize;
use tracing::warn;
use url::Url;

/// Get the URL of the OAuth 2.0 [authorization provider] for the current
/// homeserver of the given Matrix client.
///
/// [authorization provider]: https://github.com/matrix-org/matrix-spec-proposals/pull/2965
pub(crate) async fn fetch_auth_issuer(client: &Client) -> Option<Url> {
    #[allow(deprecated)]
    let result = client
        .send(get_authentication_issuer::msc2965::Request::new())
        .await;

    let issuer = match result {
        Ok(response) => response.issuer,
        Err(error) => {
            if error
                .as_client_api_error()
                .is_none_or(|error| error.status_code != StatusCode::NOT_FOUND)
            {
                warn!("Could not fetch authentication issuer: {error:?}");
            }

            return None;
        }
    };

    match issuer.parse() {
        Ok(url) => Some(url),
        Err(error) => {
            warn!("Could not parse authorization provider `{issuer}` as a URL: {error}");
            None
        }
    }
}

/// Part of an OAuth 2.0 provider metadata.
#[derive(Debug, Clone, Deserialize)]
struct ProviderMetadata {
    account_management_uri: Url,
}

/// Get the [account management URL] of the given authorization provider with
/// the given Matrix client, by using OIDC provider discovery.
///
/// [account management URL]: https://github.com/matrix-org/matrix-spec-proposals/pull/4191
pub(crate) async fn discover_account_management_url(
    client: &Client,
    issuer: Url,
) -> Result<Url, Box<dyn Error + Send + Sync>> {
    let mut config_url = issuer;
    // If the path does not end with a slash, the last segment is removed when
    // using `join`.
    if !config_url.path().ends_with('/') {
        let mut path = config_url.path().to_owned();
        path.push('/');
        config_url.set_path(&path);
    }

    let config_url = config_url.join(".well-known/openid-configuration")?;

    let http_client = client.http_client();
    let body = http_client
        .get(config_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let metadata = serde_json::from_slice::<ProviderMetadata>(&body)?;
    Ok(metadata.account_management_uri)
}

/// The possible [account management] actions.
///
/// [account management]: https://github.com/matrix-org/matrix-spec-proposals/pull/4191
#[derive(Debug, Clone)]
pub(crate) enum AccountManagementAction {
    /// View the user profile.
    Profile,
    /// Log out the session with the given device ID.
    SessionEnd { device_id: OwnedDeviceId },
    /// Deactivate the account.
    AccountDeactivate,
}

impl AccountManagementAction {
    /// The serialized action name.
    fn action_name(&self) -> &str {
        match self {
            Self::Profile => "org.matrix.profile",
            Self::SessionEnd { .. } => "org.matrix.session_end",
            Self::AccountDeactivate => "org.matrix.account_deactivate",
        }
    }

    /// Extra query field as a `(name, value)` tuple to add for this action.
    fn extra_data(&self) -> Option<(&str, &str)> {
        match self {
            Self::SessionEnd { device_id } => Some(("device_id", device_id.as_str())),
            _ => None,
        }
    }

    /// Add the given action to the given account management url
    pub(crate) fn add_to_account_management_url(&self, url: &mut Url) {
        let mut query_pairs = url.query_pairs_mut();
        query_pairs.append_pair("action", self.action_name());

        if let Some((name, value)) = self.extra_data() {
            query_pairs.append_pair(name, value);
        }
    }
}

//! Talking to an identity server, which the homeserver cannot do for us.
//!
//! Inviting somebody by email address needs an identity server: the
//! homeserver forwards the invitation, but the client must name the server
//! and hand over a token it registered there itself. The pinned ruma
//! carries the whole Identity Service API as types, but the SDK only sends
//! requests to the homeserver, so this module drives them over the SDK's
//! own HTTP client — the `mutual_rooms` precedent, with ruma doing the
//! (de)serializing.

use std::{borrow::Cow, cell::RefCell, collections::BTreeSet};

use gtk::glib;
use matrix_sdk::reqwest;
use ruma::{
    JsOption,
    api::{
        IncomingResponse, MatrixVersion, OutgoingRequestExt, SupportedVersions,
        auth_scheme::SendAccessToken,
        client::account::request_openid_token,
        identity_service::tos::{
            accept_terms_of_service::v2 as accept_terms, get_terms_of_service::v2 as get_terms,
        },
    },
    events::{GlobalAccountDataEventType, identity_server::IdentityServerEventContent},
    serde::Raw,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, error};
use url::Url;

use super::Session;
use crate::spawn_tokio;

/// An error from the identity server module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentityServerError {
    /// There is no identity server to talk to.
    ///
    /// Either the account has none configured and the homeserver suggests
    /// none, or the account data says explicitly that none should be used.
    NoServer,
    /// Something else went wrong.
    Other,
}

/// What inviting by email is waiting on.
#[derive(Debug, Clone)]
pub(crate) enum EmailInviteReadiness {
    /// The identity server has terms the account has not accepted.
    Terms(Vec<PendingTerm>),
    /// The invite can be sent.
    Ready {
        /// The host and port of the identity server, the way
        /// `POST /invite` wants it named.
        id_server: String,
        /// The token registered with the identity server.
        token: String,
    },
}

/// One policy of the identity server, in the language it was picked in.
#[derive(Debug, Clone)]
pub(crate) struct PendingTerm {
    /// The name of the policy.
    pub(crate) name: String,
    /// The URL of the policy document.
    pub(crate) url: String,
}

/// The content of the `m.accepted_terms` account data.
///
/// The pinned ruma does not type it, so it is spelled out here.
#[derive(Debug, Default, Serialize, Deserialize)]
struct AcceptedTermsContent {
    /// The URLs the account has accepted.
    #[serde(default)]
    accepted: Vec<String>,
}

/// The identity server in use, and where it comes from.
#[derive(Debug, Clone)]
pub(crate) enum IdentityServerChoice {
    /// Set on the account, in the `m.identity_server` account data.
    Account(String),
    /// The account data says explicitly that none should be used.
    Declined,
    /// Suggested by the homeserver's `.well-known` document.
    Homeserver(String),
    /// Nothing is set and the homeserver suggests nothing.
    None,
}

/// The identity server the given session would use, and where it comes
/// from.
pub(crate) async fn identity_server_choice(session: &Session) -> IdentityServerChoice {
    let client = session.client();
    let handle = spawn_tokio!(async move {
        let preference = match client
            .account()
            .account_data::<IdentityServerEventContent>()
            .await
        {
            Ok(Some(raw)) => raw
                .deserialize()
                .map_or(JsOption::Undefined, |content| content.base_url),
            _ => JsOption::Undefined,
        };

        match preference {
            JsOption::Some(base_url) => IdentityServerChoice::Account(base_url),
            JsOption::Null => IdentityServerChoice::Declined,
            JsOption::Undefined => {
                let Some(user_id) = client.user_id() else {
                    return IdentityServerChoice::None;
                };
                match well_known_identity_server(
                    client.http_client(),
                    user_id.server_name().as_str(),
                )
                .await
                {
                    Some(base_url) => IdentityServerChoice::Homeserver(base_url),
                    None => IdentityServerChoice::None,
                }
            }
        }
    });

    handle.await.expect("task was not aborted")
}

/// Set the identity server preference of the given session.
///
/// `Some` names a server, `Null` refuses one on purpose, and `Undefined`
/// removes the preference so the homeserver's suggestion applies again. A
/// named server must answer the status endpoint before it is written.
pub(crate) async fn set_identity_server_preference(
    session: &Session,
    preference: JsOption<String>,
) -> Result<(), IdentityServerError> {
    let client = session.client();
    let handle = spawn_tokio!(async move {
        if let JsOption::Some(base_url) = &preference {
            let base_url = Url::parse(base_url.trim_end_matches('/'))
                .map_err(|_| IdentityServerError::NoServer)?;
            let status_url = base_url
                .join("/_matrix/identity/v2")
                .map_err(|_| IdentityServerError::Other)?;
            let answers = client
                .http_client()
                .get(status_url)
                .send()
                .await
                .is_ok_and(|response| response.status().is_success());
            if !answers {
                return Err(IdentityServerError::NoServer);
            }
        }

        // The content type in ruma cannot be constructed, so the little
        // JSON is written by hand: a URL, an explicit null, or nothing.
        let content = match &preference {
            JsOption::Some(base_url) => serde_json::json!({ "base_url": base_url }),
            JsOption::Null => serde_json::json!({ "base_url": null }),
            JsOption::Undefined => serde_json::json!({}),
        };
        let raw = Raw::new(&content).map_err(|_| IdentityServerError::Other)?;
        client
            .account()
            .set_account_data_raw(
                GlobalAccountDataEventType::from("m.identity_server"),
                raw.cast_unchecked(),
            )
            .await
            .map_err(|error| {
                error!("Could not write the identity server preference: {error}");
                IdentityServerError::Other
            })?;

        Ok(())
    });

    let result = handle.await.expect("task was not aborted");
    if result.is_ok() {
        // The next use resolves against the new preference.
        session.identity_server().reset();
    }
    result
}

/// The identity server of a session, resolved and registered with lazily.
///
/// Nothing here talks to any identity server until something asks to invite
/// by email — an identity server learns who this account is the moment it
/// is registered with, and that is not this module's call to make on its
/// own.
#[derive(Debug, Default)]
pub(crate) struct IdentityServer {
    /// The resolved and validated base URL.
    base_url: RefCell<Option<Url>>,
    /// The token registered with the identity server at `base_url`.
    token: RefCell<Option<String>>,
    /// Whether the terms of the identity server were checked and none is
    /// waiting to be accepted.
    terms_done: std::cell::Cell<bool>,
}

impl IdentityServer {
    /// Forget everything resolved so far.
    ///
    /// The next use resolves, registers and checks the terms again.
    pub(crate) fn reset(&self) {
        self.base_url.take();
        self.token.take();
        self.terms_done.set(false);
    }

    /// What inviting by email is waiting on, for the given session.
    ///
    /// Resolves and validates the identity server, registers with it, and
    /// checks its terms of service, caching every step for the rest of the
    /// run.
    pub(crate) async fn email_invite_readiness(
        &self,
        session: &Session,
    ) -> Result<EmailInviteReadiness, IdentityServerError> {
        let base_url = self.resolve(session).await?;
        let token = self.register(session, &base_url).await?;

        if !self.terms_done.get() {
            let pending = self.pending_terms(session, &base_url).await?;
            if !pending.is_empty() {
                return Ok(EmailInviteReadiness::Terms(pending));
            }
            self.terms_done.set(true);
        }

        let id_server = host_and_port(&base_url).ok_or(IdentityServerError::Other)?;
        Ok(EmailInviteReadiness::Ready { id_server, token })
    }

    /// Accept the given policy URLs on behalf of the account.
    ///
    /// Tells the identity server, and records the URLs in the
    /// `m.accepted_terms` account data the way the specification asks, so
    /// no client asks about the same documents twice.
    pub(crate) async fn accept_terms(
        &self,
        session: &Session,
        urls: Vec<String>,
    ) -> Result<(), IdentityServerError> {
        let base_url = self.resolve(session).await?;
        let token = self.register(session, &base_url).await?;

        let http = session.client().http_client().clone();
        let base = base_url.clone();
        let urls_clone = urls.clone();
        let handle = spawn_tokio!(async move {
            let request = accept_terms::Request::new(urls_clone)
                .try_into_http_request::<Vec<u8>>(
                    base_str(&base),
                    SendAccessToken::IfRequired(&token),
                    Cow::Owned(supported_versions()),
                )
                .map_err(|_| IdentityServerError::Other)?;
            let response = execute(&http, request).await?;
            accept_terms::Response::try_from_http_response(response)
                .map_err(|_| IdentityServerError::Other)
        });
        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("The identity server did not accept the terms: {error:?}");
                IdentityServerError::Other
            })?;

        // Record the acceptance in the account data. Failing to write it is
        // not failing to accept — the identity server already knows — so it
        // is only logged.
        let client = session.client();
        let handle = spawn_tokio!(async move {
            let account = client.account();
            let event_type = GlobalAccountDataEventType::from("m.accepted_terms");

            let mut content = match account.account_data_raw(event_type.clone()).await {
                Ok(Some(raw)) => raw
                    .deserialize_as_unchecked::<AcceptedTermsContent>()
                    .unwrap_or_default(),
                _ => AcceptedTermsContent::default(),
            };
            for url in urls {
                if !content.accepted.contains(&url) {
                    content.accepted.push(url);
                }
            }

            match Raw::new(&content) {
                Ok(raw) => {
                    if let Err(error) = account
                        .set_account_data_raw(event_type, raw.cast_unchecked())
                        .await
                    {
                        error!("Could not record the accepted terms: {error}");
                    }
                }
                Err(error) => {
                    error!("Could not serialize the accepted terms: {error}");
                }
            }
        });
        handle.await.expect("task was not aborted");

        self.terms_done.set(true);
        Ok(())
    }

    /// Resolve the identity server of the session.
    ///
    /// The account data is asked first — `m.identity_server` is where a
    /// preference lives, and an explicit null there means "none, and stop
    /// asking". The homeserver's `.well-known` is the fallback. Whatever is
    /// found must answer the status endpoint before it is believed.
    async fn resolve(&self, session: &Session) -> Result<Url, IdentityServerError> {
        if let Some(base_url) = self.base_url.borrow().clone() {
            return Ok(base_url);
        }

        let client = session.client();
        let handle = spawn_tokio!(async move {
            // The preference on the account.
            let preference = match client
                .account()
                .account_data::<IdentityServerEventContent>()
                .await
            {
                Ok(Some(raw)) => raw.deserialize().map_or_else(
                    |error| {
                        debug!("Could not parse the m.identity_server account data: {error}");
                        JsOption::Undefined
                    },
                    |content| content.base_url,
                ),
                Ok(None) => JsOption::Undefined,
                Err(error) => {
                    debug!("Could not read the m.identity_server account data: {error}");
                    JsOption::Undefined
                }
            };

            let candidate = match preference {
                JsOption::Some(base_url) => Some(base_url),
                // The account asked for no identity server, which is an
                // answer, not an absence.
                JsOption::Null => return Err(IdentityServerError::NoServer),
                JsOption::Undefined => None,
            };

            let http = client.http_client();

            let candidate = if candidate.is_some() {
                candidate
            } else {
                // The `.well-known` of the server name, which is not
                // always the homeserver's own host.
                let server_name = client
                    .user_id()
                    .ok_or(IdentityServerError::Other)?
                    .server_name();
                well_known_identity_server(http, server_name.as_str()).await
            };

            let Some(candidate) = candidate else {
                return Err(IdentityServerError::NoServer);
            };
            let base_url = Url::parse(candidate.trim_end_matches('/'))
                .map_err(|_| IdentityServerError::NoServer)?;

            // The specification asks a client to check that the identity
            // server answers before using it.
            let status_url = base_url
                .join("/_matrix/identity/v2")
                .map_err(|_| IdentityServerError::Other)?;
            let answers = http
                .get(status_url)
                .send()
                .await
                .is_ok_and(|response| response.status().is_success());
            if !answers {
                debug!("The identity server at {base_url} does not answer");
                return Err(IdentityServerError::NoServer);
            }

            Ok(base_url)
        });

        let base_url = handle.await.expect("task was not aborted")?;
        self.base_url.replace(Some(base_url.clone()));
        Ok(base_url)
    }

    /// Register with the identity server at the given URL.
    ///
    /// The homeserver vouches for the account with an `OpenID` token,
    /// which the identity server exchanges for a token of its own.
    async fn register(
        &self,
        session: &Session,
        base_url: &Url,
    ) -> Result<String, IdentityServerError> {
        if let Some(token) = self.token.borrow().clone() {
            return Ok(token);
        }

        let client = session.client();
        let base = base_url.clone();
        let handle = spawn_tokio!(async move {
            let user_id = client
                .user_id()
                .ok_or(IdentityServerError::Other)?
                .to_owned();
            let openid = client
                .send(request_openid_token::v3::Request::new(user_id))
                .await
                .map_err(|error| {
                    error!("Could not get an OpenID token from the homeserver: {error}");
                    IdentityServerError::Other
                })?;

            let request = ruma::api::identity_service::authentication::register::v2::Request::new(
                openid.access_token,
                openid.token_type,
                openid.matrix_server_name,
                openid.expires_in,
            )
            .try_into_http_request::<Vec<u8>>(
                base_str(&base),
                SendAccessToken::None,
                Cow::Owned(supported_versions()),
            )
            .map_err(|_| IdentityServerError::Other)?;

            let response = execute(client.http_client(), request).await?;
            ruma::api::identity_service::authentication::register::v2::Response::try_from_http_response(response)
                .map_err(|error| {
                    error!("Could not register with the identity server: {error:?}");
                    IdentityServerError::Other
                })
        });

        let token = handle.await.expect("task was not aborted")?.token;
        self.token.replace(Some(token.clone()));
        Ok(token)
    }

    /// The policies of the identity server that the account has not
    /// accepted yet.
    async fn pending_terms(
        &self,
        session: &Session,
        base_url: &Url,
    ) -> Result<Vec<PendingTerm>, IdentityServerError> {
        let client = session.client();
        let base = base_url.clone();
        // The interface's languages, read on this thread: the list is
        // locale state, not something to ask for from a worker.
        let languages: Vec<String> = glib::language_names()
            .iter()
            .map(ToString::to_string)
            .collect();
        let handle = spawn_tokio!(async move {
            let request = get_terms::Request::new()
                .try_into_http_request::<Vec<u8>>(
                    base_str(&base),
                    SendAccessToken::None,
                    Cow::Owned(supported_versions()),
                )
                .map_err(|_| IdentityServerError::Other)?;
            let response = execute(client.http_client(), request).await?;
            let terms = get_terms::Response::try_from_http_response(response)
                .map_err(|_| IdentityServerError::Other)?;

            // What was accepted before, from any client.
            let accepted: BTreeSet<String> = match client
                .account()
                .account_data_raw(GlobalAccountDataEventType::from("m.accepted_terms"))
                .await
            {
                Ok(Some(raw)) => raw
                    .deserialize_as_unchecked::<AcceptedTermsContent>()
                    .map(|content| content.accepted.into_iter().collect())
                    .unwrap_or_default(),
                _ => BTreeSet::new(),
            };

            let mut pending = Vec::new();
            for policies in terms.policies.into_values() {
                // Pick the translation the interface would pick, falling
                // back the way gettext does: language, then English, then
                // whatever the server has.
                let localized = languages
                    .iter()
                    .find_map(|language| {
                        let language = language.split(['_', '.']).next()?;
                        policies.localized.get(language)
                    })
                    .or_else(|| policies.localized.get("en"))
                    .or_else(|| policies.localized.values().next());

                let Some(localized) = localized else {
                    continue;
                };
                if policies
                    .localized
                    .values()
                    .any(|translation| accepted.contains(&translation.url))
                {
                    // Accepting one translation of a policy accepts the
                    // policy.
                    continue;
                }

                pending.push(PendingTerm {
                    name: localized.name.clone(),
                    url: localized.url.clone(),
                });
            }

            Ok(pending)
        });

        handle.await.expect("task was not aborted")
    }
}

/// The `SupportedVersions` the identity endpoints are built against.
///
/// The Identity Service API's paths have been stable since Matrix 1.0, so
/// any version satisfies the path builder.
fn supported_versions() -> SupportedVersions {
    SupportedVersions {
        versions: [MatrixVersion::V1_0].into(),
        features: Default::default(),
    }
}

/// The base URL as the string ruma wants: no trailing slash.
fn base_str(base: &Url) -> &str {
    base.as_str().trim_end_matches('/')
}

/// The host and port of the given URL, the shape `id_server` fields want.
fn host_and_port(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

/// Execute the given request and hand the whole response back.
async fn execute(
    http: &reqwest::Client,
    request: http::Request<Vec<u8>>,
) -> Result<http::Response<Vec<u8>>, IdentityServerError> {
    let request = reqwest::Request::try_from(request).map_err(|_| IdentityServerError::Other)?;
    let response = http.execute(request).await.map_err(|error| {
        error!("Could not reach the identity server: {error}");
        IdentityServerError::Other
    })?;

    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|_| IdentityServerError::Other)?;

    http::Response::builder()
        .status(status)
        .body(body.to_vec())
        .map_err(|_| IdentityServerError::Other)
}

/// The identity server the `.well-known` of the given server names, if any.
async fn well_known_identity_server(http: &reqwest::Client, server_name: &str) -> Option<String> {
    /// The part of the `.well-known` document this module reads.
    #[derive(Debug, Deserialize)]
    struct WellKnown {
        /// The identity server information.
        #[serde(rename = "m.identity_server")]
        identity_server: Option<WellKnownIdentityServer>,
    }

    /// The identity server information in the `.well-known` document.
    #[derive(Debug, Deserialize)]
    struct WellKnownIdentityServer {
        /// The base URL of the identity server.
        base_url: String,
    }

    let url = format!("https://{server_name}/.well-known/matrix/client");
    let response = match http.get(url).send().await {
        Ok(response) => response,
        Err(error) => {
            debug!("Could not fetch the .well-known document: {error}");
            return None;
        }
    };
    if !response.status().is_success() {
        return None;
    }

    let body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => {
            debug!("Could not read the .well-known document: {error}");
            return None;
        }
    };
    match serde_json::from_slice::<WellKnown>(&body) {
        Ok(well_known) => well_known
            .identity_server
            .map(|identity_server| identity_server.base_url),
        Err(error) => {
            debug!("Could not parse the .well-known document: {error}");
            None
        }
    }
}

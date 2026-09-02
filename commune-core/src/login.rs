//! Logging in, creating an account, and resetting a forgotten password.
//!
//! The headless counterpart of the application's `login/`
//! (`src/login/mod.rs`, `homeserver_page.rs`, `method_page.rs`,
//! `in_browser_page.rs`, `register_page.rs`, `reset_password_page.rs`) and
//! of the stage selection in `components/dialogs/auth/mod.rs`: what a login
//! does, in the order the pages drive it, with the pages themselves left
//! behind. A [`LoginFlow`] is the application's `Login` object without its
//! navigation stack — one client bound to one homeserver, and every step
//! that client can take.
//!
//! What stayed in the application: the local HTTP server the browser is
//! redirected back to (`local_server.rs`), because a redirect is the
//! embedder's to receive — the desktop listens on loopback and Android
//! registers a custom scheme — so every method that needs a redirect URI
//! takes it as an argument; the `AuthDialog`, which asks a person for a
//! password, a registration token or their acceptance of the terms; the
//! password meter; and every sentence.
//!
//! What the embedder tells the core once, through [`crate::config`]: the
//! OAuth 2.0 client registration — the client URI and the redirect URIs it
//! registers — and the name a new device is given.

use matrix_sdk::{
    Client, ClientBuildError, Error as SdkError, HttpError,
    authentication::oauth::{
        ClientRegistrationData, OAuthAuthorizationData,
        error::{OAuthDiscoveryError, OAuthError},
        registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
    },
    config::RequestConfig,
    sanitize_server_name,
    utils::UrlOrQuery,
};
use ruma::{
    ClientSecret, OwnedClientSecret, OwnedServerName, OwnedSessionId, ServerName,
    api::{
        client::{
            account::{
                change_password, get_username_availability, register,
                request_password_change_token_via_email,
            },
            discovery::get_authorization_server_metadata::v1::Prompt,
            session::get_login_types::v3::LoginType,
            uiaa::{AuthData, AuthType, Dummy, Terms, ThirdpartyIdCredentials, UiaaInfo},
        },
        error::{ErrorBody, ErrorKind, StandardErrorBody},
    },
    assign,
    serde::Raw,
};
use tracing::{error, warn};
use url::Url;

use crate::{UserFacingError, config, spawn_tokio};

/// What can go wrong while logging in.
#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    /// The client for the homeserver could not be built: the homeserver
    /// could not be discovered, or the URL is not a homeserver.
    ///
    /// Boxed because the SDK's errors are large enough that carrying them
    /// by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Build(Box<ClientBuildError>),
    /// The homeserver's authorization server could not be discovered, for
    /// a reason other than it not having one.
    #[error(transparent)]
    Discovery(Box<OAuthDiscoveryError>),
    /// The homeserver would not say how it can be logged in to.
    #[error(transparent)]
    LoginTypes(Box<HttpError>),
    /// The homeserver does not say it can create an account in the
    /// browser.
    #[error("the homeserver does not allow creating an account from here")]
    AccountCreationUnsupported,
    /// The OAuth 2.0 authorization URL could not be built.
    #[error(transparent)]
    Authorization(Box<OAuthError>),
    /// The Matrix SSO URL could not be built.
    #[error(transparent)]
    SsoUrl(Box<SdkError>),
    /// The homeserver refused the login.
    #[error(transparent)]
    Login(Box<SdkError>),
}

impl UserFacingError for LoginError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so these are only the fallback.
            Self::Build(error) => error.to_string(),
            Self::Login(error) => error.to_string(),
            Self::Discovery(_) | Self::LoginTypes(_) | Self::Authorization(_) | Self::SsoUrl(_) => {
                "Could not set up login".to_owned()
            }
            Self::AccountCreationUnsupported => {
                "This homeserver does not allow creating an account from here".to_owned()
            }
        }
    }
}

/// What can go wrong while creating an account.
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    /// The homeserver wants an authentication stage completed first.
    ///
    /// Boxed because the info carries every flow the homeserver offers.
    #[error("the homeserver wants an authentication stage completed")]
    Uiaa(Box<UiaaInfo>),
    /// The homeserver does not allow creating an account.
    ///
    /// Told apart from the other refusals because the catch-all for
    /// `M_FORBIDDEN` is "Invalid credentials", which is not what it means
    /// here.
    #[error("the homeserver does not allow creating an account")]
    Forbidden,
    /// The homeserver refused for another reason.
    #[error(transparent)]
    Server(Box<SdkError>),
}

impl UserFacingError for RegisterError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::Uiaa(_) => "Could not create account".to_owned(),
            Self::Forbidden => "This homeserver does not allow creating an account".to_owned(),
            Self::Server(error) => error.to_string(),
        }
    }
}

/// What can go wrong while resetting a password.
#[derive(Debug, thiserror::Error)]
pub enum ResetPasswordError {
    /// No account on the homeserver uses the email address.
    #[error("no account uses that email address")]
    EmailNotFound,
    /// The homeserver cannot send email, or refuses the address.
    #[error("the homeserver cannot send email")]
    EmailDenied,
    /// The link in the email has not been opened yet.
    #[error("the link in the email has not been opened")]
    LinkNotOpened,
    /// The proof that the email was confirmed could not be built.
    #[error("could not build the email identity authentication data")]
    InvalidAuth,
    /// The homeserver refused for another reason.
    #[error(transparent)]
    Server(Box<HttpError>),
}

impl UserFacingError for ResetPasswordError {
    fn to_user_facing(&self) -> String {
        match self {
            // The address is not on any account here. Said plainly rather
            // than vaguely: a homeserver that answers this has already told
            // anybody asking, so there is nothing to protect by being coy.
            Self::EmailNotFound => {
                "No account on this homeserver uses that email address.".to_owned()
            }
            Self::EmailDenied => {
                "This homeserver cannot send email, so a password cannot be reset here.".to_owned()
            }
            Self::LinkNotOpened => "Open the link in the email first, then try again".to_owned(),
            Self::InvalidAuth => "Could not reset password".to_owned(),
            Self::Server(error) => error.to_string(),
        }
    }
}

/// The API a homeserver logs in through, and what it offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginApi {
    /// The OAuth 2.0 API, which replaces the Matrix native flows.
    OAuth,
    /// The Matrix native API.
    Matrix {
        /// Whether logging in with a password is offered.
        supports_password: bool,
        /// Whether logging in with SSO is offered.
        supports_sso: bool,
    },
}

/// What a homeserver says about a username somebody would like to
/// register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsernameAvailability {
    /// The homeserver says the username is free.
    Free,
    /// The homeserver says the username cannot be registered, without
    /// saying why.
    Unavailable,
    /// The username is already taken.
    Taken,
    /// The username is not valid on this homeserver.
    Invalid,
    /// The username is reserved by the homeserver.
    Reserved,
    /// The homeserver did not say, so the username is treated as usable.
    ///
    /// The register request is the authority anyway; this endpoint is a
    /// courtesy and a homeserver is allowed not to answer it.
    Unknown,
}

impl UsernameAvailability {
    /// Whether registration may be attempted with a username in this state.
    #[must_use]
    pub const fn allows_register(self) -> bool {
        matches!(self, Self::Free | Self::Unknown)
    }
}

/// How the browser came back from a Matrix SSO login.
#[derive(Debug, Clone)]
pub enum SsoCallback {
    /// The redirect, with the login token in its query.
    Redirect(UrlOrQuery),
    /// The login token, already taken out of the redirect.
    Token(String),
}

/// The session a password reset is happening in, as far as the homeserver
/// is concerned.
#[derive(Debug, Clone)]
pub struct EmailSession {
    /// The address the link was sent to.
    pub address: String,
    /// The session ID the homeserver gave us for it.
    pub sid: OwnedSessionId,
    /// The secret that proves the session is ours.
    ///
    /// It is generated once per reset rather than once per request, because
    /// it is what ties the `requestToken` call to the `password` call.
    pub client_secret: OwnedClientSecret,
    /// How many times the link has been asked for.
    ///
    /// The spec uses this to tell "send it again" apart from a retried
    /// request, so it must go up only when the user asks for another email.
    pub send_attempt: u32,
}

/// The next stage of a user-interactive authentication, chosen the way the
/// application's `AuthDialog` chooses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthStage {
    /// The completed stages.
    pub completed: Vec<AuthType>,
    /// The stage to perform.
    pub stage: AuthType,
    /// The ID of the authentication session.
    pub session: Option<String>,
}

impl AuthStage {
    /// The stages the application's dialog can walk a person through.
    pub const INTERACTIVE: &[AuthType] = &[
        AuthType::Password,
        AuthType::Sso,
        AuthType::Dummy,
        AuthType::RegistrationToken,
        AuthType::Terms,
    ];

    /// The stages that need nothing from a person: the dummy stage, and the
    /// terms once they have been shown and accepted.
    pub const WITHOUT_INPUT: &[AuthType] = &[AuthType::Dummy, AuthType::Terms];

    /// Choose the next stage from the given UIAA info, preferring one of the
    /// given supported stages.
    ///
    /// The possible next stages are the next stage in the flows that have
    /// the same stages as the ones already completed. The first supported
    /// one wins; if none is supported, the first one is taken, for the
    /// web-based fallback.
    ///
    /// Returns `None` if the next stage could not be determined.
    #[must_use]
    pub fn next(uiaa_info: &UiaaInfo, supported: &[AuthType]) -> Option<Self> {
        let stages = uiaa_info
            .flows
            .iter()
            .filter_map(|flow| flow.stages.strip_prefix(uiaa_info.completed.as_slice()))
            .filter_map(|stages_left| stages_left.first());

        let mut next_stage = None;
        for stage in stages {
            if supported.contains(stage) {
                // We found a supported stage.
                next_stage = Some(stage);
                break;
            } else if next_stage.is_none() {
                // We will default to the first stage if we do not find one that we support.
                next_stage = Some(stage);
            }
        }

        let stage = next_stage?.clone();

        Some(Self {
            completed: uiaa_info.completed.clone(),
            stage,
            session: uiaa_info.session.clone(),
        })
    }

    /// The authentication data for the password stage, for the given user
    /// — the application's password page of the `AuthDialog`.
    #[must_use]
    pub fn password_data(&self, user_id: &ruma::UserId, password: &str) -> AuthData {
        use ruma::api::client::uiaa::{Password, UserIdentifier};

        AuthData::Password(assign!(
            Password::new(UserIdentifier::Matrix(user_id.to_owned().into()), password.to_owned()),
            { session: self.session.clone() }
        ))
    }

    /// The authentication data for this stage, when it needs nothing from a
    /// person.
    ///
    /// The dummy stage never does. The terms stage is answered by
    /// acceptance, so a caller taking this for it has shown them, or is
    /// creating the account on the understanding that creating it is
    /// accepting them. Anything else needs input and returns `None`.
    #[must_use]
    pub fn auth_data_without_input(&self) -> Option<AuthData> {
        let session = self.session.clone();

        match self.stage {
            AuthType::Dummy => Some(AuthData::Dummy(assign!(Dummy::new(), { session }))),
            AuthType::Terms => Some(AuthData::Terms(assign!(Terms::new(), { session }))),
            _ => None,
        }
    }
}

/// A login in progress: one client bound to one homeserver, and every step
/// it can take.
///
/// Cheap to clone; every clone shares the same client.
#[derive(Debug, Clone)]
pub struct LoginFlow {
    /// The client to log in with.
    client: Client,
    /// The name of the server that was chosen, when it is a domain name.
    server_name: Option<OwnedServerName>,
    /// The API the homeserver logs in through.
    api: LoginApi,
}

impl LoginFlow {
    /// Discover the given homeserver and what it offers for logging in.
    ///
    /// With `autodiscovery`, `homeserver` is a server name to discover the
    /// homeserver of — or a URL, which is taken as the homeserver's
    /// `.well-known` says. Without it, `homeserver` is the homeserver's
    /// URL and is only checked to be a Matrix homeserver. The default
    /// homeserver is always a domain name, so it is always discovered; the
    /// setting is about a URL somebody types.
    pub async fn discover(homeserver: &str, autodiscovery: bool) -> Result<Self, LoginError> {
        let client = if autodiscovery {
            Self::build_client_with_autodiscovery(homeserver).await?
        } else {
            Self::build_client_with_url(homeserver).await?
        };

        let server_name = autodiscovery
            .then(|| sanitize_server_name(homeserver).ok())
            .flatten();

        let api = Self::discover_login_api(&client).await?;

        Ok(Self {
            client,
            server_name,
            api,
        })
    }

    /// Try to build a client by using homeserver autodiscovery.
    async fn build_client_with_autodiscovery(homeserver: &str) -> Result<Client, LoginError> {
        let homeserver = homeserver.to_owned();
        let handle = spawn_tokio!(async move {
            Self::client_builder()
                .server_name_or_homeserver_url(homeserver)
                .build()
                .await
        });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|build_error| {
                warn!("Could not discover homeserver: {build_error}");
                LoginError::Build(Box::new(build_error))
            })
    }

    /// Try to build a client by using the homeserver's URL.
    async fn build_client_with_url(homeserver: &str) -> Result<Client, LoginError> {
        let homeserver = homeserver.to_owned();
        let handle = spawn_tokio!(async move {
            let client = Self::client_builder()
                .respect_login_well_known(false)
                .homeserver_url(homeserver)
                .build()
                .await?;

            // Call the `GET /versions` endpoint to make sure that the URL belongs to a
            // Matrix homeserver.
            client.server_versions().await?;

            Ok::<_, ClientBuildError>(client)
        });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|build_error| LoginError::Build(Box::new(build_error)))
    }

    /// Discover the login API supported by the homeserver.
    async fn discover_login_api(client: &Client) -> Result<LoginApi, LoginError> {
        // Check if the server supports the OAuth 2.0 API.
        let oauth = client.oauth();
        let handle = spawn_tokio!(async move { oauth.server_metadata().await });

        match handle.await.expect("task was not aborted") {
            Ok(_) => return Ok(LoginApi::OAuth),
            Err(discovery_error) => {
                if !discovery_error.is_not_supported() {
                    warn!("Could not get authorization server metadata: {discovery_error}");
                    return Err(LoginError::Discovery(Box::new(discovery_error)));
                }
                // Fallback to the Matrix native API.
            }
        }

        let matrix_auth = client.matrix_auth();
        let handle = spawn_tokio!(async move { matrix_auth.get_login_types().await });

        let login_types = handle
            .await
            .expect("task was not aborted")
            .map_err(|types_error| {
                warn!("Could not get available Matrix login types: {types_error}");
                LoginError::LoginTypes(Box::new(types_error))
            })?
            .flows;

        Ok(LoginApi::Matrix {
            supports_password: login_types
                .iter()
                .any(|login_type| matches!(login_type, LoginType::Password(_))),
            supports_sso: login_types
                .iter()
                .any(|login_type| matches!(login_type, LoginType::Sso(_))),
        })
    }

    /// Construct a client builder with the proper configuration.
    fn client_builder() -> matrix_sdk::ClientBuilder {
        Client::builder()
            .request_config(RequestConfig::new().retry_limit(2))
            // Otherwise the SDK builds its own client with the TLS backend
            // that does not work on Android. See `crate::tls`.
            .http_client(crate::tls::matrix_client())
    }

    /// The client this login uses.
    ///
    /// Authenticated once a login step succeeded;
    /// `SessionList::adopt_logged_in_client` is where it becomes a session.
    #[must_use]
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The client this login uses, taken out of the flow.
    #[must_use]
    pub fn into_client(self) -> Client {
        self.client
    }

    /// The URL of the homeserver.
    #[must_use]
    pub fn homeserver(&self) -> Url {
        self.client.homeserver()
    }

    /// The name of the server that was chosen, when it is a domain name.
    ///
    /// `None` when a URL was typed instead, since a URL is not a server
    /// name and guessing one from it would be wrong.
    #[must_use]
    pub fn server_name(&self) -> Option<&ServerName> {
        self.server_name.as_deref()
    }

    /// The API the homeserver logs in through, and what it offers.
    #[must_use]
    pub fn api(&self) -> LoginApi {
        self.api
    }

    /// Log in with the password login type.
    pub async fn login_with_password(
        &self,
        username: &str,
        password: &str,
    ) -> Result<(), LoginError> {
        let client = self.client.clone();
        let username = username.to_owned();
        let password = password.to_owned();

        let handle = spawn_tokio!(async move {
            client
                .matrix_auth()
                .login_username(&username, &password)
                .initial_device_display_name(config::app_name())
                .send()
                .await
        });

        handle
            .await
            .expect("task was not aborted")
            .map(|_| ())
            .map_err(|login_error| {
                warn!("Could not log in: {login_error}");
                LoginError::Login(Box::new(login_error))
            })
    }

    /// Whether the homeserver says it can create an account in the
    /// browser.
    ///
    /// A server that does not advertise `create` among its prompts is
    /// allowed to ignore the parameter, which would silently show a log-in
    /// page to somebody who has no account.
    pub async fn supports_oauth_account_creation(&self) -> bool {
        let oauth = self.client.oauth();
        let handle = spawn_tokio!(async move { oauth.server_metadata().await });

        match handle.await.expect("task was not aborted") {
            Ok(metadata) => metadata.prompt_values_supported.contains(&Prompt::Create),
            Err(discovery_error) => {
                warn!("Could not get authorization server metadata: {discovery_error}");
                false
            }
        }
    }

    /// Prepare to log in via the OAuth 2.0 API: the authorization to open
    /// in the browser, which comes back on the given redirect URI.
    ///
    /// With `create_account`, the browser is asked to create one — the
    /// standard's own way to say "this person has no account yet", from
    /// `OpenID` Connect's `prompt=create` — after checking that the homeserver
    /// says it can.
    pub async fn oauth_authorization(
        &self,
        redirect_uri: Url,
        create_account: bool,
    ) -> Result<OAuthAuthorizationData, LoginError> {
        if create_account && !self.supports_oauth_account_creation().await {
            return Err(LoginError::AccountCreationUnsupported);
        }

        let oauth = self.client.oauth();
        let handle = spawn_tokio!(async move {
            let mut builder =
                oauth.login(redirect_uri, None, Some(client_registration_data()), None);

            if create_account {
                builder = builder.prompt(vec![Prompt::Create]);
            }

            builder.build().await
        });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|authorization_error| {
                warn!("Could not construct OAuth 2.0 authorization URL: {authorization_error}");
                LoginError::Authorization(Box::new(authorization_error))
            })
    }

    /// Finish the OAuth 2.0 login process with what came back on the
    /// redirect URI.
    pub async fn finish_oauth_login(&self, redirect: UrlOrQuery) -> Result<(), LoginError> {
        let oauth = self.client.oauth();
        let handle = spawn_tokio!(async move { oauth.finish_login(redirect).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|login_error| {
                warn!("Could not log in via OAuth 2.0: {login_error}");
                LoginError::Login(Box::new(login_error))
            })
    }

    /// Prepare to log in via the Matrix SSO API: the URL to open in the
    /// browser, which comes back on the given redirect URI.
    pub async fn sso_url(&self, redirect_uri: &Url) -> Result<Url, LoginError> {
        let matrix_auth = self.client.matrix_auth();
        let redirect_uri = redirect_uri.to_string();
        let handle =
            spawn_tokio!(async move { matrix_auth.get_sso_login_url(&redirect_uri, None).await });

        let url = handle
            .await
            .expect("task was not aborted")
            .map_err(|url_error| {
                warn!("Could not build Matrix SSO URL: {url_error}");
                LoginError::SsoUrl(Box::new(url_error))
            })?;

        Ok(Url::parse(&url).expect("Matrix SSO URL should be a valid URL"))
    }

    /// Finish the Matrix SSO login process with what came back on the
    /// redirect URI.
    pub async fn finish_sso_login(&self, callback: SsoCallback) -> Result<(), LoginError> {
        let matrix_auth = self.client.matrix_auth();

        let handle = spawn_tokio!(async move {
            let builder = match callback {
                SsoCallback::Redirect(redirect) => matrix_auth
                    .login_with_sso_callback(redirect)
                    .map_err(|callback_error| SdkError::UnknownError(callback_error.into()))?,
                SsoCallback::Token(token) => matrix_auth.login_token(&token),
            };

            builder
                .initial_device_display_name(config::app_name())
                .send()
                .await
        });

        handle
            .await
            .expect("task was not aborted")
            .map(|_| ())
            .map_err(|login_error| {
                warn!("Could not log in via SSO: {login_error}");
                LoginError::Login(Box::new(login_error))
            })
    }

    /// Ask the homeserver whether the given username can be registered.
    pub async fn check_username_availability(&self, username: &str) -> UsernameAvailability {
        let client = self.client.clone();
        let request = get_username_availability::v3::Request::new(username.to_owned());
        let handle = spawn_tokio!(async move { client.send(request).await });

        match handle.await.expect("task was not aborted") {
            Ok(response) if response.available => UsernameAvailability::Free,
            Ok(_) => UsernameAvailability::Unavailable,
            Err(availability_error) => match http_error_kind(&availability_error) {
                Some(ErrorKind::UserInUse) => UsernameAvailability::Taken,
                Some(ErrorKind::InvalidUsername) => UsernameAvailability::Invalid,
                Some(ErrorKind::Exclusive) => UsernameAvailability::Reserved,
                _ => {
                    // The homeserver does not have to answer this, and the
                    // register request settles it anyway.
                    warn!(
                        "Could not check whether the username is available: {availability_error}"
                    );
                    UsernameAvailability::Unknown
                }
            },
        }
    }

    /// Create an account with the given username and password, answering
    /// the homeserver's authentication with the given data.
    ///
    /// One request: a homeserver that wants a stage completed answers with
    /// [`RegisterError::Uiaa`], and the caller chooses the stage with
    /// [`AuthStage::next`] and calls again with its data — the application's
    /// `AuthDialog` loop, with the dialog left to the embedder. On success
    /// the client is logged in, the same as after a password login.
    pub async fn register(
        &self,
        username: &str,
        password: &str,
        auth: Option<AuthData>,
    ) -> Result<(), RegisterError> {
        let client = self.client.clone();
        let mut request = register::v3::Request::new();
        request.username = Some(username.to_owned());
        request.password = Some(password.to_owned());
        request.initial_device_display_name = Some(config::app_name().to_owned());
        request.auth = auth;

        let handle = spawn_tokio!(async move { client.matrix_auth().register(request).await });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(register_error) => {
                if let Some(uiaa_info) = register_error.as_uiaa_response() {
                    return Err(RegisterError::Uiaa(Box::new(uiaa_info.clone())));
                }

                warn!("Could not create account: {register_error}");

                if matches!(
                    register_error.client_api_error_kind(),
                    Some(ErrorKind::Forbidden)
                ) {
                    return Err(RegisterError::Forbidden);
                }

                Err(RegisterError::Server(Box::new(register_error)))
            }
        }
    }

    /// Ask the homeserver to email a password-reset link to the given
    /// address.
    ///
    /// A resend keeps the secret of the previous session for the same
    /// address and bumps the attempt; a first send makes both.
    pub async fn request_password_reset_email(
        &self,
        address: &str,
        previous: Option<&EmailSession>,
    ) -> Result<EmailSession, ResetPasswordError> {
        let address = address.trim().to_owned();

        let (client_secret, send_attempt) = match previous {
            Some(session) if session.address == address => {
                (session.client_secret.clone(), session.send_attempt + 1)
            }
            _ => (ClientSecret::new(), 1),
        };

        let client = self.client.clone();
        let request = request_password_change_token_via_email::v3::Request::new(
            client_secret.clone(),
            address.clone(),
            send_attempt.into(),
        );
        let handle = spawn_tokio!(async move { client.send(request).await });

        match handle.await.expect("task was not aborted") {
            Ok(response) => Ok(EmailSession {
                address,
                sid: response.sid,
                client_secret,
                send_attempt,
            }),
            Err(email_error) => {
                warn!("Could not request a password reset email: {email_error}");
                Err(reset_email_error(email_error))
            }
        }
    }

    /// Set the new password, if the link has been opened.
    ///
    /// Every other session is signed out: the account has been out of the
    /// owner's hands for as long as the password was unknown to them. This
    /// is the endpoint's own default, said out loud.
    pub async fn reset_password(
        &self,
        session: &EmailSession,
        password: &str,
    ) -> Result<(), ResetPasswordError> {
        let auth = email_identity_auth(session).ok_or(ResetPasswordError::InvalidAuth)?;

        let client = self.client.clone();
        let password = password.to_owned();
        let handle = spawn_tokio!(async move {
            let mut request = change_password::v3::Request::new(password);
            request.logout_devices = true;
            request.auth = Some(auth);

            client.send(request).await
        });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(reset_error) => {
                if reset_error.as_uiaa_response().is_some() {
                    // The homeserver is still waiting for the link to be
                    // opened. That is not a failure, it is a "not yet".
                    return Err(ResetPasswordError::LinkNotOpened);
                }

                warn!("Could not reset the password: {reset_error}");
                Err(ResetPasswordError::Server(Box::new(reset_error)))
            }
        }
    }
}

/// Client registration data for the OAuth 2.0 API, from what the embedder
/// configured.
///
/// The redirect URIs are the embedder's: the desktop application registers
/// the IPv4 and IPv6 loopback addresses its local server redirects to, and
/// Android its fixed custom-scheme URI, alone, since it has to match
/// exactly what the authorization request sends. The client URI is the
/// embedder's too, and matrix.org's authorization server checks it against
/// a custom scheme read as reverse DNS — see the application's
/// `client_registration_data` for the whole story.
fn client_registration_data() -> ClientRegistrationData {
    let oauth_client = config::oauth_client();

    let mut client_metadata = ClientMetadata::new(
        ApplicationType::Native,
        vec![OAuthGrantType::AuthorizationCode {
            redirect_uris: oauth_client.redirect_uris.clone(),
        }],
        Localized::new(oauth_client.client_uri.clone(), None),
    );
    client_metadata.client_name = Some(Localized::new(config::app_name().to_owned(), None));

    Raw::new(&client_metadata)
        .expect("client metadata should serialize to JSON successfully")
        .into()
}

/// The authentication data proving the email address of the given session
/// was confirmed.
///
/// `EmailIdentity` is `non_exhaustive`, so it is built through
/// `AuthData::new` from the JSON the spec describes rather than from its
/// fields.
fn email_identity_auth(session: &EmailSession) -> Option<AuthData> {
    let credentials =
        ThirdpartyIdCredentials::new(session.sid.clone(), session.client_secret.clone());

    let credentials = match serde_json::to_value(&credentials) {
        Ok(credentials) => credentials,
        Err(serialize_error) => {
            error!("Could not serialize the third-party identifier credentials: {serialize_error}");
            return None;
        }
    };

    let mut data = serde_json::Map::new();
    data.insert("threepid_creds".to_owned(), credentials);

    match AuthData::new("m.login.email.identity", None, data) {
        Ok(auth) => Some(auth),
        Err(build_error) => {
            error!("Could not construct the email identity authentication data: {build_error}");
            None
        }
    }
}

/// What to make of a homeserver that would not send the email.
fn reset_email_error(error: HttpError) -> ResetPasswordError {
    match http_error_kind(&error) {
        Some(ErrorKind::ThreepidNotFound) => ResetPasswordError::EmailNotFound,
        // What Synapse answers when it has no email configuration at all, as
        // well as when it refuses the address itself.
        Some(ErrorKind::ThreepidDenied) => ResetPasswordError::EmailDenied,
        _ => ResetPasswordError::Server(Box::new(error)),
    }
}

/// The error kind of the given HTTP error, if it is a Matrix API error.
fn http_error_kind(error: &HttpError) -> Option<&ErrorKind> {
    let ErrorBody::Standard(StandardErrorBody { kind, .. }) = &error.as_client_api_error()?.body
    else {
        return None;
    };

    Some(kind)
}

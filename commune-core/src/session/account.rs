//! The account itself: its password, its third-party identifiers, and its
//! deactivation.
//!
//! These are the application's account settings subpages
//! (`account_settings/general_page/{change_password,third_party_ids,
//! deactivate_account}_subpage.rs`) with the widgets taken out. Each request
//! that the homeserver guards with user-interactive authentication is made
//! once without credentials and, when the homeserver asks for the password
//! stage, once more with the account password — the same two steps
//! `UserSessions::sign_out` takes, since an embedder with a password field
//! has nothing else to answer with.

use ruma::{
    OwnedClientSecret, OwnedSessionId, UInt,
    api::{client::uiaa::AuthData, error::ErrorKind},
    thirdparty::Medium,
};
use thiserror::Error;

use crate::{UserFacingError, spawn_tokio};

/// All errors that can occur while managing the account.
#[derive(Debug, Error)]
pub enum AccountManagementError {
    /// The homeserver wants the account's password before it will do this,
    /// and none was given.
    #[error("the homeserver asks for the account password")]
    NeedsPassword,
    /// The homeserver asked for a stage this core cannot answer.
    #[error("this homeserver asks for a sign-in step this app cannot answer")]
    UnsupportedAuth,
    /// The homeserver rejected the new password as too weak.
    #[error("the password was rejected for being too weak")]
    WeakPassword,
    /// The address is already linked to an account.
    #[error("the address is already linked to an account")]
    ThreepidInUse,
    /// The homeserver does not accept the address.
    #[error("the homeserver does not accept the address")]
    ThreepidDenied,
    /// The homeserver refused.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::Error>),
}

impl UserFacingError for AccountManagementError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::NeedsPassword => "Enter your password to confirm.".to_owned(),
            Self::UnsupportedAuth => {
                "This homeserver asks for a step this app cannot answer yet.".to_owned()
            }
            Self::WeakPassword => "Password rejected for being too weak".to_owned(),
            Self::ThreepidInUse => "This email address is already linked to an account".to_owned(),
            Self::ThreepidDenied => "The homeserver does not accept this email address".to_owned(),
            Self::Server(error) => error.to_string(),
        }
    }
}

impl AccountManagementError {
    /// The error for a homeserver refusal, read for the kinds the
    /// application names.
    fn from_server(error: matrix_sdk::Error) -> Self {
        match error.client_api_error_kind() {
            Some(ErrorKind::WeakPassword) => Self::WeakPassword,
            Some(ErrorKind::ThreepidInUse) => Self::ThreepidInUse,
            Some(ErrorKind::ThreepidDenied) => Self::ThreepidDenied,
            _ => Self::Server(Box::new(error)),
        }
    }
}

/// The medium of a third-party identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThirdPartyMedium {
    /// An email address.
    Email,
    /// A phone number.
    Phone,
}

impl From<ThirdPartyMedium> for Medium {
    fn from(medium: ThirdPartyMedium) -> Self {
        match medium {
            ThirdPartyMedium::Email => Medium::Email,
            ThirdPartyMedium::Phone => Medium::Msisdn,
        }
    }
}

/// A third-party identifier linked to the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirdPartyId {
    /// The address.
    pub address: String,
    /// What kind of address it is.
    pub medium: ThirdPartyMedium,
}

/// The third-party identifiers on the account, and whether the homeserver
/// lets them change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirdPartyIds {
    /// The identifiers.
    pub ids: Vec<ThirdPartyId>,
    /// Whether the homeserver's capabilities allow adding and removing
    /// them.
    pub can_change: bool,
}

/// An email address whose validation link was sent, waiting to be added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEmail {
    /// The address.
    pub address: String,
    /// The secret this validation session was opened with.
    pub client_secret: OwnedClientSecret,
    /// The validation session on the homeserver.
    pub sid: OwnedSessionId,
    /// How many times the email was requested for this address.
    pub send_attempt: u32,
}

/// Make the given request, answering the homeserver's password stage with
/// the given password if it asks for one.
async fn with_password_auth<T, F, Fut>(
    user_id: String,
    password: Option<String>,
    request: F,
) -> Result<T, AccountManagementError>
where
    F: Fn(Option<AuthData>) -> Fut,
    Fut: Future<Output = matrix_sdk::Result<T>>,
{
    use ruma::api::client::uiaa::{MatrixUserIdentifier, Password};

    match request(None).await {
        Ok(value) => Ok(value),
        Err(error) => {
            let Some(info) = error.as_uiaa_response() else {
                return Err(AccountManagementError::from_server(error));
            };
            // The stage list says what this homeserver will take. Only the
            // password stage can be answered here.
            let wants_password = info.flows.iter().any(|flow| {
                flow.stages
                    .iter()
                    .any(|stage| stage.as_str() == "m.login.password")
            });
            if !wants_password {
                return Err(AccountManagementError::UnsupportedAuth);
            }
            let Some(password) = password else {
                return Err(AccountManagementError::NeedsPassword);
            };

            let auth = AuthData::Password(ruma::assign!(
                Password::new(MatrixUserIdentifier::new(user_id).into(), password),
                { session: info.session.clone() }
            ));
            request(Some(auth))
                .await
                .map_err(AccountManagementError::from_server)
        }
    }
}

impl super::Session {
    /// Change the account's password to the given one.
    ///
    /// `current_password` answers the homeserver's password stage.
    pub async fn change_password(
        &self,
        new_password: &str,
        current_password: Option<&str>,
    ) -> Result<(), AccountManagementError> {
        let client = self.client();
        let user_id = self.user_id().to_string();
        let new_password = new_password.to_owned();
        let current_password = current_password.map(ToOwned::to_owned);

        spawn_tokio!(async move {
            with_password_auth(user_id, current_password, |auth| {
                let client = client.clone();
                let new_password = new_password.clone();
                async move { client.account().change_password(&new_password, auth).await }
            })
            .await
            .map(|_| ())
        })
        .await
        .expect("task was not aborted")
    }

    /// Deactivate the account, keeping its messages: the application's
    /// deactivation does not erase them either.
    ///
    /// The session's local data is the caller's to clean up afterwards;
    /// the homeserver has already forgotten the access token.
    pub async fn deactivate_account(
        &self,
        current_password: Option<&str>,
    ) -> Result<(), AccountManagementError> {
        let client = self.client();
        let user_id = self.user_id().to_string();
        let current_password = current_password.map(ToOwned::to_owned);

        spawn_tokio!(async move {
            with_password_auth(user_id, current_password, |auth| {
                let client = client.clone();
                async move { client.account().deactivate(None, auth, false).await }
            })
            .await
            .map(|_| ())
        })
        .await
        .expect("task was not aborted")
    }

    /// The third-party identifiers on the account.
    pub async fn third_party_ids(&self) -> Result<ThirdPartyIds, AccountManagementError> {
        let client = self.client();

        spawn_tokio!(async move {
            let can_change = client
                .homeserver_capabilities()
                .can_change_thirdparty_ids()
                .await
                .unwrap_or(true);
            let response = client
                .account()
                .get_3pids()
                .await
                .map_err(AccountManagementError::from_server)?;

            let ids = response
                .threepids
                .into_iter()
                .filter_map(|id| {
                    let medium = match id.medium {
                        Medium::Email => ThirdPartyMedium::Email,
                        Medium::Msisdn => ThirdPartyMedium::Phone,
                        _ => return None,
                    };
                    Some(ThirdPartyId {
                        address: id.address,
                        medium,
                    })
                })
                .collect();

            Ok(ThirdPartyIds { ids, can_change })
        })
        .await
        .expect("task was not aborted")
    }

    /// Remove the given third-party identifier from the account.
    pub async fn delete_third_party_id(
        &self,
        address: &str,
        medium: ThirdPartyMedium,
    ) -> Result<(), AccountManagementError> {
        let client = self.client();
        let address = address.to_owned();

        spawn_tokio!(async move {
            client
                .account()
                .delete_3pid(&address, medium.into(), None)
                .await
                .map(|_| ())
                .map_err(AccountManagementError::from_server)
        })
        .await
        .expect("task was not aborted")
    }

    /// Ask the homeserver to send a validation link to the given email
    /// address, so it can be added to the account.
    ///
    /// Asking again for the same address continues the same validation
    /// session with the next send attempt, so the homeserver resends
    /// rather than opening another.
    pub async fn request_email_validation(
        &self,
        address: &str,
        previous: Option<&PendingEmail>,
    ) -> Result<PendingEmail, AccountManagementError> {
        let client = self.client();
        let address = address.to_owned();
        let (client_secret, send_attempt) = match previous {
            Some(pending) if pending.address == address => {
                (pending.client_secret.clone(), pending.send_attempt + 1)
            }
            _ => (ruma::ClientSecret::new(), 1),
        };

        spawn_tokio!(async move {
            let response = client
                .account()
                .request_3pid_email_token(&client_secret, &address, UInt::from(send_attempt))
                .await
                .map_err(AccountManagementError::from_server)?;

            Ok(PendingEmail {
                address,
                client_secret,
                sid: response.sid,
                send_attempt,
            })
        })
        .await
        .expect("task was not aborted")
    }

    /// Add the pending email address to the account, once its validation
    /// link was opened.
    ///
    /// `current_password` answers the homeserver's password stage.
    pub async fn add_pending_email(
        &self,
        pending: &PendingEmail,
        current_password: Option<&str>,
    ) -> Result<(), AccountManagementError> {
        let client = self.client();
        let user_id = self.user_id().to_string();
        let client_secret = pending.client_secret.clone();
        let sid = pending.sid.clone();
        let current_password = current_password.map(ToOwned::to_owned);

        spawn_tokio!(async move {
            with_password_auth(user_id, current_password, |auth| {
                let client = client.clone();
                let client_secret = client_secret.clone();
                let sid = sid.clone();
                async move { client.account().add_3pid(&client_secret, &sid, auth).await }
            })
            .await
            .map(|_| ())
        })
        .await
        .expect("task was not aborted")
    }
}

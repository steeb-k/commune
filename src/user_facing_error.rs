use std::time::{Duration, SystemTime};

use gettextrs::gettext;
use matrix_sdk::{ClientBuildError, Error, HttpError, RumaApiError};
use ruma::api::{
    client::error::{
        Error as ClientApiError, ErrorBody,
        ErrorKind::{Forbidden, LimitExceeded, UserDeactivated},
        RetryAfter,
    },
    error::FromHttpResponseError,
};

use crate::ngettext_f;

pub trait UserFacingError {
    fn to_user_facing(&self) -> String;
}

impl UserFacingError for HttpError {
    fn to_user_facing(&self) -> String {
        match self {
            HttpError::Reqwest(error) => {
                // TODO: Add more information based on the error
                if error.is_timeout() {
                    gettext("The connection timed out. Try again later.")
                } else {
                    gettext("Could not connect to the homeserver.")
                }
            }
            HttpError::Api(FromHttpResponseError::Server(RumaApiError::ClientApi(
                ClientApiError {
                    body: ErrorBody::Standard { kind, message },
                    ..
                },
            ))) => {
                match kind {
                    Forbidden { .. } => gettext("The provided username or password is invalid."),
                    UserDeactivated => gettext("The account is deactivated."),
                    LimitExceeded { retry_after } => {
                        if let Some(retry_after) = retry_after {
                            let duration = match retry_after {
                                RetryAfter::Delay(duration) => *duration,
                                RetryAfter::DateTime(until) => until
                                    .duration_since(SystemTime::now())
                                    // An error means that the date provided is in the past, which
                                    // doesn't make sense. Let's not panic anyway and default to 1
                                    // second.
                                    .unwrap_or_else(|_| Duration::from_secs(1)),
                            };
                            let secs = duration.as_secs() as u32;
                            ngettext_f(
                                // Translators: Do NOT translate the content between '{' and '}',
                                // this is a variable name.
                                "You exceeded the homeserver’s rate limit, retry in 1 second.",
                                "You exceeded the homeserver’s rate limit, retry in {n} seconds.",
                                secs,
                                &[("n", &secs.to_string())],
                            )
                        } else {
                            gettext("You exceeded the homeserver’s rate limit, try again later.")
                        }
                    }
                    _ => {
                        // TODO: The server may not give us pretty enough error message. We should
                        // add our own error message.
                        message.clone()
                    }
                }
            }
            _ => gettext("An unexpected connection error occurred."),
        }
    }
}

impl UserFacingError for Error {
    fn to_user_facing(&self) -> String {
        match self {
            Error::DecryptorError(_) => gettext("Could not decrypt the event."),
            Error::Http(http_error) => http_error.to_user_facing(),
            _ => gettext("An unexpected error occurred."),
        }
    }
}

impl UserFacingError for ClientBuildError {
    fn to_user_facing(&self) -> String {
        match self {
            ClientBuildError::Url(_) => gettext("This is not a valid URL."),
            ClientBuildError::AutoDiscovery(_) => {
                gettext("Homeserver auto-discovery failed. Try entering the full URL manually.")
            }
            ClientBuildError::Http(err) => err.to_user_facing(),
            ClientBuildError::SqliteStore(_) => gettext("Could not open the store."),
            _ => gettext("An unexpected error occurred."),
        }
    }
}

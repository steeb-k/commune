use matrix_sdk::{
    ruma::api::{
        client::error::ErrorKind::{Forbidden, LimitExceeded, UserDeactivated},
        error::{FromHttpResponseError, ServerError},
    },
    Error, HttpError,
};

use gettextrs::gettext;

pub trait UserFacingMatrixError {
    fn to_user_facing(self) -> String;
}

impl UserFacingMatrixError for HttpError {
    fn to_user_facing(self) -> String {
        match self {
            HttpError::Reqwest(_) => {
                // TODO: Add more information based on the error
                gettext("Unable to connect to the homeserver.")
            }
            HttpError::ClientApi(FromHttpResponseError::Http(ServerError::Known(error))) => {
                match error.kind {
                    Forbidden => gettext("The provided username or password is invalid."),
                    UserDeactivated => gettext("The user is deactivated."),
                    LimitExceeded { retry_after_ms } => {
                        if let Some(ms) = retry_after_ms {
                            gettext(format!(
                                "You exceeded the homeservers rate limit, retry in {} seconds.",
                                ms.as_secs()
                            ))
                        } else {
                            gettext("You exceeded the homeservers rate limit, try again later.")
                        }
                    }
                    _ => {
                        // TODO: The server may not give us pretty enough error message. We should add our own error message.
                        error.message
                    }
                }
            }
            _ => gettext("An unknown connection error occurred."),
        }
    }
}

impl UserFacingMatrixError for Error {
    fn to_user_facing(self) -> String {
        match self {
            Error::Http(http_error) => http_error.to_user_facing(),
            _ => gettext("An unknown error occurred."),
        }
    }
}

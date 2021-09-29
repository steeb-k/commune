use matrix_sdk::{
    ruma::api::error::{FromHttpResponseError, ServerError},
    HttpError,
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
                gettext("Couldn't connect to the server.")
            }
            HttpError::ClientApi(FromHttpResponseError::Http(ServerError::Known(error))) => {
                // TODO: The server may not give us pretty enough error message. We should add our own error message.
                error.message
            }
            _ => gettext("An Unknown error occurred."),
        }
    }
}

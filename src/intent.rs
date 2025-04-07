use std::borrow::Cow;

use gtk::{glib, prelude::*};
use ruma::OwnedUserId;

use crate::{session::model::VerificationKey, utils::matrix::MatrixIdUri};

/// Intents that can be handled by a session.
///
/// It cannot be cloned intentionnally, so it is handled only once.
#[derive(Debug)]
pub(crate) enum SessionIntent {
    /// Show the target of a Matrix ID URI.
    ShowMatrixId(MatrixIdUri),
    /// Show an ongoing identity verification.
    ShowIdentityVerification(VerificationKey),
}

impl SessionIntent {
    /// Get the application action name for this session intent type.
    pub(crate) fn app_action_name(&self) -> &'static str {
        match self {
            SessionIntent::ShowMatrixId(_) => "app.show-matrix-id",
            SessionIntent::ShowIdentityVerification(_) => "app.show-identity-verification",
        }
    }

    /// Convert this intent to a `GVariant` with the given session ID.
    pub(crate) fn to_variant_with_session_id(&self, session_id: &str) -> glib::Variant {
        let payload = match self {
            Self::ShowMatrixId(uri) => uri.to_variant(),
            Self::ShowIdentityVerification(key) => key.to_variant(),
        };
        (session_id, payload).to_variant()
    }

    /// Convert a `GVariant` to a `SessionIntent` and session ID, given the
    /// intent type.
    ///
    /// Returns an  `(session_id, intent)` tuple on success. Returns `None` if
    /// the payload could not be parsed successfully.
    pub(crate) fn from_variant_with_session_id(
        intent_type: SessionIntentType,
        variant: &glib::Variant,
    ) -> Option<(String, Self)> {
        let (session_id, payload) = variant.get::<(String, glib::Variant)>()?;

        let intent = match intent_type {
            SessionIntentType::ShowMatrixId => Self::ShowMatrixId(payload.get::<MatrixIdUri>()?),
            SessionIntentType::ShowIdentityVerification => {
                let (user_id_str, flow_id) = payload.get::<(String, String)>()?;
                let user_id = OwnedUserId::try_from(user_id_str).ok()?;
                Self::ShowIdentityVerification(VerificationKey { user_id, flow_id })
            }
        };

        Some((session_id, intent))
    }
}

impl From<MatrixIdUri> for SessionIntent {
    fn from(value: MatrixIdUri) -> Self {
        Self::ShowMatrixId(value)
    }
}

impl From<VerificationKey> for SessionIntent {
    fn from(value: VerificationKey) -> Self {
        Self::ShowIdentityVerification(value)
    }
}

/// The type of an intent that can be handled by a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionIntentType {
    /// Show the target of a Matrix ID URI.
    ShowMatrixId,
    /// Show an ongoing identity verification.
    ShowIdentityVerification,
}

impl SessionIntentType {
    /// Get the action name for this session intent type.
    pub(crate) fn action_name(self) -> &'static str {
        match self {
            SessionIntentType::ShowMatrixId => "show-matrix-id",
            SessionIntentType::ShowIdentityVerification => "show-identity-verification",
        }
    }
}

impl StaticVariantType for SessionIntentType {
    fn static_variant_type() -> Cow<'static, glib::VariantTy> {
        <(String, glib::Variant)>::static_variant_type()
    }
}

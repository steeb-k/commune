use std::borrow::Cow;

use gtk::{glib, prelude::*};
use ruma::OwnedUserId;

use crate::{session::model::VerificationKey, utils::matrix::MatrixIdUri};

/// An intent that can be handled by a session.
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
    /// Construct a `SessionIntent` from its type and a payload in a `GVariant`.
    ///
    /// Returns the intent on success. Returns `None` if the payload could not
    /// be parsed successfully.
    pub(crate) fn parse(intent_type: SessionIntentType, payload: &glib::Variant) -> Option<Self> {
        let intent = match intent_type {
            SessionIntentType::ShowMatrixId => Self::ShowMatrixId(payload.get::<MatrixIdUri>()?),
            SessionIntentType::ShowIdentityVerification => {
                let (user_id_str, flow_id) = payload.get::<(String, String)>()?;
                let user_id = OwnedUserId::try_from(user_id_str).ok()?;
                Self::ShowIdentityVerification(VerificationKey { user_id, flow_id })
            }
        };

        Some(intent)
    }

    /// Construct a `SessionIntent` from its type and a payload in a `GVariant`
    /// containing a session ID.
    ///
    /// Returns a `(session_id, intent)` tuple on success. Returns `None` if the
    /// payload could not be parsed successfully.
    pub(crate) fn parse_with_session_id(
        intent_type: SessionIntentType,
        payload: Option<&glib::Variant>,
    ) -> Option<(String, Self)> {
        let (session_id, payload) = payload?.get::<(String, glib::Variant)>()?;
        let intent = Self::parse(intent_type, &payload)?;
        Some((session_id, intent))
    }

    /// Convert this intent to a `GVariant` with the given session ID.
    pub(crate) fn to_variant_with_session_id(&self, session_id: &str) -> glib::Variant {
        let payload = self.to_variant();
        (session_id, payload).to_variant()
    }
}

impl ToVariant for SessionIntent {
    fn to_variant(&self) -> glib::Variant {
        match self {
            SessionIntent::ShowMatrixId(matrix_uri) => matrix_uri.to_variant(),
            SessionIntent::ShowIdentityVerification(verification_key) => (
                verification_key.user_id.as_str(),
                verification_key.flow_id.as_str(),
            )
                .to_variant(),
        }
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

    /// Get the application action name for this session intent type.
    pub(crate) fn app_action_name(self) -> &'static str {
        match self {
            SessionIntentType::ShowMatrixId => "app.show-matrix-id",
            SessionIntentType::ShowIdentityVerification => "app.show-identity-verification",
        }
    }
}

impl StaticVariantType for SessionIntentType {
    fn static_variant_type() -> Cow<'static, glib::VariantTy> {
        <(String, glib::Variant)>::static_variant_type()
    }
}

use matrix_sdk::encryption::verification::VerificationRequest;
use ruma::{events::key::verification::VerificationMethod, OwnedUserId};

mod identity_verification;
mod verification_list;

pub use self::{
    identity_verification::{
        IdentityVerification, VerificationState, VerificationSupportedMethods,
    },
    verification_list::VerificationList,
};
use crate::{components::Camera, prelude::*};

/// A unique key to identify an identity verification.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct VerificationKey {
    /// The ID of the user being verified.
    pub user_id: OwnedUserId,
    /// The ID of the verification.
    pub flow_id: String,
}

impl VerificationKey {
    /// Create a new `VerificationKey` with the given user ID and flow ID.
    pub fn new(user_id: OwnedUserId, flow_id: String) -> Self {
        Self { user_id, flow_id }
    }

    /// Create a new `VerificationKey` from the given [`VerificationRequest`].
    pub fn from_request(request: &VerificationRequest) -> Self {
        Self::new(
            request.other_user_id().to_owned(),
            request.flow_id().to_owned(),
        )
    }
}

/// Load the supported verification methods on this system.
async fn load_supported_verification_methods() -> Vec<VerificationMethod> {
    let mut methods = vec![
        VerificationMethod::SasV1,
        VerificationMethod::QrCodeShowV1,
        VerificationMethod::ReciprocateV1,
    ];

    let has_cameras = Camera::has_cameras().await;

    if has_cameras {
        methods.push(VerificationMethod::QrCodeScanV1);
    }

    methods
}

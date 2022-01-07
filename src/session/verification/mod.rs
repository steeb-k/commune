mod identity_verification;
mod verification_list;

pub use self::identity_verification::{
    IdentityVerification, Mode as VerificationMode, SasData, State as VerificationState,
};
pub use self::verification_list::{FlowId, VerificationList};

use std::time::Duration;
/// The time a verification is valid after it's creation.
#[allow(dead_code)]
pub const VERIFICATION_CREATION_TIMEOUT: Duration = Duration::from_secs(60 * 10);
/// The time a verification is valid after it was received by the client.
#[allow(dead_code)]
pub const VERIFICATION_RECEIVE_TIMEOUT: Duration = Duration::from_secs(60 * 2);

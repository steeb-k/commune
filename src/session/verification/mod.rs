mod emoji;
mod identity_verification;
mod incoming_verification;
mod session_verification;
mod verification_list;

pub use self::emoji::Emoji;
pub use self::identity_verification::{IdentityVerification, Mode as VerificationMode, SasData};
pub use self::incoming_verification::IncomingVerification;
pub use self::session_verification::SessionVerification;
pub use self::verification_list::VerificationList;

use std::time::Duration;
/// The time a verification is valid after it's creation.
pub const VERIFICATION_CREATION_TIMEOUT: Duration = Duration::from_secs(60 * 10);
/// The time a verification is valid after it was received by the client.
#[allow(dead_code)]
pub const VERIFICATION_RECEIVE_TIMEOUT: Duration = Duration::from_secs(60 * 2);

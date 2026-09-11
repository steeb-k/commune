//! The public half of the key that signs the release feed.
//!
//! The feed is a small JSON document saying which version is current and
//! where to download it (see [`super`]). Every artifact it points at is
//! already signed by the platform — Authenticode on Windows, Developer ID on
//! macOS, the APK signing key on Android — and none of those signatures can
//! be forged by anyone who merely controls the feed. What the platform
//! signatures cannot say is *which* correctly signed build is the current
//! one, so a feed that could be rewritten in transit could hold an
//! installation at a version whose bugs the attacker knows. This signature
//! is what closes that: the manifest is signed by the release workflow and
//! checked here before a single field of it is read.
//!
//! The private half was generated on 11 September 2026 and lives only in the
//! repository's `UPDATE_FEED_PRIVATE_KEY` secret and in the maintainer's
//! offline backup. It is never in this tree, and no build needs it.
//!
//! # Rotating it
//!
//! An installation trusts exactly the key it was built with, so a rotation
//! only reaches builds published after it. The sequence that does not strand
//! anybody is: add the new key here as a second accepted key, release,
//! wait until the old builds are gone, then remove the first. Replacing the
//! constant in one commit strands every installation that has not already
//! updated — they will go on checking a feed they can no longer verify, and
//! report that the check failed, forever. `VERIFYING_KEYS` is a list for
//! that reason.

/// The keys a release manifest's signature is accepted from, newest first.
///
/// More than one entry means a rotation is in flight; see the module docs
/// for the order that does not strand old installations.
pub(super) const VERIFYING_KEYS: &[[u8; 32]] = &[[
    0x74, 0x07, 0xbc, 0xa0, 0xef, 0xb4, 0x11, 0xa3, 0x7b, 0x16, 0xfe, 0xca, 0xcd, 0x2f, 0xe9, 0x10,
    0x3f, 0x55, 0x96, 0x71, 0x4d, 0x57, 0xe6, 0x30, 0x22, 0x97, 0x95, 0x14, 0x2d, 0xcb, 0x86, 0x03,
]];

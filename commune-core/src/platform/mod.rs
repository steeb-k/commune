//! Per-platform plumbing that has no business being anywhere else.

#[cfg(target_os = "android")]
pub mod android;

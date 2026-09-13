//! A small HTTP client for the few things that are fetched outside Matrix.
//!
//! Everything the application downloads normally goes through the homeserver,
//! which is what makes it safe: the homeserver enforces a size limit and the
//! remote host never learns anything about the user. This module exists for
//! the cases where that is not true — currently only the GIF search — so that
//! the size limit at least is still enforced somewhere.

use std::sync::LazyLock;

use futures_util::StreamExt;
use matrix_sdk::reqwest;

use crate::tls;

/// The HTTP client shared by everything that talks to a non-Matrix host.
pub static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    // Through `tls`, because `reqwest`'s own default does not work on Android.
    // See `crate::tls`.
    tls::client_builder()
        .user_agent(concat!("Commune/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("HTTP client should be constructible with the default TLS backend")
});

/// An error that occurred while fetching a URL.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// The request could not be sent, or the connection failed before a
    /// response came back — not the server answering with an error status;
    /// that is [`Self::Status`].
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    /// The server answered, but with an error status.
    ///
    /// Kept apart from [`Self::Request`] — instead of folding this into the
    /// `reqwest::Error` that `error_for_status()` would produce — so that a
    /// caller can tell "the server is unreachable" from "the server said no"
    /// (a 404 in particular), and so that the distinction can be constructed
    /// directly in a test: a `reqwest::Error` has no public constructor, but
    /// a `StatusCode` does.
    #[error("The server responded with {0}")]
    Status(reqwest::StatusCode),
    /// The response is bigger than the caller is willing to read.
    #[error("The response is larger than the {0} byte limit")]
    TooLarge(u64),
}

/// Download the body at the given URL, reading at most `max_size` bytes.
///
/// Must be called from the tokio runtime.
pub async fn fetch(url: &str, max_size: u64) -> Result<Vec<u8>, HttpError> {
    let response = CLIENT.get(url).send().await?;

    if let Some(status) = response
        .error_for_status_ref()
        .err()
        .and_then(|error| error.status())
    {
        return Err(HttpError::Status(status));
    }

    // Trust the advertised length only to refuse early; it is not authoritative,
    // so the body is counted as it arrives too.
    if response.content_length().is_some_and(|len| len > max_size) {
        return Err(HttpError::TooLarge(max_size));
    }

    let mut data = Vec::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;

        if data.len() as u64 + chunk.len() as u64 > max_size {
            return Err(HttpError::TooLarge(max_size));
        }

        data.extend_from_slice(&chunk);
    }

    Ok(data)
}

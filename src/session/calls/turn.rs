use std::time::{Duration, Instant};

use matrix_sdk::Client as MatrixClient;
use ruma::api::client::voip::get_turn_server_info;
use tracing::{debug, warn};

use crate::spawn_tokio;

/// How long before a credential expires we ask for a new one.
///
/// The homeserver signs a credential that is valid for a fixed time, and a call
/// that outlives it keeps using it — the relay only checks at allocation time.
/// This margin is about not starting a call with a credential that is about to
/// go stale.
// `Duration::from_mins` would say this better and is not stable yet.
#[allow(clippy::duration_suboptimal_units)]
const REFRESH_MARGIN: Duration = Duration::from_secs(60);

/// A TURN server to offer to `webrtcbin`.
#[derive(Debug, Clone)]
pub(crate) struct TurnServer {
    /// The URI, in the form `turn://user:password@host:port`.
    ///
    /// This is `webrtcbin`'s spelling, not the one the homeserver uses.
    pub(crate) uri: String,
}

/// The TURN credentials for a session.
///
/// The homeserver hands out a username and password that are valid for a while
/// and are not tied to any particular call, so they are fetched once and
/// shared.
#[derive(Debug, Default)]
pub(crate) struct TurnCredentials {
    /// The servers, already in `webrtcbin`'s spelling.
    servers: Vec<TurnServer>,
    /// When the credentials stop being usable.
    expires_at: Option<Instant>,
}

impl TurnCredentials {
    /// Whether these credentials can still be used.
    pub(crate) fn is_fresh(&self) -> bool {
        self.expires_at
            .is_some_and(|expires_at| Instant::now() + REFRESH_MARGIN < expires_at)
    }

    /// The servers to hand to `webrtcbin`.
    pub(crate) fn servers(&self) -> &[TurnServer] {
        &self.servers
    }
}

/// Ask the homeserver where to find a TURN server.
///
/// A homeserver is not required to run one, and answers `M_NOT_FOUND` or an
/// empty list when it does not. That is not an error: a call between two people
/// who can reach each other directly works without a relay, and the only way to
/// find out is to try.
pub(crate) async fn load_turn_credentials(client: &MatrixClient) -> TurnCredentials {
    let client = client.clone();
    let handle =
        spawn_tokio!(async move { client.send(get_turn_server_info::v3::Request::new()).await });

    let response = match handle.await.expect("task was not aborted") {
        Ok(response) => response,
        Err(error) => {
            // Not an error in itself — a homeserver need not run one — but it
            // decides whether anybody behind a NAT can be called at all, so it
            // is worth saying out loud rather than at debug.
            warn!("The homeserver offered no TURN server: {error}");
            return TurnCredentials::default();
        }
    };

    let servers = response
        .uris
        .iter()
        .filter_map(|uri| {
            let server = webrtcbin_turn_uri(uri, &response.username, &response.password);

            if server.is_none() {
                warn!("Ignoring TURN URI that we cannot use: {uri}");
            }

            server
        })
        .map(|uri| TurnServer { uri })
        .collect::<Vec<_>>();

    // The URIs carry no credentials — those are separate fields — so they are
    // safe to log, and worth logging: a `turn_uris` pointing at a LAN address
    // looks exactly like a working one until somebody calls in from outside.
    debug!(
        "The homeserver offered {} TURN URI(s), {} of them usable: {:?}",
        response.uris.len(),
        servers.len(),
        response.uris
    );

    TurnCredentials {
        servers,
        expires_at: Some(Instant::now() + response.ttl),
    }
}

/// Convert a TURN URI from the homeserver into the form `webrtcbin` wants.
///
/// The homeserver speaks RFC 7065 — `turn:host:port?transport=udp` — with the
/// credentials alongside. `webrtcbin` wants them inside the URI, as
/// `turn://user:password@host:port`. The two are not the same grammar, and the
/// conversion is not just string concatenation: Synapse's username is a
/// timestamp joined to a full user ID, so it carries both `:` and `@` and has
/// to be percent-encoded or the authority is unparsable.
///
/// `turns:` maps to `turns://`. Anything else is refused rather than guessed
/// at.
fn webrtcbin_turn_uri(uri: &str, username: &str, password: &str) -> Option<String> {
    let (scheme, rest) = uri.split_once(':')?;

    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "turn" => "turn",
        "turns" => "turns",
        // `stun:` is handled separately: it takes no credentials.
        _ => return None,
    };

    if rest.is_empty() {
        return None;
    }

    Some(format!(
        "{scheme}://{}:{}@{rest}",
        percent_encode_userinfo(username),
        percent_encode_userinfo(password),
    ))
}

/// Percent-encode a string so that it can sit in the userinfo of a URI.
///
/// The unreserved set of RFC 3986 is kept and everything else is escaped, which
/// is more than strictly required — the userinfo grammar also admits
/// sub-delimiters — but a password is opaque and there is nothing to gain from
/// leaving any of it bare.
fn percent_encode_userinfo(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());

    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synapse_credentials_survive_the_move_into_the_authority() {
        // Synapse's username is a timestamp joined to a user ID, so it carries
        // the two characters that would otherwise end the authority early.
        let uri = webrtcbin_turn_uri(
            "turn:127.0.0.1:3478?transport=udp",
            "1787551090:@alice:localhost",
            "qU1Sm5pPc6bk0IH35F7QJKvG1hs=",
        )
        .unwrap();

        assert_eq!(
            uri,
            "turn://1787551090%3A%40alice%3Alocalhost:qU1Sm5pPc6bk0IH35F7QJKvG1hs%3D@127.0.0.1:3478?transport=udp"
        );
    }

    #[test]
    fn turns_is_kept_and_anything_else_is_refused() {
        assert!(
            webrtcbin_turn_uri("turns:example.org:5349", "u", "p")
                .unwrap()
                .starts_with("turns://")
        );
        assert!(webrtcbin_turn_uri("stun:example.org:3478", "u", "p").is_none());
        assert!(webrtcbin_turn_uri("https://example.org", "u", "p").is_none());
        assert!(webrtcbin_turn_uri("turn:", "u", "p").is_none());
    }
}

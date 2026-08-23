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
    /// How the URI says to reach it, for the log and for the ordering.
    ///
    /// The credentials are in the URI, so this is the only part of it that can
    /// be logged.
    pub(crate) transport: String,
}

/// How much we would rather have a given transport, lowest first.
///
/// Measured on 23 August 2026: given three TURN servers, `webrtcbin` gathers a
/// relay candidate from **one** of them. Handed the homeserver's list in order
/// it took the first; handed it reversed it took the last-but-one. So the order
/// decides which relay a call gets, and UDP is the one to want — a relay
/// reached over TCP carries every packet of the call through a stream socket,
/// with the head-of-line blocking that implies.
fn transport_rank(uri: &str) -> u8 {
    let transport = uri.rsplit("transport=").next().unwrap_or("");

    match (uri.starts_with("turns:"), transport) {
        (false, "udp") => 0,
        (false, _) => 1,
        (true, _) => 2,
    }
}

/// The transport a URI names, for the log.
fn transport_of(uri: &str) -> String {
    let transport = uri
        .split_once("transport=")
        .map_or("unspecified", |(_, rest)| {
            rest.split('&').next().unwrap_or("unspecified")
        });

    if uri.starts_with("turns:") {
        format!("{transport} with TLS")
    } else {
        transport.to_owned()
    }
}

/// The ICE servers a call should be given.
///
/// Two different things with two different jobs. A TURN server carries the
/// call when nothing else can and needs credentials to do it; a STUN server
/// only answers "what address did this packet come from", takes no
/// credentials, and is how a client behind a NAT learns of an address the
/// other end can reach it on.
#[derive(Debug, Clone, Default)]
pub(crate) struct IceServers {
    /// The STUN server, if the homeserver named one.
    ///
    /// `webrtcbin` takes a single one, so the first usable is the one used.
    pub(crate) stun: Option<String>,
    /// The TURN servers, in the order they should be offered.
    pub(crate) turn: Vec<TurnServer>,
}

impl IceServers {
    /// Whether there is nothing here to help a call across a NAT.
    pub(crate) fn is_empty(&self) -> bool {
        self.stun.is_none() && self.turn.is_empty()
    }
}

/// The TURN credentials for a session.
///
/// The homeserver hands out a username and password that are valid for a while
/// and are not tied to any particular call, so they are fetched once and
/// shared.
#[derive(Debug, Default)]
pub(crate) struct TurnCredentials {
    /// The servers, already in `webrtcbin`'s spelling.
    servers: IceServers,
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
    pub(crate) fn servers(&self) -> &IceServers {
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

    // Sorted, because only the first of them is going to be used, and a call
    // relayed over UDP is the one to have.
    let mut uris = response.uris.clone();
    uris.sort_by_key(|uri| transport_rank(uri));

    let stun = uris.iter().find_map(|uri| webrtcbin_stun_uri(uri));

    let turn = uris
        .iter()
        .filter(|uri| !is_stun(uri))
        .filter_map(|uri| {
            let server = webrtcbin_turn_uri(uri, &response.username, &response.password);

            if server.is_none() {
                warn!("Ignoring TURN URI that we cannot use: {uri}");
            }

            server.map(|server| TurnServer {
                uri: server,
                transport: transport_of(uri),
            })
        })
        .collect::<Vec<_>>();

    let servers = IceServers { stun, turn };

    // The URIs carry no credentials — those are separate fields — so they are
    // safe to log, and worth logging: a `turn_uris` pointing at a LAN address
    // looks exactly like a working one until somebody calls in from outside.
    debug!(
        "The homeserver offered {} ICE URI(s), {} TURN server(s) usable in the \
         order they will be offered and {} for STUN: {:?}",
        response.uris.len(),
        servers.turn.len(),
        servers.stun.as_deref().unwrap_or("none"),
        uris
    );

    TurnCredentials {
        servers,
        expires_at: Some(Instant::now() + response.ttl),
    }
}

/// Whether a URI names a STUN server rather than a TURN one.
fn is_stun(uri: &str) -> bool {
    let scheme = uri.split(':').next().unwrap_or_default();

    scheme.eq_ignore_ascii_case("stun") || scheme.eq_ignore_ascii_case("stuns")
}

/// Convert a STUN URI from the homeserver into the form `webrtcbin` wants.
///
/// The homeserver speaks RFC 7064 — `stun:host:port` — and `webrtcbin` wants
/// `stun://host:port`. There are no credentials in either: a STUN server is
/// asked one question, "what address did this packet come from", and needs to
/// know nothing about who is asking.
///
/// **This client names no STUN server of its own, and that has not changed.**
/// Pointing every call at somebody else's server to learn our own address
/// tells that third party a call is happening; the homeserver's operator
/// choosing to name one is a different thing entirely, and refusing to use it
/// only meant a client behind a NAT went without a reflexive candidate — which
/// on a network whose TURN server sits inside the same NAT leaves it with no
/// address the other end can reach at all.
///
/// `stuns:` is refused: it is STUN over TLS, and `webrtcbin` has no spelling
/// for it.
fn webrtcbin_stun_uri(uri: &str) -> Option<String> {
    let (scheme, rest) = uri.split_once(':')?;

    if !scheme.eq_ignore_ascii_case("stun") {
        if scheme.eq_ignore_ascii_case("stuns") {
            warn!("Ignoring a STUN URI over TLS, which webrtcbin cannot be told about: {uri}");
        }
        return None;
    }

    if rest.is_empty() {
        return None;
    }

    // A query string is RFC 7064's `?transport=`, which a STUN URI has no use
    // for and `webrtcbin` does not parse.
    let rest = rest.split('?').next().unwrap_or(rest);

    Some(format!("stun://{rest}"))
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
        // `stun:` is handled by `webrtcbin_stun_uri`: it takes no credentials.
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
    fn a_stun_uri_the_homeserver_names_is_used() {
        assert_eq!(
            webrtcbin_stun_uri("stun:stun.example.org:3478").as_deref(),
            Some("stun://stun.example.org:3478")
        );

        // RFC 7064 allows a query a STUN URI has no use for.
        assert_eq!(
            webrtcbin_stun_uri("stun:stun.example.org:3478?transport=udp").as_deref(),
            Some("stun://stun.example.org:3478")
        );

        // Over TLS there is nothing to hand `webrtcbin`, and a TURN URI is a
        // different function's business.
        assert!(webrtcbin_stun_uri("stuns:stun.example.org:5349").is_none());
        assert!(webrtcbin_stun_uri("turn:turn.example.org:3478").is_none());
        assert!(webrtcbin_stun_uri("stun:").is_none());
    }

    #[test]
    fn stun_and_turn_uris_are_told_apart() {
        assert!(is_stun("stun:stun.example.org:3478"));
        assert!(is_stun("STUNS:stun.example.org:5349"));
        assert!(!is_stun("turn:turn.example.org:3478?transport=udp"));
    }

    #[test]
    fn udp_is_offered_before_tcp_and_tls() {
        let mut uris = [
            "turns:turn.example.org:5349?transport=tcp".to_owned(),
            "turn:turn.example.org:3478?transport=tcp".to_owned(),
            "turn:turn.example.org:3478?transport=udp".to_owned(),
        ];
        uris.sort_by_key(|uri| transport_rank(uri));

        // Only the first is used, so it has to be the one worth having.
        assert!(uris[0].ends_with("transport=udp"));
        assert!(uris[2].starts_with("turns:"));
    }

    #[test]
    fn the_transport_is_named_for_the_log() {
        assert_eq!(transport_of("turn:h:3478?transport=udp"), "udp");
        assert_eq!(transport_of("turns:h:5349?transport=tcp"), "tcp with TLS");
        assert_eq!(transport_of("turn:h:3478"), "unspecified");
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

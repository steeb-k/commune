//! Where TLS trust roots come from.
//!
//! Everywhere except Android this is `reqwest`'s business and this module does
//! nothing: it hands back a plain builder and the platform verifier that
//! `reqwest` selects takes care of it.
//!
//! # Why Android is different
//!
//! `reqwest`'s `rustls` feature enables `rustls-platform-verifier`
//! unconditionally — there is no root-store feature to choose instead — and on
//! Android that crate refuses to work until it has been initialized with a JVM
//! handle and a `Context`:
//!
//! ```text
//! panicked at rustls-platform-verifier-0.7.0/src/android.rs:90:10:
//! Expect rustls-platform-verifier to be initialized
//! ```
//!
//! It panics rather than returning an error, and it does so inside whichever
//! tokio task was making the request, so the visible symptom is a request that
//! never finishes. It also cannot simply be initialized: verification calls
//! into a Kotlin class, `org.rustls.platformverifier.CertificateVerifier`,
//! which has to be built into the APK. That component is
//! [not published to Maven][gh115] and is not in the published crate either —
//! 0.7.0 ships `src/`, `examples/` and licences and nothing else — so using it
//! means vendoring the component out of the crate's git repository and teaching
//! pixiewood's generated Gradle project to find it.
//!
//! [gh115]: https://github.com/rustls/rustls-platform-verifier/issues/115
//!
//! # What this does instead
//!
//! Android keeps its system trust store as ordinary PEM files in
//! [`ANDROID_CA_DIR`], readable by any application, so the roots are loaded
//! from there and given to `rustls` directly. The verifier is still linked; it
//! is simply never asked anything.
//!
//! What that buys: the roots are the device's own, so they follow system
//! updates rather than whatever was vendored on the day the app was built.
//!
//! What it does not buy, and these are real:
//!
//! * **`rustls` does the verifying, not Android.** Certificate transparency
//!   policy, operator-configured pinning and the per-app network security
//!   config are Android's, and none of them apply here. `rustls`'s own path
//!   building and hostname checking still do.
//! * **User-installed CAs are ignored.** They live in
//!   `/data/misc/user/0/cacerts-added`, which an application cannot read.
//!   Android has not trusted them for applications by default since Nougat, so
//!   this matches the platform default rather than departing from it — but it
//!   does mean an intercepting proxy will not work without more than a
//!   certificate.
//!
//! Replacing this with the real verifier is the right end state and needs the
//! Kotlin component; see `doc/android.md`.

use matrix_sdk::reqwest;

/// The directory holding Android's system trust store, as hashed PEM files.
#[cfg(target_os = "android")]
pub(crate) const ANDROID_CA_DIR: &str = "/system/etc/security/cacerts";

/// A `reqwest` client builder that will trust the right roots.
#[cfg(not(target_os = "android"))]
pub(crate) fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
}

/// A `reqwest` client builder that will trust the right roots.
#[cfg(target_os = "android")]
pub(crate) fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().use_preconfigured_tls(self::android::config().clone())
}

/// The client `matrix-sdk` should use, so that it does not build its own with
/// the platform verifier in it.
///
/// # Panics
///
/// If a `reqwest` client cannot be constructed at all, which would mean the TLS
/// backend is unusable and nothing this application does would work.
pub(crate) fn matrix_client() -> reqwest::Client {
    client_builder()
        .build()
        .expect("HTTP client should be constructible")
}

#[cfg(target_os = "android")]
mod android {
    use std::{
        fs,
        io::BufReader,
        sync::{Arc, LazyLock},
    };

    use rustls::{ClientConfig, RootCertStore};
    use tracing::{debug, error, warn};

    use super::ANDROID_CA_DIR;

    /// The TLS configuration, built once from the device's trust store.
    ///
    /// Not an `Arc`: `reqwest` downcasts what it is given to
    /// `Option<rustls::ClientConfig>` exactly, and anything else — an `Arc` of
    /// one included — is refused at runtime with "Unknown TLS backend passed to
    /// `use_preconfigured_tls`". So callers get a clone of the config itself,
    /// which is cheap because its expensive parts are already refcounted.
    static CONFIG: LazyLock<ClientConfig> = LazyLock::new(build_config);

    /// The shared TLS configuration.
    pub(super) fn config() -> &'static ClientConfig {
        &CONFIG
    }

    /// Build a TLS configuration trusting the device's system roots.
    fn build_config() -> ClientConfig {
        let roots = load_roots();

        // An empty store is not treated as a reason to give up and trust
        // everything: it means every connection fails to verify, which is
        // noisy, safe, and obvious in the log above.
        if roots.is_empty() {
            error!("No system trust roots were loaded; every TLS connection will fail");
        }

        // The provider is named rather than taken from the process default,
        // which has to have been installed by someone else first and panics if
        // it was not.
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());

        let mut config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("aws-lc-rs should support the default protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth();

        // `reqwest` would negotiate these itself; supplying the configuration
        // means supplying them too, and without them every connection falls
        // back to HTTP/1.1.
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        config
    }

    /// Read every certificate in Android's system trust store.
    fn load_roots() -> RootCertStore {
        let mut roots = RootCertStore::empty();

        let entries = match fs::read_dir(ANDROID_CA_DIR) {
            Ok(entries) => entries,
            Err(error) => {
                error!("Could not read the system trust store at {ANDROID_CA_DIR}: {error}");
                return roots;
            }
        };

        let mut files = 0_usize;
        let mut ignored = 0_usize;

        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    warn!("Ignoring unreadable entry in the system trust store: {error}");
                    continue;
                }
            };

            let file = match fs::File::open(&path) {
                Ok(file) => file,
                Err(error) => {
                    warn!("Could not open {}: {error}", path.display());
                    continue;
                }
            };
            files += 1;

            // Each file holds one PEM certificate followed by a human-readable
            // dump of it, which is not PEM and is skipped by the parser.
            let certs = rustls_pemfile::certs(&mut BufReader::new(file))
                .filter_map(Result::ok)
                .collect::<Vec<_>>();

            let (_, rejected) = roots.add_parsable_certificates(certs);
            ignored += rejected;
        }

        debug!(
            "Loaded {} trust roots from {files} files in {ANDROID_CA_DIR} ({ignored} rejected)",
            roots.len()
        );

        roots
    }
}

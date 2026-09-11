//! Finding out whether a newer release exists, and fetching it.
//!
//! Commune is installed from an artifact on a GitHub release on three of its
//! four platforms — an `.msi`, a `.tar.gz`, an `.apk` — and none of them has
//! anything behind it that would notice a new version. (The fourth, Flatpak,
//! has a whole app store for exactly that, so nothing here runs there.) This
//! module is the part of the answer that is the same everywhere: read a small
//! signed document that says what the current release is, decide whether it
//! is newer than what is running, and put the artifact on disk with its
//! checksum verified. Installing it is what differs per platform, and stays
//! in the embedder — `utils::windows_update` and `utils::macos_update` in the
//! GTK application, `Updates.kt` in the Kotlin one.
//!
//! # Why a static file and not the GitHub API
//!
//! `api.github.com` allows 60 unauthenticated requests an hour **per source
//! address**, which is shared by everyone behind one NAT. An update check is
//! the one request an application makes that every copy of it makes at the
//! same sort of time, so the API is precisely the wrong thing to build it
//! on: it would work in testing and fail in a school, an office or a
//! co-working space. The release workflow writes a small JSON manifest to a
//! branch instead, and `raw.githubusercontent.com` serves it with no quota
//! of that kind.
//!
//! # What is trusted
//!
//! The manifest is signed (see [`key`]) and the signature is checked before
//! any field is read. The artifact it names is checked against the SHA-256
//! digest in the manifest. Neither of those replaces the platform signature
//! on the artifact itself, which is what actually authorises the install and
//! which the platform — not this code — enforces; they stop a rewritten feed
//! from choosing *which* signed build an installation ends up on.

mod key;

use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use ed25519_dalek::{Signature, VerifyingKey};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, info, warn};

use crate::{
    UserFacingError, config,
    http::{self, CLIENT, HttpError},
};

/// Where the release feed is published.
///
/// A directory, not a file: the channel's name and `.json` are appended. The
/// branch is written only by the release workflow and holds nothing else.
const DEFAULT_FEED_BASE: &str = "https://raw.githubusercontent.com/steeb-k/commune/updates";

/// The environment variable that points the feed somewhere else.
///
/// For developing the updater itself, which otherwise cannot be exercised
/// without publishing a release. Any local HTTP server holding a manifest and
/// its signature will do — signed, necessarily, with a key this build trusts,
/// so a test build is also a rebuild of [`key`].
const FEED_BASE_ENV: &str = "COMMUNE_UPDATE_FEED";

/// The largest manifest that will be read.
///
/// The real ones are well under a kilobyte. This is not a tuning parameter:
/// it is the bound that stops a hostile or broken feed from being read into
/// memory until the process dies.
const MAX_MANIFEST_SIZE: u64 = 64 * 1024;

/// The largest signature that will be read. A signature is 64 bytes, spelled
/// as 128 characters of hex; the slack is for a trailing newline.
const MAX_SIGNATURE_SIZE: u64 = 256;

/// The settings key holding the serialized [`UpdateSettings`].
const UPDATE_SETTINGS_KEY: &str = "updates";

/// How often an automatic check is allowed to happen, in seconds.
///
/// The embedder calls [`check()`] whenever it likes — at startup, on a timer,
/// when the user presses a button — and [`UpdateSettings::is_check_due()`] is
/// what keeps the first two from being one request per launch for somebody
/// who restarts the application twenty times a day.
pub const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

/// Which series of releases an installation follows.
///
/// Each channel is a separate manifest, and the workflow decides what goes
/// in which: a stable release is written to both `stable` and `rc`, so that
/// somebody testing release candidates is not left behind when the real
/// release arrives; a release candidate only to `rc`; a nightly only to
/// `nightly`. That way the client does not have to know the policy, only
/// which file to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Tagged releases only. What an installation gets unless it says
    /// otherwise.
    #[default]
    Stable,
    /// Release candidates, and the stable releases that follow them.
    Rc,
    /// Every build of `main`. Only a Devel-profile installation offers it:
    /// a nightly is built with the Devel application id and writes to the
    /// Devel data directory, so it could not replace a stable install even
    /// if it were offered one.
    Nightly,
}

impl Channel {
    /// The name this channel is spelled with, in settings and in the name of
    /// its manifest.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Rc => "rc",
            Self::Nightly => "nightly",
        }
    }

    /// The channel an installation of this build follows unless the user
    /// chooses another.
    ///
    /// A Devel build is a build of `main`, so `main` is what it should
    /// follow; anything else came from a tag.
    #[must_use]
    pub fn default_for_profile() -> Self {
        if config::profile() == "devel" {
            Self::Nightly
        } else {
            Self::Stable
        }
    }

    /// The channels an installation of this build may choose between.
    ///
    /// A stable installation is never offered `Nightly`, because a nightly
    /// is a different application with a different id: choosing it would
    /// install a second copy rather than update this one.
    #[must_use]
    pub fn available_for_profile() -> &'static [Self] {
        if config::profile() == "devel" {
            &[Self::Nightly]
        } else {
            &[Self::Stable, Self::Rc]
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Channel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stable" => Ok(Self::Stable),
            "rc" => Ok(Self::Rc),
            "nightly" => Ok(Self::Nightly),
            _ => Err(()),
        }
    }
}

/// The kind of artifact this build would install to update itself.
///
/// Decided at compile time. It is deliberately not "which operating system
/// is this": two builds for the same OS could want different artifacts, and
/// the manifest keys name the artifact rather than the platform for that
/// reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// The per-user `.msi`.
    WindowsMsi,
    /// The universal `Commune.app`, in a `.tar.gz` — a tarball rather than
    /// the `.dmg` because files extracted from one are not quarantined, and
    /// a quarantined bundle is refused on first launch.
    MacOsTarball,
    /// The `arm64-v8a` `.apk`.
    AndroidApk,
}

impl Platform {
    /// The artifact this build updates itself with, if it can.
    ///
    /// `None` on Linux, where every install came from a package manager or a
    /// Flatpak remote and replacing it from inside the application would be
    /// both impossible and rude.
    #[must_use]
    pub fn current() -> Option<Self> {
        cfg_if::cfg_if! {
            if #[cfg(target_os = "windows")] {
                Some(Self::WindowsMsi)
            } else if #[cfg(target_os = "macos")] {
                Some(Self::MacOsTarball)
            } else if #[cfg(target_os = "android")] {
                Some(Self::AndroidApk)
            } else {
                None
            }
        }
    }

    /// The key this artifact is listed under in a manifest.
    #[must_use]
    pub fn feed_key(self) -> &'static str {
        match self {
            Self::WindowsMsi => "windows-x86_64-msi",
            Self::MacOsTarball => "macos-universal-tar",
            Self::AndroidApk => "android-arm64-apk",
        }
    }
}

/// Whether this process is running inside a Flatpak sandbox.
///
/// The runtime always mounts `/.flatpak-info`, and its presence is the
/// documented way to ask. Nothing else in the tree needed to know until now:
/// a Flatpak is the one packaging of Commune that already has something
/// keeping it current, so it is the one that must not be offered an updater.
#[must_use]
pub fn is_flatpak() -> bool {
    cfg!(target_os = "linux") && Path::new("/.flatpak-info").exists()
}

/// Whether this build can update itself at all.
///
/// False inside Flatpak and on Linux generally. It is still not the whole
/// question — the embedder knows things this crate cannot, such as whether a
/// macOS bundle is ad-hoc signed or a Windows build is running out of its
/// build directory — so the embedder gets the last word.
#[must_use]
pub fn is_supported() -> bool {
    Platform::current().is_some() && !is_flatpak()
}

/// One downloadable artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// Where to fetch it.
    pub url: String,
    /// Its SHA-256 digest, as lowercase hex.
    pub sha256: String,
    /// Its size in bytes. Checked against what arrives, and used to refuse
    /// early rather than after downloading a few hundred megabytes of
    /// something else.
    pub size: u64,
}

/// What a channel's manifest says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// The channel this manifest is for. Checked against the one that was
    /// asked for: a signed manifest is still the wrong manifest if the feed
    /// served `nightly.json` in answer to a request for `stable.json`.
    pub channel: Channel,
    /// The version, in the semver spelling. `RELEASING.md` explains why the
    /// version has three spellings and why this is the one that is compared.
    pub version: String,
    /// The commit count of the build, which is what orders two builds that
    /// carry the same version — every nightly in a series does.
    pub build: u64,
    /// When it was published, RFC 3339. Shown, never compared.
    pub published: String,
    /// Where a person can read what changed.
    pub notes_url: String,
    /// The artifacts, keyed by [`Platform::feed_key()`].
    pub assets: HashMap<String, Asset>,
}

impl Release {
    /// The artifact this build would install, if the manifest has one.
    #[must_use]
    pub fn asset_for_current_platform(&self) -> Option<&Asset> {
        self.assets.get(Platform::current()?.feed_key())
    }
}

/// What a completed check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    /// The version that is running.
    pub current_version: String,
    /// The build number that is running.
    pub current_build: u64,
    /// The release to move to, when the feed holds one that is newer than
    /// what is running and has an artifact this build could install.
    pub available: Option<Release>,
}

impl UpdateCheck {
    /// Whether there is something to install.
    #[must_use]
    pub fn has_update(&self) -> bool {
        self.available.is_some()
    }
}

/// Why a check or a download did not finish.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// The feed could not be reached, or did not answer.
    #[error(transparent)]
    Http(#[from] HttpError),
    /// The manifest is not signed by a key this build trusts.
    #[error("The update manifest's signature is not valid")]
    Signature,
    /// The manifest is signed but could not be understood.
    #[error("The update manifest could not be read: {0}")]
    Malformed(String),
    /// This build has no artifact in the manifest, or none it could install.
    #[error("This build of Commune cannot update itself")]
    Unsupported,
    /// What was downloaded is not what the manifest described.
    #[error("The download does not match the checksum in the update manifest")]
    Checksum,
    /// The download could not be written.
    #[error("The download could not be saved: {0}")]
    Io(#[from] std::io::Error),
}

impl UserFacingError for UpdateError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::Http(_) => "Could not reach the update server.".to_owned(),
            Self::Signature | Self::Malformed(_) => {
                "The update information could not be verified.".to_owned()
            }
            Self::Unsupported => "This installation cannot update itself.".to_owned(),
            Self::Checksum => "The download was incomplete or damaged.".to_owned(),
            Self::Io(_) => "The download could not be saved.".to_owned(),
        }
    }
}

/// The settings this module keeps.
///
/// Stored as one JSON value under a single key, the way the session settings
/// are, so that adding a field later does not need a schema change in either
/// embedder — the GTK application's `GSettings` schema has one string key for
/// all of this, and Android's file store has no schema at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// Whether to check without being asked.
    ///
    /// On by default. An installation of this application has nothing else
    /// that would tell it a security fix exists, and the check is one small
    /// signed file a day; a person who would rather it did not happen turns
    /// it off, and nothing then contacts the feed until they press a button.
    pub check_automatically: bool,
    /// The channel being followed. `None` means the profile's default, which
    /// is read at use rather than written here so that a Devel build and a
    /// stable build sharing a settings file do not fight over it.
    pub channel: Option<Channel>,
    /// When the last check finished, as a Unix timestamp in seconds.
    pub last_check: Option<u64>,
    /// The version the user asked not to be told about again.
    pub skipped_version: Option<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_automatically: true,
            channel: None,
            last_check: None,
            skipped_version: None,
        }
    }
}

impl UpdateSettings {
    /// Read them from the settings store.
    #[must_use]
    pub fn load() -> Self {
        let Some(serialized) = config::settings_store().get(UPDATE_SETTINGS_KEY) else {
            return Self::default();
        };

        serde_json::from_str(&serialized).unwrap_or_else(|parse_error| {
            error!("Could not read the update settings, using the defaults: {parse_error}");
            Self::default()
        })
    }

    /// Write them back to the settings store.
    pub fn save(&self) {
        let serialized = serde_json::to_string(self).expect("update settings serialize");
        config::settings_store().set(UPDATE_SETTINGS_KEY, &serialized);
    }

    /// The channel to check, resolving `None` to the profile's default.
    #[must_use]
    pub fn effective_channel(&self) -> Channel {
        let default = Channel::default_for_profile();

        // A channel this profile does not offer is treated as unset rather
        // than honoured. It is what a settings file shared with another
        // profile looks like, and following it would check a feed whose
        // artifacts cannot install over this build.
        self.channel
            .filter(|channel| Channel::available_for_profile().contains(channel))
            .unwrap_or(default)
    }

    /// Whether an automatic check is due.
    ///
    /// False when automatic checks are off, so the caller does not have to
    /// ask twice.
    #[must_use]
    pub fn is_check_due(&self, now: u64) -> bool {
        if !self.check_automatically {
            return false;
        }

        self.last_check.is_none_or(|last| {
            // A clock that went backwards — a laptop returning from a wrong
            // time zone, a container starting with the epoch — would
            // otherwise put the next check arbitrarily far in the future.
            now < last || now.saturating_sub(last) >= CHECK_INTERVAL_SECONDS
        })
    }

    /// Whether the user asked not to be told about this release again.
    #[must_use]
    pub fn is_skipped(&self, release: &Release) -> bool {
        self.skipped_version
            .as_ref()
            .is_some_and(|skipped| *skipped == release.version)
    }
}

/// The version this build reports as its own.
#[must_use]
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Where the feed is being read from.
fn feed_base() -> String {
    std::env::var(FEED_BASE_ENV).map_or_else(
        |_| DEFAULT_FEED_BASE.to_owned(),
        |base| base.trim_end_matches('/').to_owned(),
    )
}

/// Check whether the given channel holds a release newer than this build.
///
/// Must be called from the tokio runtime. Returns `Ok` with
/// [`UpdateCheck::available`] unset when the feed is reachable and this build
/// is current, which is the ordinary answer and not a condition worth
/// reporting to anybody.
///
/// # Errors
///
/// If the feed cannot be reached, is not signed by a trusted key, cannot be
/// parsed, or is for a different channel than the one asked for.
pub async fn check(channel: Channel) -> Result<UpdateCheck, UpdateError> {
    let base = feed_base();
    let manifest_url = format!("{base}/{channel}.json");
    let signature_url = format!("{manifest_url}.sig");

    debug!("Checking {manifest_url} for a newer release");

    let manifest = http::fetch(&manifest_url, MAX_MANIFEST_SIZE).await?;
    let signature = http::fetch(&signature_url, MAX_SIGNATURE_SIZE).await?;

    verify(&manifest, &signature, key::VERIFYING_KEYS)?;

    let release: Release = serde_json::from_slice(&manifest)
        .map_err(|parse_error| UpdateError::Malformed(parse_error.to_string()))?;

    if release.channel != channel {
        // Signed, and still not the document that was asked for. Reading it
        // would let whoever can choose which file the feed serves move an
        // installation between channels.
        return Err(UpdateError::Malformed(format!(
            "the {channel} manifest declares itself to be for {}",
            release.channel
        )));
    }

    let current_version = current_version().to_owned();
    let current_build = config::build_number();

    let newer = is_newer(&release, &current_version, current_build)?;
    let installable = release.asset_for_current_platform().is_some();

    if newer && !installable {
        // Worth a log line: it means a release went out without this
        // platform's artifact, which is a broken release rather than a
        // broken installation.
        warn!(
            "Release {} is newer than {current_version} but carries no artifact for this platform",
            release.version
        );
    }

    if newer && installable {
        info!(
            "Release {} is newer than {current_version}",
            release.version
        );
    }

    Ok(UpdateCheck {
        current_version,
        current_build,
        available: (newer && installable).then_some(release),
    })
}

/// Whether the given release is newer than the running build.
fn is_newer(
    release: &Release,
    current_version: &str,
    current_build: u64,
) -> Result<bool, UpdateError> {
    let current = semver::Version::parse(current_version).map_err(|parse_error| {
        UpdateError::Malformed(format!(
            "this build's own version is not semver: {parse_error}"
        ))
    })?;
    let offered = semver::Version::parse(&release.version).map_err(|parse_error| {
        UpdateError::Malformed(format!(
            "the manifest's version is not semver: {parse_error}"
        ))
    })?;

    Ok(match offered.cmp(&current) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        // The same version, which is every nightly in a series and both
        // artifacts of a rebuilt release. The commit count is what
        // distinguishes them.
        std::cmp::Ordering::Equal => release.build > current_build,
    })
}

/// Check a manifest against its detached signature.
///
/// The keys are a parameter rather than read from [`key`] so that the tests
/// can sign with a key of their own; nothing but a test passes anything else.
fn verify(manifest: &[u8], signature: &[u8], keys: &[[u8; 32]]) -> Result<(), UpdateError> {
    let signature = std::str::from_utf8(signature)
        .map(str::trim)
        .map_err(|_| UpdateError::Signature)?;
    let signature: [u8; 64] = hex::decode(signature)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(UpdateError::Signature)?;
    let signature = Signature::from_bytes(&signature);

    let verified = keys.iter().any(|key| {
        VerifyingKey::from_bytes(key)
            .is_ok_and(|key| key.verify_strict(manifest, &signature).is_ok())
    });

    if verified {
        Ok(())
    } else {
        Err(UpdateError::Signature)
    }
}

/// Told how far a download has got.
///
/// Called from the runtime thread doing the download, often — every chunk —
/// so an implementation that touches a UI must hand the numbers over rather
/// than draw anything itself.
pub trait DownloadProgress: Send + Sync {
    /// How many bytes have arrived out of how many are expected.
    fn progress(&self, downloaded: u64, total: u64);
}

/// Download an artifact into `dest_dir`, checking it against the manifest.
///
/// Must be called from the tokio runtime. The file is written under a
/// temporary name and renamed once its digest matches, so the path this
/// returns is never a partial download — a run that is interrupted leaves
/// rubbish that the next one overwrites, not something an installer would
/// happily try to run.
///
/// # Errors
///
/// If the artifact cannot be fetched, does not match the size or digest the
/// manifest gives, or cannot be written.
pub async fn download(
    asset: &Asset,
    dest_dir: &Path,
    progress: Option<Arc<dyn DownloadProgress>>,
) -> Result<PathBuf, UpdateError> {
    let file_name = artifact_file_name(&asset.url)?;

    tokio::fs::create_dir_all(dest_dir).await?;

    let final_path = dest_dir.join(&file_name);
    let partial_path = dest_dir.join(format!("{file_name}.part"));

    let response = CLIENT
        .get(&asset.url)
        .send()
        .await
        .map_err(HttpError::from)?
        .error_for_status()
        .map_err(HttpError::from)?;

    if response
        .content_length()
        .is_some_and(|length| length != asset.size)
    {
        return Err(UpdateError::Checksum);
    }

    let mut file = tokio::fs::File::create(&partial_path).await?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut stream = response.bytes_stream();

    let outcome = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(HttpError::from)?;

            downloaded += chunk.len() as u64;

            if downloaded > asset.size {
                return Err(UpdateError::Checksum);
            }

            hasher.update(&chunk);
            file.write_all(&chunk).await?;

            if let Some(progress) = &progress {
                progress.progress(downloaded, asset.size);
            }
        }

        file.flush().await?;
        file.sync_all().await?;

        if downloaded != asset.size {
            return Err(UpdateError::Checksum);
        }

        if hex::encode(hasher.finalize()) != asset.sha256.to_ascii_lowercase() {
            return Err(UpdateError::Checksum);
        }

        Ok(())
    }
    .await;

    drop(file);

    if let Err(download_error) = outcome {
        // Leaving a file that failed its checksum where an installer could
        // be pointed at it is the one outcome worth going out of the way to
        // avoid.
        if let Err(remove_error) = tokio::fs::remove_file(&partial_path).await {
            warn!("Could not remove the failed download: {remove_error}");
        }

        return Err(download_error);
    }

    tokio::fs::rename(&partial_path, &final_path).await?;

    info!("Downloaded {file_name} to {}", dest_dir.display());

    Ok(final_path)
}

/// The name to save an artifact under, taken from its URL.
///
/// The URL is in a signed manifest, so this is not a trust boundary, but the
/// name becomes a path and a signed mistake is still a mistake: anything that
/// is not a plain file name is refused rather than joined onto a directory.
fn artifact_file_name(url: &str) -> Result<String, UpdateError> {
    let name = url
        .rsplit('/')
        .next()
        .map(|name| name.split(['?', '#']).next().unwrap_or(name))
        .unwrap_or_default();

    let acceptable = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.starts_with('.');

    if acceptable {
        Ok(name.to_owned())
    } else {
        Err(UpdateError::Malformed(format!(
            "the artifact URL does not end in a usable file name: {url}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    /// A key pair that exists only here, so that the tests can produce
    /// manifests that verify without the real signing key being anywhere
    /// near this repository.
    fn test_keys() -> (SigningKey, [u8; 32]) {
        let signing = SigningKey::from_bytes(&[7_u8; 32]);
        let verifying = signing.verifying_key().to_bytes();

        (signing, verifying)
    }

    fn manifest_json(version: &str, build: u64, channel: &str) -> String {
        format!(
            r#"{{
                "channel": "{channel}",
                "version": "{version}",
                "build": {build},
                "published": "2026-10-02T18:40:00Z",
                "notes_url": "https://example.invalid/notes",
                "assets": {{
                    "windows-x86_64-msi": {{
                        "url": "https://example.invalid/Commune-x64.msi",
                        "sha256": "00",
                        "size": 1
                    }},
                    "macos-universal-tar": {{
                        "url": "https://example.invalid/commune.tar.gz",
                        "sha256": "00",
                        "size": 1
                    }},
                    "android-arm64-apk": {{
                        "url": "https://example.invalid/commune.apk",
                        "sha256": "00",
                        "size": 1
                    }}
                }}
            }}"#
        )
    }

    fn sign(manifest: &str) -> Vec<u8> {
        let (signing, _) = test_keys();

        hex::encode(signing.sign(manifest.as_bytes()).to_bytes()).into_bytes()
    }

    #[test]
    fn a_correctly_signed_manifest_verifies() {
        let (_, verifying) = test_keys();
        let manifest = manifest_json("1.0.0", 1, "stable");

        verify(manifest.as_bytes(), &sign(&manifest), &[verifying])
            .expect("a manifest signed by the key it is checked against verifies");
    }

    #[test]
    fn a_manifest_changed_after_signing_does_not_verify() {
        let (_, verifying) = test_keys();
        let manifest = manifest_json("1.0.0", 1, "stable");
        let signature = sign(&manifest);
        let tampered = manifest_json("9.0.0", 1, "stable");

        assert!(matches!(
            verify(tampered.as_bytes(), &signature, &[verifying]),
            Err(UpdateError::Signature)
        ));
    }

    #[test]
    fn a_manifest_signed_by_another_key_does_not_verify() {
        let manifest = manifest_json("1.0.0", 1, "stable");
        let other = SigningKey::from_bytes(&[9_u8; 32])
            .verifying_key()
            .to_bytes();

        assert!(matches!(
            verify(manifest.as_bytes(), &sign(&manifest), &[other]),
            Err(UpdateError::Signature)
        ));
    }

    #[test]
    fn a_signature_that_is_not_hex_does_not_verify() {
        let (_, verifying) = test_keys();
        let manifest = manifest_json("1.0.0", 1, "stable");

        for signature in ["", "not hex", "00", &"aa".repeat(65)] {
            assert!(
                matches!(
                    verify(manifest.as_bytes(), signature.as_bytes(), &[verifying]),
                    Err(UpdateError::Signature)
                ),
                "{signature:?} should not verify"
            );
        }
    }

    #[test]
    fn a_signature_with_trailing_whitespace_still_verifies() {
        let (_, verifying) = test_keys();
        let manifest = manifest_json("1.0.0", 1, "stable");
        let mut signature = sign(&manifest);
        signature.extend_from_slice(b"\n");

        verify(manifest.as_bytes(), &signature, &[verifying])
            .expect("a signature file ending in a newline is the ordinary case");
    }

    #[test]
    fn any_one_of_several_keys_is_enough() {
        let (_, verifying) = test_keys();
        let other = SigningKey::from_bytes(&[9_u8; 32])
            .verifying_key()
            .to_bytes();
        let manifest = manifest_json("1.0.0", 1, "stable");

        verify(manifest.as_bytes(), &sign(&manifest), &[other, verifying])
            .expect("a rotation in flight accepts both keys");
    }

    fn release(version: &str, build: u64) -> Release {
        serde_json::from_str(&manifest_json(version, build, "stable")).expect("the fixture parses")
    }

    #[test]
    fn a_higher_version_is_newer() {
        assert!(is_newer(&release("1.0.1", 1), "1.0.0", 500).expect("both versions are semver"));
    }

    #[test]
    fn a_lower_version_is_not_newer() {
        assert!(!is_newer(&release("0.9.0", 9999), "1.0.0", 1).expect("both versions are semver"));
    }

    #[test]
    fn a_release_candidate_is_older_than_its_release() {
        assert!(is_newer(&release("1.0.0", 1), "1.0.0-rc1", 1).expect("both versions are semver"));
        assert!(!is_newer(&release("1.0.0-rc1", 1), "1.0.0", 1).expect("both versions are semver"));
    }

    #[test]
    fn a_later_release_candidate_is_newer() {
        assert!(
            is_newer(&release("1.0.0-rc2", 1), "1.0.0-rc1", 1).expect("both versions are semver")
        );
    }

    #[test]
    fn the_same_version_is_ordered_by_build() {
        assert!(is_newer(&release("1.0.0-rc1", 4100), "1.0.0-rc1", 4021).expect("semver"));
        assert!(!is_newer(&release("1.0.0-rc1", 4021), "1.0.0-rc1", 4021).expect("semver"));
        assert!(!is_newer(&release("1.0.0-rc1", 4000), "1.0.0-rc1", 4021).expect("semver"));
    }

    #[test]
    fn a_version_that_is_not_semver_is_an_error() {
        assert!(matches!(
            is_newer(&release("1.rc1", 1), "1.0.0", 1),
            Err(UpdateError::Malformed(_))
        ));
    }

    #[test]
    fn every_platform_key_is_in_the_fixture() {
        let release = release("1.0.0", 1);

        for platform in [
            Platform::WindowsMsi,
            Platform::MacOsTarball,
            Platform::AndroidApk,
        ] {
            assert!(
                release.assets.contains_key(platform.feed_key()),
                "{platform:?} is missing from the fixture"
            );
        }
    }

    #[test]
    fn a_manifest_without_this_platforms_asset_offers_nothing() {
        let mut release = release("1.0.0", 1);
        release.assets.clear();

        assert!(release.asset_for_current_platform().is_none());
    }

    #[test]
    fn an_artifact_file_name_comes_from_the_url() {
        assert_eq!(
            artifact_file_name("https://example.invalid/a/b/Commune-1.0.0-x64.msi")
                .expect("a plain name is usable"),
            "Commune-1.0.0-x64.msi"
        );
        assert_eq!(
            artifact_file_name("https://example.invalid/commune.tar.gz?token=1")
                .expect("a query string is not part of the name"),
            "commune.tar.gz"
        );
    }

    #[test]
    fn an_artifact_url_without_a_usable_name_is_refused() {
        for url in [
            "https://example.invalid/",
            "https://example.invalid/..",
            "https://example.invalid/.hidden",
        ] {
            assert!(
                matches!(artifact_file_name(url), Err(UpdateError::Malformed(_))),
                "{url} should be refused"
            );
        }
    }

    #[test]
    fn the_default_settings_check_automatically() {
        let settings = UpdateSettings::default();

        assert!(settings.check_automatically);
        assert!(settings.is_check_due(0));
    }

    #[test]
    fn a_check_is_not_due_again_within_the_interval() {
        let settings = UpdateSettings {
            last_check: Some(1_000_000),
            ..UpdateSettings::default()
        };

        assert!(!settings.is_check_due(1_000_000 + CHECK_INTERVAL_SECONDS - 1));
        assert!(settings.is_check_due(1_000_000 + CHECK_INTERVAL_SECONDS));
    }

    #[test]
    fn a_clock_that_went_backwards_does_not_postpone_the_check() {
        let settings = UpdateSettings {
            last_check: Some(1_000_000),
            ..UpdateSettings::default()
        };

        assert!(settings.is_check_due(5));
    }

    #[test]
    fn a_check_is_never_due_when_automatic_checks_are_off() {
        let settings = UpdateSettings {
            check_automatically: false,
            ..UpdateSettings::default()
        };

        assert!(!settings.is_check_due(u64::MAX));
    }

    #[test]
    fn a_skipped_version_is_recognized_by_version_alone() {
        let settings = UpdateSettings {
            skipped_version: Some("1.0.0".to_owned()),
            ..UpdateSettings::default()
        };

        assert!(settings.is_skipped(&release("1.0.0", 1)));
        assert!(!settings.is_skipped(&release("1.0.1", 1)));
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let settings = UpdateSettings {
            check_automatically: false,
            channel: Some(Channel::Rc),
            last_check: Some(42),
            skipped_version: Some("1.0.0".to_owned()),
        };
        let serialized = serde_json::to_string(&settings).expect("settings serialize");

        assert_eq!(
            serde_json::from_str::<UpdateSettings>(&serialized).expect("settings parse"),
            settings
        );
    }

    #[test]
    fn settings_written_before_a_field_existed_still_load() {
        assert_eq!(
            serde_json::from_str::<UpdateSettings>("{}").expect("an empty object is the defaults"),
            UpdateSettings::default()
        );
    }

    #[test]
    fn a_channel_round_trips_through_its_name() {
        for channel in [Channel::Stable, Channel::Rc, Channel::Nightly] {
            assert_eq!(
                channel
                    .as_str()
                    .parse::<Channel>()
                    .expect("its own name parses"),
                channel
            );
        }
    }
}

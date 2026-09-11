//! Installing a downloaded release over this one, on macOS.
//!
//! The artifact is the `.tar.gz` rather than the `.dmg`, for the reason
//! `doc/macos.md` gives about the disk image: a bundle that arrives inside
//! one is quarantined, and a quarantined bundle is refused on first launch
//! until somebody clears the attribute by hand. Files a process extracts from
//! a tarball are not quarantined at all, so the tarball is the artifact that
//! can be installed without the user being sent to System Settings.
//!
//! # What replaces what
//!
//! A macOS application is a directory, and replacing one is a rename: the new
//! bundle is extracted beside the old one, checked, and then the two are
//! swapped. Nothing is ever written *into* the running bundle, so a failure
//! at any point up to the rename leaves the installed copy exactly as it was.
//!
//! # What is checked first
//!
//! Two things, and both matter more here than on Windows, because `open` will
//! launch whatever it is pointed at:
//!
//! * The new bundle's signature has to verify. `codesign --verify --strict` is
//!   the same check Gatekeeper makes.
//! * Its Team ID has to equal the running bundle's. This is what stops a
//!   correctly signed application that is not Commune from being installed as
//!   Commune, and it is also why an ad-hoc signed build never updates: an
//!   ad-hoc signature has no team, and every ad-hoc build is a different
//!   identity, so there is nothing to compare.

use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    process::Command,
};

use tracing::{debug, warn};

/// Whether this build is an installed one that may replace itself.
///
/// True only for a bundle that carries a real signing identity. A build
/// running out of a Meson prefix has no bundle to replace, and an ad-hoc
/// signed one is a developer's own build: replacing it with a release would
/// change the signing identity under every Keychain item it has stored, which
/// re-prompts for each of them.
pub(crate) fn is_installed() -> bool {
    running_bundle().is_some_and(|bundle| team_identifier(&bundle).is_some())
}

/// The `.app` directory this executable is inside, if it is inside one.
///
/// `Commune.app/Contents/MacOS/commune` walks back to `Commune.app`, which is
/// the same shape `app_bundle` resolves resources from.
fn running_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;

    (bundle.extension() == Some(OsStr::new("app"))).then(|| bundle.to_path_buf())
}

/// The Team ID in the given bundle's signature, if it has one.
///
/// `None` for an unsigned or ad-hoc signed bundle, which is exactly the case
/// this must refuse: `codesign` prints `TeamIdentifier=not set` for both.
fn team_identifier(bundle: &Path) -> Option<String> {
    let output = Command::new("/usr/bin/codesign")
        .arg("-d")
        .arg("--verbose=2")
        .arg(bundle)
        .output()
        .ok()?;

    // `codesign -d` writes its report to stderr, not stdout.
    let report = String::from_utf8_lossy(&output.stderr);

    report
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|team| !team.is_empty() && *team != "not set")
        .map(ToOwned::to_owned)
}

/// Whether the given bundle's signature is intact.
fn signature_verifies(bundle: &Path) -> bool {
    Command::new("/usr/bin/codesign")
        .arg("--verify")
        .arg("--deep")
        .arg("--strict")
        .arg(bundle)
        .status()
        .is_ok_and(|status| status.success())
}

/// Unpack the tarball into the given directory.
///
/// `/usr/bin/tar` rather than a crate: it is on every macOS, it is what wrote
/// the archive in the first place (`build-aux/macos/make-tarball.sh`), and a
/// bundle whose signature has to survive the round trip is not the place to
/// find out that a reimplementation handles some corner of the format
/// differently.
fn extract(tarball: &Path, into: &Path) -> io::Result<()> {
    std::fs::create_dir_all(into)?;

    let status = Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(into)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "tar exited with {status} while unpacking the update"
        )))
    }
}

/// The single `.app` directory directly inside the given directory.
fn bundle_in(directory: &Path) -> io::Result<PathBuf> {
    let mut found = None;

    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();

        if path.extension() == Some(OsStr::new("app")) {
            if found.is_some() {
                return Err(io::Error::other(
                    "the update archive holds more than one application",
                ));
            }

            found = Some(path);
        }
    }

    found.ok_or_else(|| io::Error::other("the update archive holds no application"))
}

/// Install the bundle in the given tarball over the running one and restart.
///
/// The caller must quit immediately afterwards. Unlike the Windows path, the
/// replacement has already happened by the time this returns — a directory
/// rename does not need this process to be gone — so what is deferred is only
/// the relaunch.
///
/// # Errors
///
/// If the archive cannot be unpacked, holds something that is not a single
/// signed Commune, or the installed bundle cannot be replaced.
pub(crate) fn install(tarball: &Path) -> io::Result<()> {
    let current = running_bundle()
        .ok_or_else(|| io::Error::other("this build is not inside an application bundle"))?;
    let parent = current
        .parent()
        .ok_or_else(|| io::Error::other("the application bundle has no parent directory"))?;

    let team = team_identifier(&current).ok_or_else(|| {
        io::Error::other("the running application is not signed with a team identity")
    })?;

    // Beside the installed bundle rather than in a temporary directory, so
    // that the swap is a rename within one file system and cannot half
    // happen. It also fails early, and harmlessly, when the directory the
    // application lives in is not writable.
    let staging = parent.join(format!(".commune-update-{}", std::process::id()));

    let result = install_from(tarball, &staging, &current, &team);

    if let Err(remove_error) = std::fs::remove_dir_all(&staging)
        && remove_error.kind() != io::ErrorKind::NotFound
    {
        warn!("Could not clean up the update staging directory: {remove_error}");
    }

    result?;

    // macOS caches application icons hard, and replacing a bundle in place is
    // the case it gets wrong — see doc/macos.md. Re-registering is what makes
    // the Dock and Finder notice.
    let _ = Command::new("/usr/bin/touch").arg(&current).status();
    let _ = Command::new(
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/\
         Support/lsregister",
    )
    .arg("-f")
    .arg(&current)
    .status();

    relaunch(&current)
}

/// Unpack, check and swap, leaving the staging directory for the caller to
/// remove either way.
fn install_from(tarball: &Path, staging: &Path, current: &Path, team: &str) -> io::Result<()> {
    extract(tarball, staging)?;

    let replacement = bundle_in(staging)?;

    if !signature_verifies(&replacement) {
        return Err(io::Error::other("the update's signature does not verify"));
    }

    let replacement_team = team_identifier(&replacement)
        .ok_or_else(|| io::Error::other("the update is not signed with a team identity"))?;

    if replacement_team != team {
        return Err(io::Error::other(format!(
            "the update is signed by team {replacement_team}, not {team}"
        )));
    }

    // Move the old bundle aside rather than deleting it: if the second rename
    // fails there is still an application on disk, and the user can put it
    // back by hand.
    let displaced = staging.join("previous.app");
    std::fs::rename(current, &displaced)?;

    if let Err(rename_error) = std::fs::rename(&replacement, current) {
        // Put it back. Leaving the user with no application at all is the one
        // outcome worth unwinding for.
        if let Err(restore_error) = std::fs::rename(&displaced, current) {
            return Err(io::Error::other(format!(
                "the update could not be installed ({rename_error}) and the previous version \
                 could not be restored ({restore_error}); it is at {}",
                displaced.display()
            )));
        }

        return Err(rename_error);
    }

    debug!("Replaced {}", current.display());

    Ok(())
}

/// Start the newly installed bundle once this process is gone.
///
/// `open -n` would refuse while an instance of the same bundle identifier is
/// still running, and this one is, so the launch is handed to a shell that
/// outlives us and waits first.
fn relaunch(bundle: &Path) -> io::Result<()> {
    let bundle = bundle.to_string_lossy().replace('\'', r"'\''");

    Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("sleep 2; /usr/bin/open -n '{bundle}'"))
        .spawn()?;

    Ok(())
}

/// Whether the given name looks like a package this platform installs.
pub(crate) fn is_installer(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".tar.gz")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_tarball_is_an_installer() {
        assert!(is_installer(Path::new("commune-1-arm64.tar.gz")));
        assert!(!is_installer(Path::new("commune-1-arm64.dmg")));
        assert!(!is_installer(Path::new("Commune.msi")));
    }

    #[test]
    fn a_bundle_is_found_in_a_directory() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::create_dir(directory.path().join("Commune.app")).expect("the bundle is created");

        assert_eq!(
            bundle_in(directory.path()).expect("one bundle is found"),
            directory.path().join("Commune.app")
        );
    }

    #[test]
    fn a_directory_with_no_bundle_is_an_error() {
        let directory = tempfile::tempdir().expect("a temporary directory");

        assert!(bundle_in(directory.path()).is_err());
    }

    #[test]
    fn a_directory_with_two_bundles_is_an_error() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::create_dir(directory.path().join("Commune.app")).expect("the bundle is created");
        std::fs::create_dir(directory.path().join("Other.app")).expect("the bundle is created");

        assert!(bundle_in(directory.path()).is_err());
    }
}

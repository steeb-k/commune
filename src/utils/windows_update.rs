//! Installing a downloaded release over this one, on Windows.
//!
//! The `.msi` this fetches is the same one a person would download and
//! double-click, and it already knows how to replace an installed copy: the
//! package is `Scope="perUser"`, so no elevation is involved, and it carries a
//! `MajorUpgrade` with a `UpgradeCode` that has not changed since the first
//! release (`build-aux/windows/commune.wxs`). There is nothing to add to the
//! installer for this; the whole job here is to check the package, get out of
//! its way, and come back afterwards.
//!
//! # Why a script
//!
//! An upgrade removes the installed files, this executable among them, so
//! this process cannot be running while it happens, and something has to
//! start the new copy once it is over. That something cannot be us. So a
//! small `.cmd` is written next to the download and started detached: it
//! waits for this process to be gone, runs `msiexec`, and launches whatever
//! is at the installed path afterwards — the new build if the upgrade worked,
//! the old one if it did not, which is the right answer either way, because
//! a failed `MajorUpgrade` rolls back and leaves the previous version
//! installed and working.

use std::{
    ffi::OsStr,
    io,
    os::windows::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
};

use tracing::{debug, warn};
use windows::{
    Win32::{
        Foundation::{HWND, TRUST_E_NOSIGNATURE},
        Security::WinTrust::{
            WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
            WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY,
            WTD_UI_NONE, WinVerifyTrust,
        },
    },
    core::PCWSTR,
};

/// Hide the helper's console instead of removing it.
///
/// `CREATE_NO_WINDOW`. The script's own `msiexec` draws the progress bar the
/// user is meant to see; the console that runs it is not — but the script
/// still needs that console to exist. It pipes `tasklist` into `find` to
/// wait for this process to exit, and a pipe is built out of the anonymous
/// handles a console process gets at creation; take the console away
/// (`DETACHED_PROCESS`, which this used to also pass, on the theory that the
/// helper must outlive us) and the pipeline has nothing to run on, so the
/// script hangs forever on its first iteration and the installer never runs.
/// A `cmd.exe` started with `CreateProcess` is not a child in any sense that
/// matters here — it is not in our job object, so it is not killed when we
/// exit — `DETACHED_PROCESS` was never the thing keeping it alive; it only
/// took away what the script needed. Hidden is fine. Absent is not.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Whether this build is an installed one that may replace itself.
///
/// The `.msi` installs into `%LOCALAPPDATA%\Programs\Commune`, so an
/// executable somewhere else is a build directory, a `.zip` someone
/// unpacked, or a `meson install` into an MSYS2 prefix. Running an installer
/// over any of those would not replace what is running — it would install a
/// *second* copy elsewhere and leave the user with two — so the updater stays
/// quiet instead.
pub(crate) fn is_installed() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };

    exe.starts_with(programs_dir())
}

/// `%LOCALAPPDATA%\Programs`, where the per-user installer puts the
/// application.
fn programs_dir() -> PathBuf {
    // `user_data_dir` is `%LOCALAPPDATA%` on Windows, which is the
    // `LocalAppDataFolder` the package's install directory hangs off.
    gtk::glib::user_data_dir().join("Programs")
}

/// The installed executable, which is what should be running when this is
/// over.
fn installed_executable() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Whether the given file carries a valid Authenticode signature.
///
/// The download already matched a SHA-256 digest out of a signed manifest, so
/// this is not the only thing standing between a user and a hostile package.
/// It is the check that does not depend on any of this code being right:
/// `msiexec` will install an unsigned per-user package without a word, and
/// refusing to hand it one that Windows itself does not vouch for is cheap.
fn is_signed(path: &Path) -> bool {
    let mut file_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();

    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: u32::try_from(size_of::<WINTRUST_FILE_INFO>()).unwrap_or_default(),
        pcwszFilePath: PCWSTR(file_path.as_mut_ptr()),
        ..Default::default()
    };

    let mut trust_data = WINTRUST_DATA {
        cbStruct: u32::try_from(size_of::<WINTRUST_DATA>()).unwrap_or_default(),
        dwUIChoice: WTD_UI_NONE,
        // No revocation check, deliberately. It is a network round trip to
        // an OCSP responder, made while the user is watching a progress bar,
        // and its failure mode is the wrong one: behind a captive portal or a
        // filtering proxy it reports a perfectly good package as untrusted
        // and the updater stops working. The chain is still built and
        // validated; what is skipped is only asking whether the certificate
        // was withdrawn, which Windows asks again for itself when the
        // installer runs.
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &raw mut file_info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };

    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;

    // SAFETY: `trust_data` is fully initialized above and outlives both
    // calls, and `file_info` and `file_path` outlive the pointers held to
    // them. The second call with `WTD_STATEACTION_CLOSE` is what the API
    // requires to release the state the first one allocated, and is made
    // whatever the first returned.
    let status = unsafe {
        WinVerifyTrust(
            HWND::default(),
            &raw mut action,
            (&raw mut trust_data).cast(),
        )
    };

    trust_data.dwStateAction = WTD_STATEACTION_CLOSE;

    // SAFETY: as above; this releases the state the verify call allocated.
    unsafe {
        WinVerifyTrust(
            HWND::default(),
            &raw mut action,
            (&raw mut trust_data).cast(),
        );
    }

    if status == 0 {
        return true;
    }

    if status == TRUST_E_NOSIGNATURE.0 {
        warn!("The downloaded package carries no Authenticode signature");
    } else {
        warn!("The downloaded package's Authenticode signature is not valid: {status:#x}");
    }

    false
}

/// Start the installer for the given package and return.
///
/// The caller must quit the application immediately afterwards: the script
/// this starts is already waiting for that, and an upgrade cannot replace
/// files this process has open.
///
/// # Invariant
///
/// The helper script waits for us with a `tasklist | find` pipeline, so it
/// needs a console to run that pipeline on. Any future change to how this is
/// launched must keep that true: hidden is fine (`CREATE_NO_WINDOW`), absent
/// is not (`DETACHED_PROCESS`, which is why this does not use it).
///
/// # Errors
///
/// If the package is not signed, or the helper script could not be written or
/// started.
pub(crate) fn install(msi: &Path) -> io::Result<()> {
    if !is_signed(msi) {
        return Err(io::Error::other(
            "the downloaded package is not signed by a trusted publisher",
        ));
    }

    let executable = installed_executable().ok_or_else(|| {
        io::Error::other("could not work out which executable to restart after the upgrade")
    })?;

    let script_path = msi.with_extension("cmd");
    let script = upgrade_script(std::process::id(), msi, &executable);

    std::fs::write(&script_path, script)?;

    debug!("Starting the upgrade helper at {}", script_path.display());

    Command::new(cmd_exe())
        .arg("/c")
        .arg(&script_path)
        // Hidden, not detached: see the invariant on this function.
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;

    Ok(())
}

/// `cmd.exe`, by absolute path where the environment names one.
///
/// `%COMSPEC%` is what every Windows sets; falling back to the bare name only
/// matters on a system where it has been unset.
fn cmd_exe() -> PathBuf {
    std::env::var_os("COMSPEC").map_or_else(|| PathBuf::from("cmd.exe"), PathBuf::from)
}

/// How many wait iterations to allow before installing anyway.
///
/// One `ping -n 2` per iteration is close enough to a second each (`ping`'s
/// own delay plus the two pings) that this bounds the wait at about two
/// minutes. A helper that waits forever for a process that will not exit —
/// whatever the reason — is worse than an installer that runs while the
/// app still appears to be running: `msiexec` reports that plainly and does
/// not corrupt anything, whereas a wedged helper never upgrades at all.
const MAX_WAIT_ITERATIONS: u32 = 120;

/// The script that waits for this process to exit, upgrades, and restarts.
///
/// Separated from [`install()`] so that it can be read — and tested — without
/// running an installer.
fn upgrade_script(pid: u32, msi: &Path, executable: &Path) -> String {
    // `tasklist`'s filter is the portable way to wait for a process from a
    // batch file: there is no `wait` for something that is not a child, and
    // this script is deliberately not a child of anything that will still be
    // alive. `ping` is the sleep, because `timeout` needs a console and this
    // script is started without one (it does still need a console to run the
    // `tasklist | find` pipeline on — see the invariant on `install()` — a
    // console it does not draw a window for is not a console it lacks).
    //
    // The wait is bounded: past `MAX_WAIT_ITERATIONS` the script installs
    // regardless of whether the old process is still around. The counter is
    // incremented and checked outside any `(...)` block so that each read of
    // `%COMMUNE_WAIT_COUNT%` is parsed fresh rather than frozen at the value
    // from when the block started.
    format!(
        "@echo off\r\n\
         setlocal\r\n\
         set COMMUNE_PID={pid}\r\n\
         set COMMUNE_WAIT_COUNT=0\r\n\
         :wait\r\n\
         set /a COMMUNE_WAIT_COUNT+=1\r\n\
         tasklist /FI \"PID eq %COMMUNE_PID%\" 2>nul | find \"%COMMUNE_PID%\" >nul\r\n\
         if errorlevel 1 goto install\r\n\
         if %COMMUNE_WAIT_COUNT% geq {max_wait} goto install\r\n\
         ping -n 2 127.0.0.1 >nul\r\n\
         goto wait\r\n\
         :install\r\n\
         msiexec /i \"{msi}\" /passive /norestart\r\n\
         start \"\" \"{executable}\"\r\n",
        max_wait = MAX_WAIT_ITERATIONS,
        msi = escape_for_batch(msi),
        executable = escape_for_batch(executable),
    )
}

/// A path as a batch file can hold it inside double quotes.
///
/// Both paths come from this process's own directories rather than from the
/// feed, so this is not a trust boundary; it is here because a `%` in a user
/// name is enough to break a batch file, and user names have `%` in them.
fn escape_for_batch(path: &Path) -> String {
    path.as_os_str()
        .to_string_lossy()
        .replace('%', "%%")
        .replace('"', "")
}

/// Whether the given name looks like a package this platform installs.
pub(crate) fn is_installer(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(OsStr::new("msi")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upgrade_script_waits_installs_and_restarts() {
        let script = upgrade_script(
            1234,
            Path::new(r"C:\cache\Commune-1.0.0-x64.msi"),
            Path::new(r"C:\Programs\Commune\bin\commune.exe"),
        );

        assert!(script.contains("PID eq %COMMUNE_PID%"));
        assert!(
            script.contains(r#"msiexec /i "C:\cache\Commune-1.0.0-x64.msi" /passive /norestart"#)
        );
        assert!(script.contains(r#"start "" "C:\Programs\Commune\bin\commune.exe""#));
        // A batch file with Unix line endings does not run.
        assert!(!script.contains('\n') || script.contains("\r\n"));
    }

    #[test]
    fn the_wait_is_bounded_and_installs_anyway() {
        let script = upgrade_script(
            1234,
            Path::new(r"C:\cache\Commune-1.0.0-x64.msi"),
            Path::new(r"C:\Programs\Commune\bin\commune.exe"),
        );

        assert!(script.contains("set COMMUNE_WAIT_COUNT=0"));
        assert!(script.contains("set /a COMMUNE_WAIT_COUNT+=1"));
        let bound = format!("if %COMMUNE_WAIT_COUNT% geq {MAX_WAIT_ITERATIONS} goto install");
        assert!(script.contains(&bound));
        // The bound check and the "still running" wait are both reachable
        // without being frozen inside the same `(...)` block as the
        // increment: neither line appears indented under a stray paren.
        assert!(!script.contains("if not errorlevel 1 ("));
        // `goto install` lands on the msiexec line even when the wait timed
        // out, not only when the process is confirmed gone.
        assert!(script.contains(":install\r\nmsiexec"));
    }

    #[test]
    fn a_percent_in_a_path_is_escaped() {
        let script = upgrade_script(
            1,
            Path::new(r"C:\Users\100%\a.msi"),
            Path::new(r"C:\Users\100%\commune.exe"),
        );

        assert!(script.contains(r"C:\Users\100%%\a.msi"));
    }

    #[test]
    fn only_an_msi_is_an_installer() {
        assert!(is_installer(Path::new("Commune.msi")));
        assert!(is_installer(Path::new("Commune.MSI")));
        assert!(!is_installer(Path::new("Commune.tar.gz")));
    }
}

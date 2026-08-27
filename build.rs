//! Embed the Windows resources into the executable.
//!
//! Everywhere else this does nothing at all.
//!
//! Windows has no metadata file beside the binary the way a macOS bundle has
//! its `Info.plist`: the icon Explorer, the taskbar and the Alt-Tab switcher
//! draw, the version the file's Properties dialog shows, and the manifest the
//! loader reads are all sections compiled into the executable itself. This puts
//! them there.

fn main() {
    // The Cargo.toml is the only input that can change what is embedded, apart
    // from the two variables read below.
    println!("cargo::rerun-if-changed=Cargo.toml");

    #[cfg(target_os = "windows")]
    windows_resources();
}

/// Compile the icon, the version information and the manifest into the
/// executable.
#[cfg(target_os = "windows")]
fn windows_resources() {
    use std::env;

    // Meson renders the icon out of `assets/appicon*.svg` and names it here,
    // because which of the two it is depends on the profile and only Meson
    // knows that. A bare `cargo build` leaves it unset and simply has no icon,
    // which is worth having rather than failing the build over.
    println!("cargo::rerun-if-env-changed=COMMUNE_WINDOWS_ICON");
    println!("cargo::rerun-if-env-changed=COMMUNE_WINDOWS_NAME");

    let mut resources = winresource::WindowsResource::new();

    if let Ok(icon) = env::var("COMMUNE_WINDOWS_ICON") {
        println!("cargo::rerun-if-changed={icon}");
        resources.set_icon(&icon);
    }

    // The name shown in the Properties dialog and in Task Manager. It carries
    // the profile, so a development build is not mistaken for a stable one in a
    // list of running processes.
    let name = env::var("COMMUNE_WINDOWS_NAME").unwrap_or_else(|_| "Commune".to_owned());

    resources
        .set("ProductName", &name)
        .set("FileDescription", &name)
        .set("CompanyName", "Commune")
        .set("LegalCopyright", "GPL-3.0-or-later")
        .set("OriginalFilename", "commune.exe");

    // `longPathAware` lets the file dialogs and everything behind them work
    // with paths past 260 characters, which is not hypothetical here: the
    // build hit that limit before the app ever ran. Nothing else is declared —
    // in particular not DPI awareness, which GTK sets for itself at startup and
    // which would be locked to whatever this said instead.
    resources.set_manifest(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings xmlns:ws2="http://schemas.microsoft.com/SMI/2016/WindowsSettings">
      <ws2:longPathAware>true</ws2:longPathAware>
    </windowsSettings>
  </application>
</assembly>
"#,
    );

    if let Err(error) = resources.compile() {
        // Not fatal: an executable with no icon and no version information runs
        // exactly as well as one with them, and refusing to build over it would
        // make the resource compiler a hard dependency of every Windows build.
        println!("cargo::warning=could not embed the Windows resources: {error}");
    }
}

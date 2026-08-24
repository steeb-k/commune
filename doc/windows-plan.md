# Plan: porting Commune to Windows

This is the working plan for the Windows port, written before any of it was built. It produces a
signed, relocatable `Commune\` folder (zipped for hand-outs) and a signed WiX 5 `.msi`, built and
signed on this Windows machine. When work starts, `doc/windows.md` becomes the ledger of what
actually exists; this file records the intended route and is updated as decisions change.

All of this work lives on the `windows-port` branch until `main` is free to take it.

## Contents

<!-- toc -->
* [Context](#context)
* [M0 — `cargo check` and `clippy` pass on Windows](#m0--cargo-check-and-clippy-pass-on-windows)
* [M1 — runs from the Meson dev build](#m1--runs-from-the-meson-dev-build)
* [M2 — relocatable, signed folder and `.zip`](#m2--relocatable-signed-folder-and-zip)
* [M3 — signed WiX 5 `.msi`](#m3--signed-wix-5-msi)
* [M4 — polish](#m4--polish)
* [M5 — notifications via WinRT toasts](#m5--notifications-via-winrt-toasts)
* [M6 — stretch: camera QR scanning](#m6--stretch-camera-qr-scanning)
* [Dependencies and recipes](#dependencies-and-recipes)
* [Docs and ledger updates](#docs-and-ledger-updates)
* [Risks and open questions](#risks-and-open-questions)
* [Critical files](#critical-files)
<!-- /toc -->

## Context

The macOS port (`doc/macos-plan.md`, `doc/macos.md`) already did most of the cross-platform work,
and much of it was cut at `not(target_os = "linux")` rather than at macOS, so Windows inherits it
for free: the image decoder abstraction (`src/utils/media/image/decoder/`, glycin on Linux, the
`image` crate elsewhere — including its `cfg(not(target_os = "linux"))` Cargo target table), the
camera and location fallbacks, the `SystemSettings` fallback, `key_bindings::PRIMARY_MASK` (already
`CONTROL_MASK` off macOS), and the unconditional `gtk::FileLauncher`.

What Windows does _not_ inherit:

* **Secrets panic.** `src/secret/mod.rs` falls to `UnimplementedSecret` outside Linux/macOS.
* **The build system requires glycin.** `meson.build:40` and the desktop-file/D-Bus blocks in
  `data/meson.build` are gated at `!= 'darwin'`, so on Windows they would still fire.
* **No data-directory decision.** GLib's `g_get_user_cache_dir()` on Windows points somewhere no
  Windows user would look (verify: reportedly the Internet-cache folder).
* **No notification backend.** GLib has no win32 `GNotification` backend at all — one less than
  macOS, whose deprecated backend at least existed.
* **No packaging**, no icon, no URL-scheme registration, and a console window on every launch.

Facts about the build machine, established up front:

* **MSYS2 is at `C:\msys64`**, and its **UCRT64** environment already holds gtk4 4.22.4,
  libadwaita 1.9.1, glib 2.88.2, gstreamer 1.28.5 and MSYS2's own Rust 1.97. Every remaining
  dependency is packaged: gtksourceview5 5.21, libshumate 1.5.1, gst-plugins-{base,good,bad,ugly,
  rs} 1.28.6 (`rs` contains `gtk4paintablesink` — built from source on macOS, a `pacman -S` here),
  libnice 0.1.23, glib-networking 2.80.1, blueprint-compiler 0.18, meson 1.12.
* **The gvsbuild prefix at `C:\gtk` was considered and rejected.** It carries gtk4 4.22.2,
  libadwaita 1.9.0 and gtksourceview5, but no GStreamer, no libshumate, no libsoup3 and no
  glib-networking — the heaviest parts of Commune's stack would all be hand-built under MSVC.
* **rustup on the Windows side has only MSVC targets**, and the MSVC ABI cannot link MSYS2's
  mingw-built GTK. The build therefore uses MSYS2's Rust (windows-gnu ABI) from a UCRT64 shell;
  rustup provides only the nightly rustfmt the pre-commit hook wants, which formats source and
  links nothing.
* **An Azure Trusted Signing certificate is available**, and the house pattern for using it exists
  next door: `seed-sync-gtk/scripts/{build-msi.ps1,sign-artifacts.ps1,publish-msi.ps1}` encode the
  WiX 5 + Azure signing workflow this plan adapts. Signing is ABI-agnostic — `signtool` signs any
  PE — so it pulls toward neither toolchain.

Decisions already made:

* Toolchain: **MSYS2 UCRT64**, MSYS2 Rust. Everything runs from a UCRT64 login shell.
* Secrets go to the **Windows Credential Manager** via the `windows` crate.
* Deliverables: a **signed relocatable folder + `.zip`** and a **signed WiX 5 `.msi`**. Unlike
  macOS there is no unsigned-distribution problem to route around. winget and MSIX are explicitly
  later, once the `.msi` is proven.
* Scope mirrors the macOS cut: location sharing stays stubbed, the 12/24-hour clock degrades
  gracefully (though a Windows backend is cheap — see M4), native notifications and camera QR
  scanning are kept but sequenced last. Calls are verified best-effort: MSYS2's GStreamer ships
  `webrtcbin`, nice, dtls and srtp, so the odds are good.

Project rules that constrain every step: the gresource prefix stays `/org/gnome/Fractal/`;
anything carrying the app name or ID is recorded in `doc/rebrand.md`; docs are ledgers in `doc/`;
the pre-commit hook (`hooks/checks/src/main.rs`) enforces nightly rustfmt, clippy pedantic, typos,
cargo-machete, cargo-deny, `cargo-sort --grouped`, a sorted `src/ui-blueprint-resources.in`, and
rumdl.

## M0 — `cargo check` and `clippy` pass on Windows

Linux and macOS must stay untouched in behaviour.

1. **Inventory the environment.** New `build-aux/windows/probe-env.sh` — MSYS2 is a POSIX
   environment, so the macOS probe's shape carries over (read-only, prints
   `name | found | version | required | status`, ends with "needs installing/building:"). It
   reports: `pkg-config --modversion`/`--atleast-version` for every dependency in
   `meson.build:25-48`; `gst-inspect-1.0` for `gtk4paintablesink`, `mfvideosrc`,
   `mfdeviceprovider`, `wasapisink`/`wasapi2sink`, `webrtcbin`, `nice`, `dtlssrtpenc`, `srtp`,
   `playbin3`, `uridecodebin3`, `level`, `videoconvert`, `audioconvert`, `webpdec`; whether gtk4's
   GStreamer **media backend** module is installed (`$MINGW_PREFIX/lib/gtk-4.0/*/media/`) — macOS
   needed `gst_media_stream` because its GTK lacked one, and this decides whether that seam widens;
   the libshumate **vector renderer** (`grep ShumateVectorRenderer` in the `.gir`, or
   `objdump -p libshumate-1.0-1.dll`); pixbuf loaders and cache; the gio modules dir and a TLS
   module; what `glib::user_data_dir()`/`user_cache_dir()` actually return on this machine; the
   schemas/icons/mime/gtksourceview data layout; tools: meson ≥ 1.4, ninja,
   glib-compile-resources/schemas, blueprint-compiler ≥ 0.18, cargo + rustc (UCRT64), rustup
   nightly rustfmt, cargo-nextest, cargo-deny, cargo-machete, cargo-sort, typos, rumdl, grass or
   sassc, ntldd, rsvg-convert, icotool or magick, `wix` (dotnet tool), signtool; and whether
   `LongPathsEnabled` is set. Paste the output into `doc/windows.md`.
2. **Gate the build system at Linux, not at not-macOS.** `meson.build:40-45`: the glycin
   dependencies move from `host_machine.system() != 'darwin'` to `== 'linux'`.
   `data/meson.build:6,96`: same change for the desktop-file block (and its validate test) and the
   D-Bus service block; metainfo and the gschema stay everywhere (the schema is required at
   runtime, `src/application.rs` aborts without it). `gnome.post_install`'s
   `update_desktop_database` gate likewise becomes linux-only. Reword the comments that say
   "macOS" to name both platforms.
3. **gettext.** Extend the darwin-only `GETTEXT_SYSTEM=1 GETTEXT_DIR=` block in
   `meson.build:112-126` to windows with the same `fs.exists(libintl.h)` guard — UCRT64 ships
   libintl, and without the variables `gettext-sys` builds a vendored static copy.
4. **Secrets.** New `src/secret/windows.rs`; the `cfg_if!` in `src/secret/mod.rs` gains a
   `windows` branch (`Secret = windows::WindowsSecret`); `unimplemented` stays for other OSes.
   The structure mirrors `macos.rs` — same `SecretExt` surface, everything through
   `spawn_tokio!`:
   * Generic credentials via the `windows` crate (`Win32_Foundation`,
     `Win32_Security_Credentials`): `CredWriteW` with `CRED_TYPE_GENERIC`,
     `TargetName = "{APP_ID}/{session id}"` (`APP_ID` is already profile-distinct),
     `UserName = user_id`, `CRED_PERSIST_LOCAL_MACHINE` (the per-user store that survives
     reboots; "local machine" names the scope of roaming, not of visibility).
   * The payload is the same `version: 1` serde_json document the macOS Keychain stores —
     homeserver, user id, device id, session id, client id, passphrase — because the Credential
     Manager, like the Keychain, has no free-form attribute search worth using. The struct is
     duplicated into `windows.rs` rather than shared: the two backends should be free to diverge.
   * `restore_sessions`: `CredEnumerateW` with filter `"{APP_ID}/*"`; `ERROR_NOT_FOUND` →
     `Ok(vec![])`; skip entries with bad JSON or a newer version (log). Blob bytes are copied out
     and zeroized after parsing.
   * `delete_session`: `CredDeleteW`. Errors → `SecretError::Service(message)`.
   * `CredentialBlob` is capped at 2560 bytes (`CRED_MAX_CREDENTIAL_BLOB_SIZE`); the payload is
     roughly 400. Assert it before writing anyway — a silent truncation here is a lost account.
   * Tokens stay in `SecretFile` (unchanged).
5. **Data dirs** (now, before any Windows user data exists). `src/utils/mod.rs`: a
   `cfg(target_os = "windows")` branch that puts both types under one folder, the way Windows
   applications do it: `%LOCALAPPDATA%\{PROFILE.dir_name}\data` and
   `…\{PROFILE.dir_name}\cache` (`glib::user_data_dir()` _is_ `%LOCALAPPDATA%`; the cache
   answer is the one that must not be used as-is). This needs `dir_path()` itself to gain a
   windows arm, since the profile segment sits in the middle rather than at the end. Record the
   paths in `doc/rebrand.md`.
6. **No console window in release.** `src/main.rs` gains
   `#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]`
   — development builds keep the console (that is where tracing goes), release builds stop
   flashing one.
7. `deny.toml` `[graph] targets` gains `x86_64-pc-windows-gnu`.
8. `Cargo.toml`: new `[target.'cfg(target_os = "windows")'.dependencies]` table with the
   `windows` crate (start with the credential features; M5 adds the WinRT ones), sorted after the
   macOS table; run the cargo-sort check.
9. **Housekeeping the first build will hit:** `git config core.longpaths true` and the
   `LongPathsEnabled` registry value (cargo target dirs under `_build` go deep); scripts and
   hooks stay LF (add a `.gitattributes` only if something actually bites, and record it).

The known risk in this milestone is **`aws-lc-sys`** (behind matrix-sdk's `rustls-aws-lc-rs`
feature) on `x86_64-pc-windows-gnu`: it wants cmake and a working C toolchain, both a `pacman -S`
away, but the target is not its happiest path. If it will not build, the fallback is switching the
SDK's crypto-provider feature to ring for this target — verify the feature name against the pinned
SDK revision before assuming it exists.

**It was not a risk at all.** With cmake installed the whole dependency tree, `aws-lc-sys`
included, compiled without a word. No fallback was needed and none was written.

Verify from a UCRT64 shell: `sh build-aux/windows/probe-env.sh`;
`meson setup _build -Dprofile=development`;
`CARGO_TARGET_DIR=_build/cargo-target cargo check && cargo clippy --all-targets -- -D warnings`;
`cargo +nightly fmt --check --all`; `cargo deny check`; `cargo machete --with-metadata`; the
cargo-sort check; `typos`; `rumdl check .`. Verify on Linux and macOS: clippy still clean,
behaviour unchanged.

## M1 — runs from the Meson dev build

Goal: log in, timeline images work.

1. Install the missing packages (exact list in [Dependencies](#dependencies-and-recipes)).
2. `meson setup _build -Dprofile=development --prefix=$MINGW_PREFIX` and
   `meson install -C _build` — installing into the UCRT64 prefix is normal MSYS2 practice, puts
   the app schema and icons where GLib already looks, and `gnome.post_install` compiles the
   schemas. No environment variables needed for a dev run.
3. **Fix what the first run reveals**, each its own commit. Candidates already visible:
   * **GTK media backend**: if the probe found no GStreamer media backend module, widen the macOS
     `gst_media_stream` seam — the `cfg(target_os = "macos")` in
     `src/components/media/{mod.rs,audio_player/mod.rs,content_viewer.rs}` becomes
     macOS-or-Windows. **Needed: there is no `lib/gtk-4.0` directory at all.** The seam was
     widened; nothing in `GstMediaStream` turned out to be macOS-specific.
   * **Shumate vector renderer**: if the packaged 1.5.1 lacks it, build libshumate 1.6 in UCRT64
     (recipe below). **Not needed: the packaged 1.5.1 has it.**
   * **GSK renderer quirks**: note which renderer win32 picked; if rendering glitches,
     `GSK_RENDERER=gl` is the first lever. Record either way.
   * **The `visual_media_row_model` test** is already `cfg(all(test, not(target_os = "macos")))`
     — if it fails here for the macOS reason, the cfg widens.
4. **Answer the GApplication uniqueness question now**, because M3 depends on it: Windows has no
   session bus, so a second `commune.exe` may become a second primary instance instead of
   forwarding to the first — which is what `matrix:` URL activation into a running app needs.
   Options if it is broken: MSYS2 packages `dbus`, and GDBus on win32 knows the `autolaunch:`
   address; or a named mutex plus a local socket in `main()` that forwards the command line.
   Thirty minutes of experiment, recorded in `doc/windows.md`, decides which.

   **Answered: it works, and so does the URI forwarding on top of it.** A second invocation with
   no arguments exits on its own and leaves one window. A second invocation carrying
   `matrix:u/alice:example.org` reached the first instance's `Application::open`, came out as
   `ShowMatrixId(User("@alice:example.org"))`, and was refused only for want of a logged-in
   session. So none of the fallbacks are needed, and the warm path of [M3](#m3--signed-wix-5-msi)
   is done before its milestone starts — what is left there is the registry key for the cold one.

   One caution for whoever repeats this: a first instance that fails to start makes the second
   one look like a second primary, because it is one. Check that the first is actually running
   before concluding anything.

Verify: password login; SSO login (`src/login/local_server.rs` binds localhost — expect a
Defender firewall prompt); send and receive text; image thumbnail, animated GIF, sticker pack;
video plays; a voice message plays (WASAPI); the Credential Manager control panel shows the
`{APP_ID}/…` entry; quit and relaunch restores the session; `meson test -C _build` passes. Calls:
with `webrtcbin`/nice/dtls/srtp all present, run the `doc/calls.md` smoke against Element and
record what happens.

## M2 — relocatable, signed folder and `.zip`

```text
Commune/
  bin/commune.exe                    # + every non-system DLL, flat beside it
  bin/gst-plugin-scanner.exe
  lib/gdk-pixbuf-2.0/2.10.0/{loaders/*.dll, loaders.cache (relative paths)}
  lib/gstreamer-1.0/*.dll            # needed subset
  lib/gio/modules/*.dll              # glib-networking TLS
  share/commune/{resources,ui-resources}.gresource
  share/locale/*/LC_MESSAGES/commune.mo
  share/glib-2.0/schemas/gschemas.compiled
  share/icons/{hicolor (app svg), Adwaita (symbolic subset + index.theme)}
  share/gtksourceview-5/, share/mime/
```

Windows does most of the relocation work that macOS had to do by hand: GLib's win32 build derives
its data directories from the location of the GLib DLL itself, and gdk-pixbuf, GIO modules and
GStreamer follow the same convention — so the bundle needs few or none of the environment
variables `app_bundle.rs` sets on macOS. Expect to set nothing; let the bundled run prove it.

1. **Runtime paths.** `src/utils/app_bundle.rs` gains a `windows` module beside the macOS one:
   if `current_exe()` sits in a `bin\` directory with a sibling
   `share\commune\resources.gresource`, the root is that prefix — `RuntimePaths` relative to it;
   otherwise the compile-time constants (the dev build installed into `$MINGW_PREFIX`, where the
   constants are correct). Environment variables only if the bundled run shows a library that did
   not relocate itself.
2. **`build-aux/windows/bundle.sh`** (UCRT64, beside its macOS namesake; the house's
   `bundle-gtk-windows.ps1` is prior art, but `ntldd` is right here): `DESTDIR` -staged
   `meson install`, then assemble the tree above; a recursive `ntldd -R` walk from `commune.exe`
   and every plugin DLL, copying the closure into `bin\`; then an audit loop asserting every
   import resolves inside the bundle or `C:\Windows` — the same role the `otool -L` audit plays
   on macOS, and just as much the point. Rewrite `loaders.cache` to relative paths; trim the
   GStreamer set to the subset the macOS `bundle.sh` established.
3. **Icon and version resources.** `build-aux/windows/make-ico.sh`: `rsvg-convert` at
   16/24/32/48/64/128/256 from `assets/appicon.svg` and `assets/appicon-devel.svg` → `icotool`
   → `commune.ico` (derived artifact, not committed). A new `build.rs` (no-op off Windows) with
   the `winresource` build-dependency embeds the icon, `VERSIONINFO`, and an application manifest
   declaring UTF-8 as the active code page and `longPathAware` — the icon is what Explorer and
   the taskbar show, and there is no bundle metadata file on Windows to carry it instead.
4. **Signing.** `build-aux/windows/sign.ps1`, adapted from `seed-sync-gtk`'s
   `sign-artifacts.ps1`: `signtool` with the Azure Trusted Signing dlib over `commune.exe` after
   bundling. The `.zip` is the signed folder archived (`.zip` itself cannot carry a signature —
   the `.msi` in M3 is the artifact that does).
5. `meson.build`: a `host_machine.system() == 'windows'` block with `run_target`s
   `windows-bundle` and `windows-zip`, mirroring the darwin block at `meson.build:199-215`.
   `.gitignore` gains the outputs.

Verify: run the bundle from the Desktop in a shell with no MSYS2 on `PATH`; Process Explorer (or
`listdlls`) shows nothing loaded from `C:\msys64`; the M1 smoke list passes from the bundle;
`signtool verify /pa` passes; extract and run inside **Windows Sandbox** — the free clean-machine
test, and where SmartScreen shows what a stranger would see.

## M3 — signed WiX 5 `.msi`

`build-aux/windows/{commune.wxs, build-msi.ps1}`, adapted from the seed-sync-gtk pipeline (WiX 5
as a dotnet tool).

1. **Per-user install** (no UAC, winget-friendly later), harvesting the M2 bundle directory
   wholesale. ARP metadata: name, publisher, version, `commune.ico`.
2. **Stable `UpgradeCode` per profile** — Devel and Stable must differ so they can coexist, the
   same way the app ID carries `.Devel`. Both GUIDs go in `doc/rebrand.md` the moment they are
   generated; an UpgradeCode is forever.
3. **Start Menu shortcut carrying `System.AppUserModel.ID = APP_ID`.** Toasts (M5) require an
   AUMID-bearing shortcut; setting it now means M5 changes no installer. A `ToastActivatorCLSID`
   slot is documented as a placeholder and wired in M5.
4. **`matrix:` URL scheme**: `HKCU\Software\Classes\matrix` → `URL Protocol`,
   `shell\open\command` → `"…\bin\commune.exe" "%1"`. The app side is `HANDLES_OPEN`/`open()`
   (`src/application.rs`), which already works — the cold path is registry-only. The warm path
   (second launch forwards to the running instance) is whatever M1's uniqueness experiment
   concluded; if forwarding is unavailable, the scheme still registers and a second instance
   opens, recorded as a known gap.
5. Sign the `.msi` with the same pipeline.

Verify: install; launch from the Start Menu; a `matrix:` link from a browser opens the app cold,
and warm per the M1 finding; upgrade over a previous version works; uninstall removes files and
registry keys and leaves `%LOCALAPPDATA%` data behind (deliberately — say so in `doc/windows.md`);
Devel and Stable install side by side.

## M4 — polish

1. **Dark mode.** libadwaita ≥ 1.6 follows the system color scheme on Windows. Verify a toggle
   of Settings → Personalization → Colors flips the app live, and record it — zero work is the
   expected answer.
2. **Clock format**, dropped on macOS because it was expensive there, is cheap here if appetite
   remains: `src/system_settings/windows.rs` reading
   `HKCU\Control Panel\International\sShortTime` (an `H` means 24-hour), behind the existing
   `SystemSettings` seam. Optional; the graceful degradation already matches macOS.
3. **Small-stuff sweep**, each verified rather than assumed: the taskbar and title bar show the
   embedded icon; drag-and-drop of a file onto the composer; IME input (the Japanese IME is the
   usual canary); file save dialogs default somewhere sensible; the shortcuts dialog shows Ctrl
   everywhere (it should already — `PRIMARY_MASK` did this in the macOS port).

## M5 — notifications via WinRT toasts

**Done.** Banners arrive with the sender's avatar, and clicking one opens the room it names —
whether or not Commune is running. `doc/windows.md` records what was built. Three things went
differently from the sketch below, all noted in place: the AUMID has to be claimed in process and
not merely declared by the installer, `tauri-winrt-notification` was rejected because it cannot
withdraw a notification, and `CustomActivator` in the registry replaced the shortcut property so
that an unpacked `.zip` behaves like an installed copy.

The macOS M5 experiments transfer almost one-to-one; reread them before starting
(`doc/macos-plan.md` M5, `doc/macos.md`).

* **Step 0 stays thirty seconds:** from a bundle, send a test notification through the real path
  (`COMMUNE_TEST_NOTIFICATION`, described in `doc/macos.md`). GLib has **no** win32
  `GNotification` backend, so the expected result is silence with no error — record it and move
  on. There is no version of "maybe GLib handles it" to chase here.

  **Answered, and the premise was wrong in both directions.** GLib does have a backend, and it is
  not a legacy one: `strings libgio-2.0-0.dll` names `GWin32NotificationBackend`, and the only
  other things it names beside it are `RoActivateInstance` and
  `api-ms-win-core-winrt-l1-1-0.dll` — so it is WinRT toasts, the same modern API this milestone
  was going to reach for. That is the opposite of macOS, where the backend existed but was
  deprecated past usefulness.

  What it will not do is carry an action:

  ```text
  GLib-GIO-WARNING: Notification actions are unsupported by this Windows backend
  ```

  That is precisely the half Commune depends on. Every notification it sends sets a default action
  with a `GVariant` target and exists to be clicked, so a banner that cannot be clicked is not a
  notification we can ship.

  And nothing was delivered: after a test notification, `HKCU\…\Notifications\Settings` lists 34
  applications and Commune is not among them — the key Windows creates when an application first
  delivers a toast. Launching from the MSI's AUMID shortcut did not change that.

  **Why, and it is not the shortcut.** An unpackaged process has no AUMID of its own —
  `GetApplicationUserModelId` returns `APPMODEL_ERROR_NO_APPLICATION` — and nothing in Commune
  ever gives it one. A shortcut declaring an AUMID is what makes the ID *valid to register
  against*; the running process still has to claim it.

  There is a worked precedent on this machine, in a sibling project of the same author
  (`~/irohdp`, `crates/ipn-gui/src/notify.rs`), and it is worth reading before writing any of
  this. Nullgate **is** in that registry key, and what it does is two things Commune does not:

  * `SetCurrentProcessExplicitAppUserModelID(APP_ID)` from `shell32`, early, so the process claims
    the ID.
  * an `HKCU\Software\Classes\AppUserModelId\{APP_ID}` key with a `DisplayName`, which is what
    Windows shows in its notification settings — written by the app itself, so it does not depend
    on having been installed.

  It then sends toasts with `tauri-winrt-notification` rather than through GLib, with
  `on_activated` for the click, and its comment records a second reason to bypass GLib that this
  investigation did not reach: GLib's backend "spawns a confusing second notification-area icon
  beside the tray icon".

  So the shape below is right, but two of its assumptions are not: the AUMID has to be claimed in
  process rather than merely declared by the installer, and `tauri-winrt-notification` is a
  shorter route to the same place than hand-rolling `ToastNotificationManager` — already proven
  here, on this Windows, by this author.
* New `src/utils/windows_notifications.rs` behind the existing dispatch in
  `src/session/notifications/mod.rs` — the two-way macOS `cfg_if!` at `:141` and `:162` becomes
  three-way. Linux keeps `GNotification` untouched.
* `windows` crate WinRT features (`UI_Notifications`, `Data_Xml_Dom`):
  `ToastNotificationManager::CreateToastNotifierWithId(APP_ID)` — the AUMID the M3 shortcut
  declared; toast XML with title and body; the sender's avatar written to
  `<cache>\notification-icons\<uuid>.png` exactly as on macOS, referenced as a `file:///` image
  (works for unpackaged apps); the notification identifier becomes Tag + Group, so withdraw is
  `ToastNotificationHistory.RemoveGroupedTagWithId` — the same mapping shape as macOS.
* **Taps with the app running:** the `Activated` event on the toast object, in-process.
* **Taps that launch the app:** a COM activator — `INotificationActivationCallback` implemented
  with `windows-implement`, its CLSID registered under `HKCU\…\CLSID\{GUID}\LocalServer32` and as
  `ToastActivatorCLSID` on the shortcut (the M3 placeholder, wired now; the GUID goes in
  `doc/rebrand.md`). The intent rides as `glib::Variant::print(true)` in the toast's launch
  arguments and is parsed back against `action_parameter_type()` — the same single-representation
  rule the macOS port established.
* **Toasts require the installed app.** The bare `.zip` has no AUMID shortcut, so it gets no
  toasts; document that as the trade rather than growing a first-run shortcut writer. **Worth
  re-testing rather than assuming**, now that the AUMID turns out to be claimed in process and the
  `AppUserModelId` class key can be written by the app itself: the `.zip` may need nothing from an
  installer after all.
* One thing macOS fought that does not exist here: signing identity churn. The Azure certificate
  is stable, so notification permission survives rebuilds by construction.

Verify: the macOS M5 list, translated — a message with the window backgrounded raises a toast
with the avatar; clicking it opens the right room in the right session, warm and cold; a
notification for a room that is read disappears; nothing asks twice across two rebuilds.

## M6 — stretch: camera QR scanning

The macOS M4 sketch with the Media Foundation source swapped in — none of the macOS one exists
yet either, so whichever platform goes first writes the pattern.

1. New `src/components/camera/windows/` (or a shared `gst/` module if the macOS one lands first —
   the pipelines differ only in the source element); `camera/mod.rs` `cfg_if!` gains the branch.
   Contract: `CameraExt` and a `CameraViewfinder` subclass mirroring `camera/linux/viewfinder.rs`;
   `has_cameras()` via `gst::DeviceMonitor` (`mfdeviceprovider`).
2. Pipeline: `mfvideosrc ! videoconvert ! tee`, one branch to `gtk4paintablesink`, one leaky
   branch to GRAY8 `appsink`; `rqrr` on `spawn_blocking`, throttled to about 5 fps. `rqrr` joins
   the windows target table.
3. Camera consent on Windows is Settings → Privacy → Camera → "Let desktop apps access your
   camera"; nothing to declare at build time, but the `Error` state should tell the user where to
   look when the source fails to open.

Verify: Verification → Scan QR shows the camera; scanning another client's QR completes; the
camera light goes out on dialog close.

## Dependencies and recipes

Everything from the UCRT64 repo, one command:

```sh
pacman -S --needed \
  mingw-w64-ucrt-x86_64-gtksourceview5 mingw-w64-ucrt-x86_64-libshumate \
  mingw-w64-ucrt-x86_64-gst-plugins-base mingw-w64-ucrt-x86_64-gst-plugins-good \
  mingw-w64-ucrt-x86_64-gst-plugins-bad mingw-w64-ucrt-x86_64-gst-plugins-ugly \
  mingw-w64-ucrt-x86_64-gst-plugins-rs mingw-w64-ucrt-x86_64-libnice \
  mingw-w64-ucrt-x86_64-glib-networking mingw-w64-ucrt-x86_64-blueprint-compiler \
  mingw-w64-ucrt-x86_64-meson mingw-w64-ucrt-x86_64-ninja mingw-w64-ucrt-x86_64-librsvg \
  mingw-w64-ucrt-x86_64-icoutils mingw-w64-ucrt-x86_64-ntldd mingw-w64-ucrt-x86_64-cmake \
  mingw-w64-ucrt-x86_64-gettext-tools
```

Already present: gtk4, libadwaita, glib2, gstreamer, adwaita-icon-theme, Rust 1.97 (all UCRT64).
Cargo tooling under the UCRT64 Rust: `cargo install cargo-nextest cargo-deny cargo-machete
cargo-sort typos-cli grass` (pacman where a package exists); rumdl likewise. From the Windows
side: rustup nightly for rustfmt, the `wix` dotnet tool and the Azure signing configuration —
both already on this machine for seed-sync-gtk, whose `artifact-signing-metadata.json` shape is
the reference.

The only from-source recipe expected: **libshumate**, if the probe finds the packaged 1.5.1
without `ShumateVectorRenderer` — tag 1.6.0, `meson setup _b --prefix=$MINGW_PREFIX
-Dvapi=false -Dgtk_doc=false -Dvector_renderer=true` (protobuf-c is packaged). Every recipe the
macOS port needed from source — the gtk4 GStreamer sink, blueprint-compiler, glib-networking —
is a package here.

## Docs and ledger updates

* `doc/windows.md` (new, in the voice of `doc/macos.md`): probe output, the UCRT64-shell
  convention, build/run/bundle/sign/msi commands, the Credential Manager entry shape, what is
  stubbed (location, clock unless M4.2 lands) and unsupported (same image-format list as macOS:
  SVG, HEIC, AVIF, JXL in the timeline), the uniqueness finding, rebase notes.
* `doc/rebrand.md`: the identifiers table gains the AUMID (= `APP_ID`), both MSI UpgradeCodes,
  the toast-activator CLSID, the `matrix:` registry keys, the credential `TargetName` prefix
  `{APP_ID}/`, the data paths `%LOCALAPPDATA%\commune[-Devel]\{data,cache}`, and `commune.ico`
  derived from `assets/appicon*.svg`; the renamed-files list gains `build-aux/windows/*` and
  `src/secret/windows.rs`.
* `README.md`: Building gains a Windows subsection pointing at `doc/windows.md`; Runtime
  dependencies gains a Windows paragraph. `CONTRIBUTING.md`: a one-line pointer.

## Risks and open questions

* **GApplication uniqueness without a session bus** — the biggest unknown, answered by experiment
  in M1 because the `matrix:` warm path and single-instance behaviour hang on it.
* **`aws-lc-sys` on `x86_64-pc-windows-gnu`** — fallback is the SDK's ring provider, feature name
  to be verified against the pinned revision.
* Whether MSYS2's gtk4 ships a GStreamer media backend, and whether its libshumate has the vector
  renderer — both probed in M0, both with known fixes.
* GTK win32 backend quirks (GSK renderer choice, IME, popovers on separate surfaces — the macOS
  sticker-picker story could have a win32 sibling) — track in `doc/windows.md`.
* **MSYS2 is a rolling release.** A `pacman -Syu` can move GTK mid-port. Update deliberately,
  and record the version set in `doc/windows.md` when packaging.
* windows-gnu debugging is gdb-flavoured; the MSVC tools read its DWARF poorly. Accepted with the
  toolchain choice.
* The `.zip` hand-out gets no toasts (no AUMID shortcut) and SmartScreen treats archives worse
  than installers even signed — the `.msi` is the recommended hand-out from the start.

## Critical files

* `src/secret/mod.rs` (plus new `src/secret/windows.rs`)
* `src/utils/mod.rs` (data dirs), `src/utils/app_bundle.rs`, `src/main.rs`, new `build.rs`
* `src/session/notifications/mod.rs` (plus new `src/utils/windows_notifications.rs`)
* `src/components/media/{mod.rs,audio_player/mod.rs,content_viewer.rs}` (if the media-backend
  seam widens)
* `Cargo.toml`, `deny.toml`, `meson.build`, `data/meson.build`
* new `build-aux/windows/{probe-env.sh,bundle.sh,make-ico.sh,sign.ps1,commune.wxs,build-msi.ps1}`
* `doc/windows.md` (new), `doc/rebrand.md`, `README.md`

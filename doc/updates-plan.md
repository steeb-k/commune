# Plan: automatic updates and CI-built releases

This is the working plan for letting an installed Commune find, fetch and install its next
version, and for moving the builds it installs from a developer's machines onto GitHub Actions.
Written on 11 September 2026 before any of it was built. When work starts, `doc/updates.md`
becomes the ledger of what exists; this file records the intended route and the decisions
behind it. The maintainer's choices from the 11 September interview are folded into the
numbered decisions; what remains open is settled when a milestone reaches it.

## Contents

<!-- toc -->
* [Context](#context)
* [Decisions](#decisions)
* [The feed](#the-feed)
* [The core module](#the-core-module)
* [Windows](#windows)
* [macOS](#macos)
* [Android](#android)
* [Linux](#linux)
* [The user interface](#the-user-interface)
* [Continuous integration](#continuous-integration)
* [Where this got to](#where-this-got-to)
* [Milestones](#milestones)
* [Risks and open questions](#risks-and-open-questions)
* [Critical files](#critical-files)
<!-- /toc -->

## Context

What exists today, per platform, as surveyed on 11 September 2026:

* **Windows** ships a per-user WiX 5 `.msi` (`build-aux/windows/commune.wxs`): no UAC on
  install or upgrade, one permanent `UpgradeCode` per profile (`build-msi.ps1:74-78`), a default
  `MajorUpgrade` with `AllowSameVersionUpgrades`, no custom actions, no launch conditions.
  Nothing in it blocks `msiexec /i … /qn`. The exe and the MSI are Authenticode-signed through
  Azure Trusted Signing (`sign.ps1`), authorised today only by an `az login` session because
  `artifact-signing-metadata.json` excludes every other credential type. The MSI version is
  `Cargo.toml`'s semver with the pre-release stripped (`1.0.0`); the app reports Meson's
  `1.rc1`, plus `-<sha>` in the Devel profile.
* **macOS** ships `Commune.app` in a `.dmg` or `.tar.gz` (`build-aux/macos/`), arm64 only,
  relocatable, runs the binary directly. Every dylib comes from conda-forge, which is why the
  bundle's floor is macOS 11 regardless of the building machine (`doc/macos.md:75-79`). A
  Developer ID identity exists (team `VLC2KZKNBH`) but its private key is Xcode cloud-managed and
  cannot be exported (`doc/macos.md:465-468`); default builds are ad-hoc signed. Notarization is
  implemented in `make-dmg.sh` and staples the ticket to the `.dmg`. `CFBundleVersion` is already
  the commit count, chosen "for any updater later" (`bundle.sh:476-481`).
* **Android** (the Kotlin app) is sideloaded with `adb`, signed with a debug keystore
  (`app/build.gradle.kts:58-69`), `versionCode = 1` and `versionName = "0.1.0"` hard-coded. It
  has no HTTP library of its own, no WorkManager, no About screen and never shows a version.
  The Pixel's installed copy carries the WSL debug key, which any new signing key cannot update
  in place.
* **Linux** ships as a Flatpak; updates are the Flatpak remote's job.
* **The app** has no updater scaffolding of any kind, no restart path, no shutdown override,
  and one non-Matrix HTTP helper: `commune_core::http::fetch` (reqwest over rustls, size-capped),
  used only by the GIF search. Global settings are a single `gio::Settings`; the app-wide
  toggles already live in the Account Settings General page. About is an `adw::AboutDialog`
  with no release notes.
* **CI**: none that runs. `.gitlab-ci.yml` is inherited and never executes on GitHub. There is no
  `.github/` directory. There are no tags and no GitHub releases yet.
* Crates already in `Cargo.lock` that the updater can use without a new dependency:
  `ed25519-dalek` (via vodozemac), `semver`, `sha2`, `flate2`, `windows` 0.62. Not present:
  `tar`, `zip`.

## Decisions

Made here, with the reasoning; each is reversible until the milestone that builds it lands.

1. **One feed, one verifier, in `commune-core`.** Fetching the manifest, checking its signature,
   comparing versions and choosing the asset for the running platform is portable logic, so it
   goes in the core and reaches the Kotlin app through UniFFI like everything else. Only the
   last step — putting the new build on disk and relaunching — is per platform and per UI.
2. **A static manifest, not the GitHub API.** Unauthenticated API calls are limited to 60 per hour
   per source IP, shared by everyone behind one NAT, and the API is the wrong dependency for a
   check that runs on every launch. The release workflow writes a small JSON manifest per channel
   and publishes it at a fixed URL; the app reads that and nothing else.
3. **The manifest is signed with an Ed25519 key whose public half is compiled into the app.** OS
   signatures (Authenticode, Developer ID, the APK signer) already stop a tampered binary from
   installing. The manifest signature stops a tampered _feed_ from pointing at an older signed
   build or at a build for the wrong channel. Verification uses `ed25519-dalek`, already linked.
4. **Semver is the comparison spelling.** `Cargo.toml`'s `1.0.0-rc1` is the one form that
   orders correctly with an off-the-shelf comparator. The app compares the manifest's
   `version` against its own `CARGO_PKG_VERSION`; Meson's `1.rc1` stays the display form.
5. **Windows installs by running the downloaded MSI.** Per-user scope and the existing
   `MajorUpgrade` make `msiexec /i <msi> /passive` a complete upgrade. The app quits first so
   no file is in use; a detached shell relaunches the installed exe when `msiexec` returns.
6. **macOS installs from the `.tar.gz`, by swapping the bundle.** Tarballs extracted by the app
   carry no quarantine attribute, the signature travels inside the bundle, and a rename swap is
   atomic. The new bundle must pass `codesign --verify --strict` and carry the same Team ID as
   the running one before it replaces anything. Ad-hoc builds never update.
7. **Android installs through the system package installer.** The app downloads the APK and hands
   it to `PackageInstaller`; Android enforces that the signer matches. This needs a stable
   release key, which is a prerequisite, not a feature.
8. **Flatpak never self-updates.** The Updates group says so and links nowhere; the check itself
   is skipped, not merely hidden.
9. **Three channels: `stable`, `rc`, `nightly`.** Tags feed the first two; every push to `main`
   feeds `nightly` with Devel-profile builds. An installed Devel build defaults to the nightly
   channel, which is what replaces the maintainer's manual build loop. A build that is not
   installed — running from a build directory, or ad-hoc signed on macOS — never checks, so a
   developer's tree is never replaced by CI's.
10. **Builds move to GitHub Actions.** Release builds run from a tag; the macOS floor of 11.0
    survives the move because conda-forge, not the runner, sets it (see [macOS](#macos)).
11. **Automatic checks are on by default** outside Flatpak. These builds have no store to update
    them; the check is one small signed file a day, and a switch turns it off. Download and
    install are always a click, never automatic.
12. **The manifest key is a GitHub Actions secret**, so a release is one step: push the tag.
    The consequence is stated under risks and accepted.
13. **macOS signs in CI with a second Developer ID Application certificate** under the same
    team, issued from a CSR generated on this PC. Same Team ID means the same designated
    requirement, so Keychain items stored by builds signed with the existing certificate stay
    readable without a prompt.
14. **The macOS release is one universal binary.** Two runner jobs build arm64 and x86_64
    bundles from the existing recipe; a merge job `lipo`s them, then signs, notarizes, staples
    and tars the result once.
15. **Windows upgrades with `/passive`**: a progress window, no questions, then relaunch.
16. **The feed is served from an `updates` branch** through `raw.githubusercontent.com`.

## The feed

One JSON document per channel, published at

```text
https://raw.githubusercontent.com/steeb-k/commune/updates/<channel>.json
```

on an orphan `updates` branch that only the release workflow commits to. `raw.githubusercontent`
has no per-IP API quota, needs no Pages configuration, and starts working the moment the
repository is public. Until then the URL 404s, which the app treats as "no update", and a
developer points `COMMUNE_UPDATE_FEED` at a local file or server to test.

Shape (illustrative; the exact fields are fixed in M1):

```json
{
  "channel": "stable",
  "version": "1.0.0",
  "build": 4021,
  "published": "2026-10-02T18:40:00Z",
  "notes_url": "https://github.com/steeb-k/commune/releases/tag/v1",
  "assets": {
    "windows-x86_64-msi": { "url": "…/Commune-1.0.0-x64.msi", "sha256": "…", "size": 91234567 },
    "macos-aarch64-tar":  { "url": "…/commune-1-arm64.tar.gz", "sha256": "…", "size": 123456789 },
    "android-arm64-apk":  { "url": "…/commune-1-arm64-v8a.apk", "sha256": "…", "size": 45678901 }
  }
}
```

The signature is a detached `<channel>.json.sig` next to it (raw Ed25519 over the file bytes,
base64). The app fetches both, verifies, then parses. `build` is the commit count — the same
number `CFBundleVersion` already carries — and is what orders two builds of the same `version`
when a nightly channel exists.

Channels: `stable` (plain releases), `rc` (release candidates; `stable` users never see them,
`rc` users also see stable releases), and `nightly` (every push to `main`, Devel profile,
ordered by `build` because `version` does not move between pushes). A tagged release publishes
to `rc` or `stable` by the tag's shape. The Stable- and Devel-profile installs have different
application IDs, data directories and `UpgradeCode`s, so following `nightly` on a Devel install
and `stable` on a Stable install side by side is the normal arrangement, and the channel combo
on a Stable build does not offer `nightly`.

Nightly assets live on one rolling GitHub pre-release named `nightly`, whose assets the
workflow replaces on every run. That moves the `nightly` tag, which is the one exception to
`RELEASING.md`'s rule that a tag is never moved, and `RELEASING.md` says so.

## The core module

`commune-core/src/updates.rs`, feature-gated on nothing (the Kotlin app needs it too):

* `Channel`, `Platform` (derived at compile time from `target_os`/`target_arch`, with the
  Flatpak case decided at runtime by `/.flatpak-info` existing — the first Flatpak detection
  in the tree), `Manifest`, `Asset`, `UpdateCheck { current, available: Option<Manifest> }`.
* `check(channel, feed_base) -> Result<UpdateCheck, UpdateError>`: fetches manifest and
  signature with `http::fetch` (size cap 64 KiB), verifies against `PUBLIC_KEY`, compares with
  `semver` then `build`, picks the asset for `Platform::current()`.
* `download(asset, dest_dir, progress) -> Result<PathBuf, UpdateError>`: streams to a temp file
  in the cache dir, enforces `size`, checks `sha256`, renames into place. Uses the existing
  `CLIENT`, adding a streaming variant of `fetch` with a progress callback.
* Settings: `UpdateSettings { check_automatically, channel, last_check, skipped_version }`
  persisted in the same store as the other core-owned settings, so the Kotlin app gets them
  for free.
* Exposed through UniFFI under the existing `ffi` feature.

The public key lives in `commune-core/src/updates/key.rs` as a constant; the private key never
enters the tree. Version compare uses the crate's own `CARGO_PKG_VERSION`, which means
`commune-core`'s version must track the application's (today `0.1.0` against `1.0.0-rc1`) —
M0 aligns them and `RELEASING.md` gains the line.

## Windows

* Asset: the Stable-profile `.msi`. The Devel profile has its own `UpgradeCode`, so a Devel
  channel would work mechanically, but see the open question.
* Install: after `download` succeeds, `src/utils/windows_update.rs` runs

  ```text
  cmd /c start "" /wait msiexec /i "<msi>" /passive & start "" "<install root>\bin\commune.exe"
  ```

  detached (`CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`), then activates `app.quit`. The
  install root is `app_bundle::windows::install_root()`. Before running it, verify the MSI's
  Authenticode signature with `WinVerifyTrust` (`windows` crate, feature
  `Win32_Security_WinTrust`) so a download that passed sha256 but is unsigned is still refused.
* The running exe is not locked against `MajorUpgrade` because the app has already quit; the
  toast-activator COM server (`windows_toast_activator.rs`) re-registers `LocalServer32` on the
  next launch, so no registry work is needed.
* Failure surfaces as a toast on the next launch rather than a console, because the release exe
  has no stderr (`src/main.rs:11-14`); the updater writes its last outcome to the settings store.

## macOS

* Asset: `.tar.gz`, produced by `make-tarball.sh` from a Developer ID-signed, notarized bundle.
  Today notarization staples the `.dmg`; the tarball path needs `xcrun stapler staple
  Commune.app` **before** `tar`, so the ticket travels with the bundle and Gatekeeper is happy
  offline. `make-dmg.sh` keeps its own stapling.
* Install (`src/utils/macos_update.rs`): extract into a sibling temporary directory of the
  running bundle (`flate2` plus a small tar reader — the `tar` crate is the one new dependency,
  or extraction shells out to `/usr/bin/tar`, which every macOS has); run
  `codesign --verify --deep --strict` and `codesign -dv` and require the Team ID to equal the
  running bundle's (read from `Info.plist`/`SecCode`); rename the old bundle to
  `~/.Trash/Commune <old version>.app` (or the cache dir when the Trash refuses), rename the
  new one into place, `touch` it and run `lsregister -f` for the icon cache
  (`doc/macos.md:600-614`); then `open -n <bundle>` from a detached `/bin/sh -c 'sleep 1; …'` and
  quit. If the bundle's parent directory is not writable, fall back to opening the download in
  Finder with a message.
* Ad-hoc builds: `Platform::current()` reports `macos-adhoc` and the check is skipped — a
  developer bundle must not be replaced by a release build, and the identity change would
  re-prompt every Keychain item (`doc/macos.md:462-465`).
* Older macOS: the floor is conda-forge's `minos` (11.0 on arm64, about 10.13 on x86_64), so a
  GitHub runner using the same `setup-conda-macos.sh` produces the same floor as the Mac does.
  The universal bundle's `LSMinimumSystemVersion` is the higher of the two, 11.0, which
  `bundle.sh` already computes from the Mach-O headers.
* Universal: `setup-conda-macos.sh --universal` already creates the second, `osx-64` env "for
  lipo later"; the `lipo` step itself is new. The arm64 job (`macos-15`) and the x86_64 job
  (`macos-15-intel`) each run the unchanged recipe and upload an ad-hoc-signed bundle. A merge
  job (`build-aux/macos/lipo-bundles.sh`) walks the arm64 bundle, and for every Mach-O file
  that also exists in the x86_64 bundle writes `lipo -create` of the pair over it; everything
  else (resources, schemas, plugin caches, which are architecture-neutral) comes from arm64,
  except `loaders.cache` and the GStreamer registry, which name plugin files and are checked
  for being identical. Signing happens after the merge, because a signature covers all slices
  at once: the merge job re-signs with the real identity in the order `bundle.sh` uses, then
  notarizes, staples the `.app`, and tars. GitHub retires the Intel runner in autumn 2027; the
  x86_64 half then either cross-compiles on arm64 (Rosetta for the from-source trio) or is
  dropped, and the feed's asset key stays `macos-universal-tar` either way.
* Signing in CI: the current identity's key is Xcode cloud-managed and not exportable
  (`doc/macos.md:465-468`), so a second Developer ID Application certificate is issued under the
  same team from a CSR generated on this PC with `openssl` — no Mac involved — and its `.p12`
  plus an App Store Connect API key (for `notarytool`) go into repository secrets. Apple allows
  several Developer ID Application certificates per team. The Keychain ACL on stored sessions
  names the designated requirement, which is Team ID plus bundle ID for Developer ID builds,
  so the certificate swap is silent for existing installs. The `notary` keychain profile on the
  Mac stays valid for local releases.

## Android

* Prerequisite: a release keystore (`android/keystore.properties`, gitignored, backed up
  offline — the shape `doc/android-plan.md:63` already names), used by both the WSL and CI
  builds, base64-encoded into a repository secret for CI. The first build signed with it cannot
  install over the Pixel's debug-keyed copy; that one reinstall loses the adopted session and
  is paid once.
* Version: `versionName` read from the workspace `Cargo.toml`, `versionCode` from
  `git rev-list --count HEAD`, both computed in `app/build.gradle.kts`, so the APK and the
  manifest agree by construction.
* Check: `coreUpdates.check()` through UniFFI on app start and daily, off the main thread, in the
  existing `CommuneState` pattern. `SettingsScreen` gains an "About" group (version — which the
  app has never shown — and an Updates row) and an "Updates" switch.
* Install: `core.download()` into `cacheDir/updates/`, then `PackageInstaller` session
  (`ACTION_INSTALL_PACKAGE` via a `FileProvider` is the simpler path and enough here). The
  manifest needs `REQUEST_INSTALL_PACKAGES` and a `<provider>` for the cache directory; the
  first install prompts the user to allow Commune to install apps.
* Alternative that costs nothing: Obtainium watches GitHub releases and does all of this.
  The in-app path is still worth having for users who do not run Obtainium, but if the Pixel
  is the only Android install for the foreseeable future, M4 can wait. Open question.

## Linux

* Flatpak: `Platform::current()` returns `flatpak` and `check` is not called. The Updates
  group shows one inert row: "Updates arrive through your app store."
* A non-Flatpak Linux build (distribution package, local `meson install`) is treated the same;
  there is no Linux asset and no channel to offer.

## The user interface

GTK, all strings through gettext and the files added to `po/POTFILES.in`:

* **Startup and daily check** in `Application::startup`, after `commune_core::config::init`,
  gated on `check_automatically` and not-Flatpak and not-Devel: `spawn_tokio!(check)` then a
  `glib::timeout_add_seconds_local(86_400, …)` in the `set_up_test_notification` shape.
* **Result**: a toast on `Application::main_window()` — "Commune 1.0 is available" with an
  _Update_ button — once per version, and never while a call is in progress. Skipping is the
  toast's dismissal plus a `skipped_version` write; the row below still offers it.
* **Settings**: an _Updates_ group on the General page (`general_page/mod.blp`, after
  _Composer_): a switch _Check for updates automatically_, a combo _Channel_ (Stable / Release
  candidates; Nightly only if that channel exists), and an action row whose subtitle is the
  last outcome ("Up to date, checked 2 hours ago" / "Commune 1.0 available") with a *Check
  now* / _Update_ button. Download progress goes in the same row's subtitle.
* **Where it lives while logged out**: Account Settings only opens with a session. The greeter
  and error page already carry the About button; the toast still fires on the main window,
  which exists in both states, and About gains a _Check for updates_ row via
  `adw::AboutDialog::add_link` or a custom section. Small; decide during M2.
* **About**: `release_notes` set from the metainfo's current `<release>` text, which Meson
  already has; `release_notes_version` = `config::VERSION`.

## Continuous integration

`.github/workflows/`, replacing nothing (the GitLab files stay inert until a rebase deletes
them):

* `check.yml` — on push and pull request: the pre-commit gate (`hooks/checks`) and
  `cargo test` on Ubuntu in the GNOME SDK container, which is what the Flatpak build already
  needs. This is the first CI the repository has had; `RELEASING.md` currently says the local
  checks are the whole gate.
* `release.yml` — on `v*` tags, four jobs then one publisher:
  * **windows** (`windows-latest`, `msys2/setup-msys2` with `ucrt64`, the `pacman -S` list from
    `doc/windows.md:132-147`, `LongPathsEnabled` via registry, `core.longpaths`): `meson setup
    -Dprofile=default`, `windows-bundle`, then `build-msi.ps1 -Profile Stable` from PowerShell
    with WiX 5 and the Trusted Signing client tools installed in the job. Signing authenticates
    with `azure/login` OIDC through a federated credential on the `skz-code` principal, using a
    CI variant of the metadata JSON that leaves `WorkloadIdentityCredential` enabled
    (`ARTIFACT_SIGNING_METADATA` points at it). MSYS2 is rolling, so the job runs `pacman -Syu`
    once and records the probe table as a job artifact for `doc/windows.md`.
  * **macos-arm64** (`macos-15`) and **macos-x86_64** (`macos-15-intel`): each runs
    `setup-conda-macos.sh` through micromamba for its own subdir, with the three from-source
    webrtc projects cached by their pinned versions (`doc/macos.md:175-177` names them as the
    obvious prebuilt candidate), `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16` to fit the runner, and
    `meson compile macos-bundle` ad-hoc signed, uploaded as a job artifact.
  * **macos-universal**: downloads both, runs `lipo-bundles.sh`, imports the `.p12` into a
    temporary keychain, signs with the real identity, `notarytool submit --wait` with the API
    key, `stapler staple` the `.app`, `make-tarball.sh`.
  * **android** (`ubuntu-latest`, NDK + `cargo-ndk`): `build-core.sh` for `arm64-v8a`, then
    `gradlew assembleRelease` with the keystore secret.
  * **flatpak** (`flathub/flatpak-github-actions`, `org.gnome.Platform//50`): the `.flatpak`
    bundle as a release asset, and the manifest for the Flathub submission later.
  * **publish**: creates the GitHub release for the tag (pre-release when the tag has `rc`),
    attaches every asset, computes sha256s, writes `<channel>.json`, signs it with the
    `UPDATE_FEED_PRIVATE_KEY` secret, and commits both files to the `updates` branch.
* `nightly.yml` — the same jobs on push to `main` with `-Dprofile=development` and
  `-Profile Devel`, publishing to the rolling `nightly` pre-release and the `nightly` channel.
  `concurrency: nightly` with `cancel-in-progress` so a burst of pushes produces one build, and
  `workflow_dispatch` for a manual run. Both workflows share one reusable `build.yml` that takes
  the profile as an input, so there is one definition of each platform job.

Cost: the Windows job is the long one (dependency tree plus the 11-minute app crate,
`doc/windows.md:236-249`); `Swatinem/rust-cache` on `CARGO_HOME` and the meson build dir cuts
the second run to the app crate alone. The macOS runner's memory fits with the codegen-units
override.

## Where this got to

All six milestones landed on 11 September 2026, in five commits. What exists
is the ledger's job to describe — [`doc/updates.md`](updates.md) — and this
section only says how far the plan got and what it changed on the way:

* **M0-M4 are done and verified** as far as this machine allows. The core has
  27 tests, the Windows helper 3; the Android APK builds, carries the derived
  version and is signed by the new release key; the GTK application compiles
  clean under pedantic clippy on the MSYS2 toolchain.
* **M5 is written and unrun.** Every workflow parses and every script it calls
  passes `bash -n`, but no tag has been pushed. The first release candidate is
  the test.
* **The macOS half has never executed.** `lipo-bundles.sh`, `sign-notarize.sh`
  and `macos_update.rs` were all written without a Mac to hand.

Three of the questions this file left open were settled by building it:

1. `download` streams by hand with `reqwest` rather than reusing the SDK's
   media helper, which is bound to Matrix content repositories.
2. macOS extracts with `/usr/bin/tar` rather than the `tar` crate. It is on
   every Mac, it is what wrote the archive, and a bundle whose signature has
   to survive the round trip is not where to discover that a reimplementation
   handles some corner of the format differently.
3. The About dialog gained nothing. Release notes for the _current_ version
   would need the metainfo parsed at runtime; the feed already carries a notes
   URL for the _next_ one, which is the one worth reading, so it is a link on
   the update row instead.

Two things the plan did not anticipate:

* The version had to reach the core from the embedder. Nothing compiled in can
  know the commit count, so `CoreConfig` grew `build_number` and Meson, Gradle
  and `bundle.sh` now all take it from the same `git rev-list --count HEAD`.
* `commune-core` had a version of its own (`0.1.0`) while the application
  shipped `1.0.0-rc1`. Since the core is what reports the running version to
  the feed, the two are now one number, and `RELEASING.md` says to bump both.

## Milestones

* **M0 — prerequisites.** Release keystore for Android; `commune-core` version aligned with the
  app's; `versionName`/`versionCode` derived, not typed; the Ed25519 feed key pair generated
  and the public half committed; `RELEASING.md` updated for all three. Decisions from the
  interview folded into this file.
* **M1 — core.** `commune-core/src/updates.rs` with tests against a manifest fixture (bad
  signature, older version, same version newer build, wrong platform, oversize). No UI.
* **M2 — GTK check and settings.** Startup/daily check, toast, Updates group, About release
  notes, Flatpak/Devel/ad-hoc gating. Verified against a local feed on Windows and on the Mac.
* **M3 — Windows and macOS install.** `windows_update.rs`, `macos_update.rs`, tarball
  stapling, the relaunch paths. Verified by installing an older build, pointing it at a local
  feed, and watching it become the newer one, on each OS. The macOS half of that verification
  needs the Mac in hand, which it is not at the time of writing; the code lands with the
  Windows half verified and the macOS check owed, recorded in `doc/updates.md`.
* **M4 — Android.** Gradle versioning, manifest permission and provider, About group, Updates
  switch, `PackageInstaller` flow. Verified on the emulator, then the Pixel (which needs the
  one-time reinstall).
* **M5 — CI.** `check.yml`, then the reusable `build.yml` one job at a time, Windows first
  because it is the slowest to get right, then the two macOS jobs and the lipo merge, Android,
  Flatpak; `nightly.yml` goes live as soon as the Windows job works, because that is the loop
  the maintainer wants replaced, and grows a platform as each job lands; `release.yml` last,
  with the first tagged release candidate as the end-to-end test of the feed.
  Prerequisites outside the tree, all doable from this PC: the second Developer ID certificate
  and its `.p12`, an App Store Connect API key, a federated credential for the `skz-code`
  Trusted Signing principal, and the repository secrets.
* **M6 — docs.** `doc/updates.md` ledger; `RELEASING.md` rewritten around the workflow;
  `doc/windows.md`, `doc/macos.md`, `android-kotlin/README.md` sections; the three HTML pages
  do not change (updating is not a client-comparison or spec row).

## Risks and open questions

Risks:

* **A compromised CI secret is a compromised feed.** With the feed key in GitHub, whoever can
  run the release workflow can publish an update. Keeping the key local (sign the manifest on
  the maintainer's machine, upload from there) removes that but makes releases a two-step.
* **The Windows startup registration race** (`doc/startup-registration-race.md`) means the
  relaunch after `msiexec` could produce a second primary process if anything else launches
  Commune in the same seconds; the detached shell waits for `msiexec` to exit, which is well
  past the window.
* **macOS `/Applications` ownership.** A bundle installed by an admin and run by a standard user
  cannot be swapped; the fallback is Finder.
* **MSYS2 drift.** A rolling toolchain in CI can move GTK under a release without a commit
  changing. Pinning MSYS2 is not something the project does today; the probe table artifact is
  the audit trail.
* **`AllowSameVersionUpgrades`** already lets an MSI of the same `1.0.0` replace itself, so
  `rc1 → rc2` (both `1.0.0` to MSI) works, and so does every nightly (all `1.0.0` to MSI);
  downgrades are refused by `MajorUpgrade`, which is what we want.
* **Nightly runner cost.** Every push to `main` builds five platforms; the Windows job alone is
  tens of minutes before caching. Concurrency cancellation bounds it to one build per burst,
  and the free tier's minutes are what pays until the repository is public, when they stop
  counting.
* **The Intel runner's autumn 2027 retirement** is the universal binary's expiry date unless the
  x86_64 half moves to cross-compilation on arm64.

Decided in the 11 September 2026 interview (the questions this section used to hold):
three channels with Devel following nightly; feed key in a GitHub secret; a second Developer
ID certificate issued from this PC; the full Android installer; automatic checks on by default;
the `updates` branch as host; `/passive`; one universal macOS binary. Each is now a numbered
entry under [Decisions](#decisions).

Still open, to be settled when the milestone reaches them:

1. Whether `download` in the core is streamed by hand or the SDK's media download helper is
   reused (M1).
2. The tar reader on macOS: the `tar` crate or `/usr/bin/tar` (M3).
3. Whether the About dialog or the greeter carries _Check for updates_ while logged out (M2).

## Critical files

* `commune-core/src/updates.rs`, `commune-core/src/updates/key.rs`, `commune-core/src/http.rs`
  (streaming fetch), `commune-core/src/settings.rs`, `commune-core/Cargo.toml` (version)
* `src/application.rs` (startup check, timer, toast), `src/utils/windows_update.rs`,
  `src/utils/macos_update.rs`, `src/utils/app_bundle.rs` (install root, Team ID),
  `src/account_settings/general_page/mod.{blp,rs}`, `po/POTFILES.in`
* `build-aux/macos/make-tarball.sh` (staple before tar), a new `build-aux/macos/lipo-bundles.sh`,
  `build-aux/windows/build-msi.ps1`, a CI copy of `artifact-signing-metadata.json`
* `android-kotlin/app/build.gradle.kts` (version, signing), `AndroidManifest.xml`,
  `ui/SettingsScreen.kt`, a new `Updates.kt`
* `.github/workflows/{check,build,release,nightly}.yml`, `RELEASING.md`, `doc/updates.md`

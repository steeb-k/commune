# Automatic updates

How an installed Commune finds out that a newer release exists and replaces
itself with it, and how the builds it installs are made.

[`doc/updates-plan.md`](updates-plan.md) is the plan that produced this and
carries the reasoning behind each decision; this page is the ledger of what
exists. Read this one before touching the updater, and that one before
changing what it does.

## Contents

<!-- toc -->
* [What exists](#what-exists)
* [The feed](#the-feed)
* [The core](#the-core)
* [What each platform does](#what-each-platform-does)
* [When the updater stays quiet](#when-the-updater-stays-quiet)
* [The keys](#the-keys)
* [Continuous integration](#continuous-integration)
* [Testing it without publishing a release](#testing-it-without-publishing-a-release)
* [What has not been seen working](#what-has-not-been-seen-working)
<!-- /toc -->

## What exists

| Piece | Where |
| --- | --- |
| Fetching, verifying, comparing, downloading | `commune-core/src/updates/mod.rs` |
| The public key the feed is checked against | `commune-core/src/updates/key.rs` |
| The `UniFFI` surface for the Kotlin app | the Updates section of `commune-core/src/facade.rs` |
| The GTK object, the toast and the timer | `src/updates.rs` |
| The GTK settings rows | the Updates group of `src/account_settings/general_page/mod.{blp,rs}` |
| Installing, Windows | `src/utils/windows_update.rs` |
| Installing, macOS | `src/utils/macos_update.rs` |
| The Kotlin state and the package-installer hand-off | `android-kotlin/app/src/main/java/io/github/steeb_k/commune/Updates.kt` |
| The Kotlin rows | the About group of `.../ui/SettingsScreen.kt` |
| Writing and signing a manifest | `build-aux/ci/publish-feed.sh` |
| Publishing it | `build-aux/ci/commit-feed.sh` |
| The workflows | `.github/workflows/{check,build,release,nightly}.yml` |

## The feed

One JSON document per channel, at

```text
https://raw.githubusercontent.com/steeb-k/commune/updates/<channel>.json
```

with a detached signature at `<channel>.json.sig` beside it: 64 bytes of
Ed25519, spelled as 128 characters of hex so the branch stays readable text.

`raw.githubusercontent.com` rather than the GitHub API because unauthenticated
API requests are limited to 60 an hour **per source address**, which everyone
behind one NAT shares. An update check is the one request every copy of an
application makes at the same sort of time, so the API is the thing that would
work in testing and fail in an office.

A manifest looks like this, and `commune-core/tests/fixtures/stable.json` is a
real one, signed by the real key:

```json
{
  "channel": "stable",
  "version": "1.0.0",
  "build": 4021,
  "published": "2026-09-11T00:00:00Z",
  "notes_url": "https://github.com/steeb-k/commune/releases/tag/v1",
  "assets": {
    "windows-x86_64-msi": { "url": "…", "sha256": "…", "size": 91234567 }
  }
}
```

`version` is the semver spelling, which is the one of the three in
[`RELEASING.md`](../RELEASING.md) that orders correctly. `build` is
`git rev-list --count HEAD`, and it exists because the version cannot order two
builds of one release — every nightly in a series is `1.0.0-rc1`. The same
number is `CFBundleVersion` on macOS and `versionCode` on Android.

Three channels:

| Channel | Holds | Written by |
| --- | --- | --- |
| `stable` | tagged releases | a tag with no pre-release part |
| `rc` | release candidates, **and** the stable releases that follow them | every tag |
| `nightly` | every build of `main`, Devel profile | a push to `main` |

A stable release is written to both `stable` and `rc` so that somebody testing
candidates is not left behind when the real release arrives. The client does
not know that policy; it reads one file.

## The core

`commune_core::updates` is everything that is the same on every platform.

* `check(channel)` fetches the manifest and the signature, verifies, parses,
  and compares. It returns `Ok` with nothing available when the build is
  current, which is the ordinary answer and not an error.
* `download(asset, dir, progress)` streams to a `.part` file, enforces the
  advertised size, checks the SHA-256 and renames into place. A download that
  fails its digest is removed rather than left where an installer could be
  pointed at it.
* `UpdateSettings` is one JSON value under the `updates` key of whatever
  settings store the embedder provided — `GSettings` on the desktop, a file on
  Android. Automatic checks are **on** by default: these builds have no store
  behind them, and the check is one small signed file a day.

The signature is checked **before any field is parsed**. It is not what stops a
hostile artifact from being installed — Authenticode, Developer ID and the APK
signing key do that, and the platform enforces them — it is what stops a
rewritten feed from choosing _which_ correctly signed build an installation
ends up on.

## What each platform does

**Windows.** Downloads the `.msi`, checks its Authenticode signature with
`WinVerifyTrust`, writes a small `.cmd` beside it and starts it detached, then
quits. The script waits for the process to be gone, runs
`msiexec /i … /passive /norestart`, and starts whatever is at the installed
path — the new build if the upgrade worked, the old one if it did not, because
a failed `MajorUpgrade` rolls back. Nothing in
`build-aux/windows/commune.wxs` had to change: the package is already
`perUser`, so there is no elevation, and its `UpgradeCode` has not moved since
the first release.

Revocation checking is deliberately off. It is a network round trip whose
failure mode is the wrong one: behind a captive portal it calls a good package
untrusted and the updater stops working. Windows asks again for itself when
the installer runs.

**macOS.** Downloads the `.tar.gz` — not the `.dmg`, because a bundle that
arrives inside a disk image is quarantined and refused on first launch, and
files a process extracts from a tarball never are. Extracts beside the
installed bundle, runs `codesign --verify --deep --strict`, requires the Team
ID to equal the running bundle's, and swaps the two directories by rename,
putting the old one back if the second rename fails. Nothing is written
_into_ the running bundle, so every failure short of the swap leaves the
installed copy exactly as it was. Afterwards it `touch`es the bundle and runs
`lsregister -f`, which is what makes the Dock and Finder notice — see
[`doc/macos.md`](macos.md) on the icon cache.

**Android.** Downloads the APK and hands it to the system package installer
through a `FileProvider` that exposes exactly one cache directory. Installing
is deliberately not something the app does: Android replaces an installed
application only with one signed by the same key, and it is the system that
enforces that. `REQUEST_INSTALL_PACKAGES` is declared, but the user still has
to allow it once; the first press opens the settings page that can and says
so.

**Linux.** Nothing. A Flatpak has an app store behind it, and the Updates group
says so rather than hiding the question.

## When the updater stays quiet

An update that cannot be installed must not be announced, so the check is not
made at all unless this build could replace itself. That is a narrower
question than which operating system it is:

* Inside Flatpak — `/.flatpak-info` exists. The first Flatpak detection in the
  tree; nothing else needed to know.
* On Windows, unless the executable is under `%LOCALAPPDATA%\Programs`.
  Anywhere else is a build directory, an unpacked `.zip` or a `meson install`
  into an MSYS2 prefix, and running an installer would leave the user with two
  copies rather than a newer one.
* On macOS, unless the bundle carries a real Team ID. An ad-hoc signature means
  a developer's own build, and replacing it with a release would change the
  identity under every Keychain item it has stored and re-prompt for each.

## The keys

Three, and none of them is in this repository.

**The feed key** signs the manifest. Its public half is
`commune-core/src/updates/key.rs`, compiled into every build; the private half
is the `UPDATE_FEED_PRIVATE_KEY` repository secret and the maintainer's offline
backup. `VERIFYING_KEYS` is a list because a rotation has to go in two steps —
add the new key, release, wait for the old builds to go, then remove the old
one. Replacing the constant in one commit strands every installation that has
not already updated.

**The Windows identity** is Azure Trusted Signing, account `skz-code`. A laptop
authorises a signature with an `az login` session; CI authorises it with an
OIDC token, which is the whole difference between
`artifact-signing-metadata.json` and `artifact-signing-metadata.ci.json`.

The identity is the `commune-ci-signing` app registration, created
11 September 2026, which holds **no client secret at all**: it trusts GitHub's
OIDC issuer for the subject `repo:steeb-k/commune:environment:release`, and
that is why the Windows job declares `environment: release`. A federated
credential's subject cannot contain a wildcard and release tags vary in name,
so one environment stands in for every tag and for `main` alike. The
environment requires no approval; adding a reviewer to it is what would make a
release wait for one.

Its one permission is **Artifact Signing Certificate Profile Signer**, scoped
to the `ddrx-pcsvc` profile rather than the account — the role was renamed from
Trusted Signing in Azure's rebrand, which is why searching for the old name
finds nothing.

Without `AZURE_CLIENT_ID` the Windows job still builds and still publishes —
it just publishes a package the updater refuses, because `windows_update.rs`
checks Authenticode before handing anything to `msiexec`. The run says so as a
warning and in the job summary. That is the right failure: an installer nobody
can auto-install beats quietly loosening what "signed" means.

**The Android key** is `android-kotlin/commune-release.jks`, named by a
git-ignored `keystore.properties` beside it, and base64 in the
`ANDROID_KEYSTORE_BASE64` secret. It is the one file in this project that
cannot be regenerated: an APK signed by anything else cannot upgrade an
installed one, so losing it means every installation has to be uninstalled by
hand. Back it up offline. A checkout without it still builds — the release
variant falls back to the debug key — and produces something installable that
simply cannot upgrade a released copy.

**The Mac identity** is a second Developer ID Application certificate under
team `VLC2KZKNBH`, issued 11 September 2026 from a CSR generated off the Mac
because the certificate already on it is Xcode's cloud-managed kind and cannot
be exported. The team is what an installed copy compares, not the certificate,
so a second one under the same team updates silently. It expires in September
2031.

Notarizing needs a credential of its own, and `sign-notarize.sh` takes either:
an App Store Connect team API key, or an app-specific password from
appleid.apple.com. **Neither has anything to do with the App Store.**
Notarization is the other half of Developer ID — the arrangement for software
distributed outside the store — and involves no listing, no review and no app
record. The name of the first credential is the only thing suggesting
otherwise. Commune could not be listed in any case: it is GPL-3 with many
copyright holders, which the store's terms do not permit.

Notarizing is also skippable, with `ALLOW_UNNOTARIZED=1`, and the consequence
is narrower than it looks. The updater extracts the tarball itself, so what it
installs is never quarantined and Gatekeeper never asks. What breaks is the
_first_ install by somebody who downloads the tarball in a browser and unpacks
it in Finder: that copy is refused on launch. Shipping un-notarized is
therefore a decision about new users, not about updates.

## Continuous integration

`check.yml` on every push and pull request: the same `hooks/checks` binary the
pre-commit hook runs, plus `hooks/template-checks` and the tests.

`build.yml` is called by the other two and builds everything: the Android APK,
the Flatpak bundle, the Windows bundle and MSI in MSYS2, and two macOS bundles
merged with `lipo` and then signed and notarized. It takes the profile as an
input, which is the only thing that differs between a release and a nightly.

**They run in that order, one after another, rather than at once.** Android
first because it is the client most people run, macOS last because it is the
one most likely to fail — so a failure stops the chain before the expensive
half of it, instead of four platforms failing in parallel for one reason. Each
job runs when the one before it succeeded _or was skipped_, which is what keeps
the platform picker working; a job whose predecessor actually failed does not
run at all.

**Every build embeds the KLIPY key**, from the `KLIPY_API_KEY` secret: a Meson
option on the desktop platforms, a `local.properties` line on Android, and an
injected `config-opts` entry for Flatpak. The tracked manifest stays clean, so
the key is in the repository nowhere — but it _is_ in the binaries, and a
client-side key compiled into a distributed binary comes back out with
`strings`. Shipping it publicly is therefore publishing it, which was the
decision taken on 11 September 2026 with that understood. If the quota is
burned, the GIF search fails with a toast and nothing else breaks.

The one asymmetry: a Flathub build uses the tracked manifest, which has no key,
so a Flatpak installed from Flathub has no GIF search.

`release.yml` on a `v*` tag. `nightly.yml` on a press, with a platform picker,
plus one scheduled build a week — deliberately **not** on every push, because
five platforms take the better part of two hours and most commits touch code
three of them do not compile. Free minutes on a public repository make that
affordable rather than useful.

The press matters more than it looks: there is no Mac, so CI is the only way a
macOS build gets made at all. Tying that to release tags alone would mean
minting a permanent tag and bumping three version files every time somebody
wanted to see a change on macOS, which is a worse loop than the one this
replaced. A build with platforms deselected does not publish, because a
manifest written from a partial build tells every other platform there is
nothing to install.

Both end by writing the feed. The nightly release is a rolling tag, which is the one
exception to [`RELEASING.md`](../RELEASING.md)'s rule that a tag is never
moved, and it says so there.

MSYS2 is a rolling release with no pinning mechanism, which is a real risk for
a release build: GTK can move underneath the port without a commit changing.
The Windows job uploads `probe-env.txt` as an artifact, and that is the audit
trail for what a given build actually used.

## Testing it without publishing a release

`COMMUNE_UPDATE_FEED` points the check somewhere else — any HTTP server with a
manifest and a signature on it:

```sh
export UPDATE_FEED_PRIVATE_KEY="$(cat feed-private.pem)"
sh build-aux/ci/publish-feed.sh \
    --channel stable --version 9.0.0 --build 99999 \
    --notes-url https://example.invalid/notes \
    --out-dir /tmp/feed \
    --asset 'windows-x86_64-msi|/tmp/Commune.msi|http://localhost:8000/Commune.msi'

(cd /tmp/feed && python3 -m http.server 8000) &
COMMUNE_UPDATE_FEED=http://localhost:8000 ./commune
```

The manifest has to be signed by a key the build trusts, so testing the check
against a key of your own means rebuilding with it in `key.rs`.

## What has not been seen working

Kept honest rather than hopeful:

* **Every workflow is unrun.** They parse, and every script they call passes
  `bash -n`, but no tag has been pushed. The first release candidate is their
  test, and the Windows job is the one most likely to need a second go.
* **The macOS install path has never run.** It was written without a Mac to
  hand. `lipo-bundles.sh` and `sign-notarize.sh` have never been executed at
  all, and `macos_update.rs` has only been compiled.
* **The Android APK has not been installed on the Pixel.** It builds and is
  signed with the release key (SHA-256 `4E:C5:A0:98:…`), which means the first
  install has to uninstall the debug-keyed copy and loses that device's adopted
  session.
* **The `updates` branch does not exist yet**, so every check 404s, which the
  app reads as "no update" and says nothing about.
* **The notary credential has never been exercised.** It is an app-specific
  password, set 11 September 2026, and `notarytool` exists only on macOS — so
  nothing on the development machine can test it. The first release build is
  what proves it, which is an argument for making the first tag a candidate.
* The Windows install path is covered by unit tests for the script it writes,
  but no MSI has actually been installed over another one by the updater.

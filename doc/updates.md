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
* [Watching a run](#watching-a-run)
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
| Creating the GitHub release the feed points at | `build-aux/ci/publish-release.sh` |
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
the first release. One thing did have to change there, on
13 September 2026: the package sets `REINSTALLMODE` to `amus`, because the
installer's default rule skips a file whose installed copy has a higher
version, costs that decision while the previous product is still there, and
then removes the previous product's copy. A package built where MSYS2 was
older than the runner's left sixteen DLLs missing with `msiexec` reporting
success, and the application could not start. Every file is installed now,
whatever is already there.

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

**Linux.** Nothing in the application. A Flatpak updates through whatever
remote it was installed from, and the Updates group says so rather than hiding
the question.

Which remote that will be is **undecided**, and one option is closed:
**Flathub is ruled out, because it rejects LLM-generated code.** The plan is a
self-hosted OSTree repository on the maintainer's own server behind a reverse
proxy — `RELEASING.md` already sketches the shape, and OSTree being
content-addressed means each publish moves only the objects that changed. How
CI reaches that server, and whether the repository is public, are open
questions as of 12 September 2026.

Until then CI publishes a single `.flatpak` bundle, and a bundle has **no
update path at all**: it carries no remote, so `flatpak update` has nothing to
check. Fine for handing somebody a build, useless as a channel, and worth
saying in the release notes of anything that ships that way.

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

It carries **two** federated credentials for what is logically one subject.
GitHub now presents the immutable-ID form —
`repo:steeb-k@133536278/commune@1340279514:environment:release` — rather than
the readable one, and Azure matches the subject as an exact string, so the
readable credential alone fails with `AADSTS700213: no matching federated
identity record`. Both are registered: the ID form because it is what arrives,
the readable one because it is what the documentation everywhere describes and
what would arrive again if that behaviour were turned off.

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

It reaches a runner as `MACOS_CERTIFICATE_P12`, base64 of a `.p12` holding the
private key, the certificate and Apple's intermediate — the intermediate
because a runner has no keychain that already contains it, and `codesign`
cannot build a chain without one. That is the usual reason a certificate that
works on a Mac fails in CI.

**The `.p12` has to be written with SHA-1 and 3DES**, which `make-p12.sh` now
passes explicitly. OpenSSL 3 defaults to AES-256-CBC with a SHA-256 MAC, and
Apple's Security framework cannot read that at all — `security import` rejects
it with

```text
MAC verification failed during PKCS12 import (wrong password?)
```

with the password perfectly correct. That is as misleading as an error message
gets, and it is what stopped the first macOS build that ever reached the
signing step. The weak algorithms are not a weakness here: the file exists for
the few seconds between a runner decoding it and importing it into a throwaway
keychain, and what protects it is a 28-character random password in another
secret.

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

Both end by writing the feed. The release itself is created by
`build-aux/ci/publish-release.sh`, not a bare `gh release create`: `gh`
creates a release with files as a draft, uploads to it, then un-drafts it as
three separate calls, and a 5xx from the first of those left a v1.rc3 release
stuck as a draft while the step that created it still reported success. The
script retries a failing call instead of falling back to one that cannot tell
a draft from a published release, always un-drafts afterward in case an
earlier attempt was interrupted partway, and refuses to return success until
`gh release view` shows the release published with every asset uploaded and
downloadable — so the feed step never writes a manifest pointing at a release
nobody can see. The nightly release is a rolling tag, which is the one
exception to [`RELEASING.md`](../RELEASING.md)'s rule that a tag is never
moved, and it says so there.

MSYS2 is a rolling release with no pinning mechanism, which is a real risk for
a release build: GTK can move underneath the port without a commit changing.
The Windows job uploads `probe-env.txt` as an artifact, and that is the audit
trail for what a given build actually used.

**The Windows job checks it can sign before it builds anything.** Everything
before the bundle takes about two minutes; the bundle itself takes forty. A
credential that does not work should therefore say so at minute two, and the
first version of this job said so at minute forty-three — having compiled the
whole application to find out. The signing steps now come first and end with a
real signature, because nothing short of that exercises the endpoint, the
federated identity, the signing library and `signtool` together.

Getting that check to mean anything took three tries, and the two wrong ones
are worth keeping. The first signed a copy of a system binary and then asked
whether it was signed: it was, by Microsoft, before the job ever started, so
the check passed without the credential working at all. The second tried to
strip that signature first — which cannot be done, because system binaries are
signed through a catalog and there is no embedded signature in the file to
remove, so `signtool` answered `0x57` and the guard refused to continue. The
third compiles a one-line C# program with the .NET Framework compiler, which
sits at a fixed path on every Windows image. A binary that has existed for two
seconds is one nobody has signed, which is what makes the check sound: the step
asserts the probe is unsigned first and signed afterwards, so the signature at
the end can only have come from the step in the middle.

## Watching a run

`build-aux/ci/watch-run.sh` blocks until a run finishes and then prints the
failing step and the tail of its log. It exists because of a failure of
process rather than of code: runs were being started and then left, so the
person who found out the build was broken was the one who had asked for it.

```sh
gh workflow run nightly.yml -f platforms=windows -f publish=false
bash build-aux/ci/watch-run.sh
```

The rule it encodes: **a run nobody is waiting on is a run whose result
arrives by complaint.** Do not call a build done until something has printed a
conclusion.

## Testing it without publishing a release

`COMMUNE_UPDATE_FEED` points the check somewhere else — any HTTP server with a
manifest and a signature on it:

```sh
export UPDATE_FEED_PRIVATE_KEY="$(cat feed-private.pem)"
bash build-aux/ci/publish-feed.sh \
    --channel stable --version 9.0.0 --build 99999 \
    --notes-url https://example.invalid/notes \
    --out-dir /tmp/feed \
    --asset 'windows-x86_64-msi|/tmp/Commune.msi|http://localhost:8000/Commune.msi'

(cd /tmp/feed && python3 -m http.server 8000) &
COMMUNE_UPDATE_FEED=http://localhost:8000 ./commune
```

The manifest has to be signed by a key the build trusts, so testing the check
against a key of your own means rebuilding with it in `key.rs`.

A release APK has no way to receive an env var, and setting one would not
reliably reach the core in any case: an `arm64`-only release build run under
ARM translation on an x86_64 emulator has its `Os.setenv` write the guest
libc's environment, which is not necessarily the one `reqwest` reads from on
the other side of that translation. `CommuneApplication.onCreate()` instead
looks for a file at `getExternalFilesDir(null)/update-feed` — on an emulator
or a device that is `/sdcard/Android/data/io.github.steeb_k.commune/files/`,
writable by `adb push` — and if it is there, reads its first line and calls
`setUpdateFeed()`, an in-process override `commune_core::updates::feed_base()`
consults before the environment variable, before anything touches the core.
The emulator reaches the host's `python3 -m http.server` at
`http://10.0.2.2:8000`. Nobody has this file in production. Because the
updater orders two builds of one version by `versionCode`, a locally-served
manifest also needs a test APK the running one reads as newer:
`./gradlew assembleRelease -PcommuneVersionCode=<n>` (or `assembleDebug` with
the same property) overrides the commit count that `versionCode` would
otherwise use.

The debug variant — `x86_64`, what an emulator runs natively — is what an
emulator test of the updater itself actually wants; a release build is
`arm64`-only and only runs on one at all through ARM translation. Either way,
`build-core.sh` has to have produced a fresh `.so` for the ABI being tested:
`app/src/main/jniLibs` is one directory shared by every Gradle variant, so an
ABI a previous, narrower invocation (`--arm64`, say) did not rebuild is
whatever was last written there, and the script now warns rather than
silently packaging it. Plain `./build-core.sh --release` builds both ABIs
already; reach for `--arm64` or `--x86_64` only when the other one
genuinely does not matter for what is being tested.

## What has not been seen working

Kept honest rather than hopeful. Ticked off as each one actually runs.

**Run, on 12 September 2026.** The gate, the Android APK, the Flatpak bundle,
the signed Windows MSI and the signed, notarized, stapled macOS universal
tarball all build on runners. The Windows installer is signed by Trusted
Signing, proven by a probe that signs a binary compiled seconds earlier. The
Mac bundle is `x86_64 arm64` in one file, and the notary service answered
`Accepted` in eighty seconds.

Getting macOS there took eight attempts, and every one found something real:
the Intel runner cannot build the arm64 conda environment, neither half had a
SASS compiler, codesign was being handed the bundle's main executable before
the dylibs it encloses, the two halves legitimately differ by one library, the
discarded ad-hoc signature looked like a divergence, and the `.p12` was
written in a format Apple cannot read. One of the eight was a bad edit of mine
that shipped half a fix.

**Released, on 12 and 13 September 2026.** `v1.rc1`, `v1.rc2` and `v1.rc3`
went out through `release.yml`; the `updates` branch exists and carries the
`rc` feed. The third of those sat as a draft for six hours after a green run,
which is what `publish-release.sh` now exists to refuse.

**Seen updating itself, on 13 September 2026.** Three of the four paths, each
on a real installation, driven through the application's own rows:

* **Windows.** An installed candidate checked the `rc` feed, downloaded the
  signed MSI, verified it, quit, and the helper ran `msiexec` and started the
  new build. The first attempt at this found the helper never reached
  `msiexec` at all — started without a console, its `tasklist | find` wait
  loop hung on the first iteration, twice out of twice — which is why the
  first three candidates cannot update themselves on Windows: install 1.rc4
  from the release page once, and it updates itself from then on.
* **Android.** The check, the download with its percentage, the package
  installer sheet and a new `versionCode` running with the session intact,
  on the emulator against a locally signed feed, for the debug variant natively
  and for the arm64 release variant under ARM translation. The first attempt
  found the check itself dead on arrival — a UniFFI future polled with no
  runtime under it — so the first three candidates' Android builds cannot
  check at all; install 1.rc4 by hand once.
* **The feed's answers.** Up to date, available, skipped, an unreachable
  server, and a channel with no manifest on it, each with its sentence, on both
  platforms.

Still unrun or unseen:

* **macOS.** `macos_update.rs` has only ever been compiled. No Mac has run the
  updater; the first person to press Update on one is the test.
* **The Android APK has not been installed on the Pixel.** It is signed with
  the release key (SHA-256 `4E:C5:A0:98:…`), so the first install has to
  uninstall the debug-keyed copy and loses that device's adopted session.
* **A stable release.** No tag without a pre-release part has been pushed, so
  the `stable` manifest has never been written; a candidate that follows
  `stable` is told so in words rather than left thinking the server is down.

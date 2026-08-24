# Building and running on Windows

The ledger for the Windows port. `doc/windows-plan.md` is the route that was planned before any of
it was built; this file records what actually exists, what is stubbed, and what bit us on the way.

## Contents

<!-- toc -->
* [State today](#state-today)
* [The GTK environment](#the-gtk-environment)
* [Setting the environment up](#setting-the-environment-up)
* [Building](#building)
* [Packaging](#packaging)
* [What bit us](#what-bit-us)
* [What differs from Linux](#what-differs-from-linux)
* [Not done yet](#not-done-yet)
* [Rebasing](#rebasing)
<!-- /toc -->

## State today

**M0, M2, M3 and M5 are done, and M1 is most of the way there.** The tree builds for
`x86_64-pc-windows-gnu`, and
`cargo check`, `cargo clippy --all-targets -- -D warnings`, nightly `cargo fmt --check`,
`cargo deny`, `cargo machete`, `cargo sort`, `typos`, `rumdl` and the pre-commit hook all pass.
`meson setup`, `ninja` and `meson install` work, and **the app runs**: it opens its window, renders
the welcome screen with the correct icons and dark mode, and quits cleanly.

There is a **relocatable folder** and a **per-user `.msi`** around it. The folder starts with
`PATH` cut to `C:\Windows` and loads 158 modules, every one of them its own and none from MSYS2.
The installer was installed, exercised and uninstalled: it registers the Start Menu shortcut with
its AUMID, registers `matrix:`, and removes all three cleanly.

Two of the plan's open questions have been answered by experiment, both favourably:

* **`GApplication` uniqueness works** without a session bus. A second invocation exits on its own
  rather than becoming a second primary instance.
* **`matrix:` URIs forward to the running instance.** A second invocation carrying
  `matrix:u/alice:example.org` reached the first instance's `Application::open`, was parsed into
  `ShowMatrixId(User("@alice:example.org"))`, and was refused only because no session was logged
  in. So the warm path of M3 already works, and all that scheme handling still needs is the
  registry key an installer writes.

The **Credential Manager backend is exercised** by
`secret::windows::tests::a_session_survives_a_round_trip`, which stores a session, reads every
field back, and deletes it. That test writes to the real credential store, because the API has no
notion of another one; it is worth the intrusion because this is the only place in the application
that hands raw pointers to the operating system, and everything it protects is lost if it is wrong.
`cargo nextest run` is 156 tests, all passing, and `meson test` passes.

**Logging in works**, against a real homeserver, and syncing and the timeline with it. So does
**session restore** — quit and relaunch and the session comes back out of the Credential Manager —
which completes that path end to end, since restoring is the one part the round-trip test could
not cover.

Three things came out of that first session with an account, and none of them is what it looked
like at first glance. They are written up in [What bit us](#what-bit-us): calls fail over Remote
Desktop for want of a microphone and not for want of a port; the message search index cannot live
on disk here, because a room ID is not a legal Windows file name; and GLib turns out to have a
Windows notification backend after all, which changes what M5 is.

| Area | State |
| --- | --- |
| Toolchain | MSYS2 UCRT64, mingw ABI, `x86_64-pc-windows-gnu` |
| Runtime paths | Meson's compile-time constants — a bundle relocates itself, see below |
| Image decoding | `image` crate, shared with macOS via `cfg(not(target_os = "linux"))` |
| Video and audio playback | Own `GtkMediaStream`, `src/components/media/gst_media_stream.rs` |
| Secrets | Windows Credential Manager, `src/secret/windows.rs`, round-tripped by a test |
| Message search | In-memory index — it cannot be stored on disk here, see below |
| Data directories | `%LOCALAPPDATA%\commune[-Devel]\{data,cache}` |
| Console window | Suppressed in release builds only, `src/main.rs` |
| Location sharing | Stubbed, `is_available()` is false and the UI hides it |
| System 12/24h clock | Read from `sShortTime` at startup, `src/system_settings/windows.rs` |
| Camera QR scanning | Stubbed, returns no cameras |
| Relocatable folder, `.zip` | `build-aux/windows/bundle.sh` |
| Installer | Per-user WiX 5 MSI, `build-aux/windows/{commune.wxs,build-msi.ps1}` |
| Signing | `build-aux/windows/sign.ps1`, Azure Trusted Signing; skipped without metadata |
| `matrix:` URLs | Works cold and warm; the installer writes the registry key |
| Notifications | Our own WinRT toasts, `src/utils/windows_notifications.rs`; clicks work |

## The GTK environment

Everything comes from **MSYS2's UCRT64 repository**, and the whole port is built from a UCRT64
shell. This was chosen over the gvsbuild prefix at `C:\gtk` that other projects on this machine
use, because gvsbuild carries GTK and gtksourceview but no GStreamer, no libshumate, no libsoup3
and no glib-networking — the heaviest parts of Commune's stack would all have been hand-built —
while UCRT64 packages every one of them, including the `gtk4paintablesink` that had to be compiled
from source for macOS.

The consequence is the **mingw ABI**: MSYS2's GTK cannot be linked by an MSVC-targeting Rust, so
the build uses MSYS2's own Rust and the `x86_64-pc-windows-gnu` target. The rustup toolchain on the
Windows side is MSVC and is used for exactly two things, neither of which links anything of ours:
nightly `rustfmt`, and installing `rumdl` (see [What bit us](#what-bit-us)).

The version set this was built against, from `build-aux/windows/probe-env.sh`:

| | |
| --- | --- |
| glib / gio | 2.88.2 |
| gtk4 | 4.22.4 |
| libadwaita | 1.9.1 |
| gstreamer | 1.28.5, plugins 1.28.6 |
| gtksourceview-5 | 5.21.0 |
| libshumate | 1.5.1, **with** the vector renderer |
| libsoup | 3.6.6 |
| gdk-pixbuf / librsvg | 2.44.7 / 2.62.3 |
| sqlite3 | 3.53.4 |
| rustc / cargo | 1.97.0 (MSYS2 build) |
| meson / ninja | 1.12.0 / 1.13.2 |
| blueprint-compiler | 0.18.0 |

MSYS2 is a rolling release, so `pacman -Syu` can move GTK underneath the port. Update deliberately,
and re-run the probe and record the table above when packaging.

Two things the probe looks for and did **not** find, both expected:

* **No GTK GStreamer media backend.** There is no `lib/gtk-4.0` directory at all, so `GtkMediaFile`
  has no backend — the same hole macOS has, filled by the same `GstMediaStream`.
* **No `Adwaita Sans` or `Adwaita Mono`.** libadwaita asks for them and Pango falls back with
  `couldn't load font … expect ugly output` on stderr. Cosmetic today; a bundle should carry them.

## Setting the environment up

From an **MSYS2 UCRT64** shell:

```sh
pacman -S --needed \
  mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-libadwaita \
  mingw-w64-ucrt-x86_64-gtksourceview5 mingw-w64-ucrt-x86_64-libshumate \
  mingw-w64-ucrt-x86_64-gstreamer mingw-w64-ucrt-x86_64-gst-plugins-base \
  mingw-w64-ucrt-x86_64-gst-plugins-good mingw-w64-ucrt-x86_64-gst-plugins-bad \
  mingw-w64-ucrt-x86_64-gst-plugins-ugly mingw-w64-ucrt-x86_64-gst-plugins-rs \
  mingw-w64-ucrt-x86_64-libnice mingw-w64-ucrt-x86_64-glib-networking \
  mingw-w64-ucrt-x86_64-libsoup3 mingw-w64-ucrt-x86_64-libwebp \
  mingw-w64-ucrt-x86_64-sqlite3 mingw-w64-ucrt-x86_64-librsvg \
  mingw-w64-ucrt-x86_64-adwaita-icon-theme \
  mingw-w64-ucrt-x86_64-blueprint-compiler mingw-w64-ucrt-x86_64-meson \
  mingw-w64-ucrt-x86_64-ninja mingw-w64-ucrt-x86_64-cmake \
  mingw-w64-ucrt-x86_64-make mingw-w64-ucrt-x86_64-gettext-tools \
  mingw-w64-ucrt-x86_64-pkgconf mingw-w64-ucrt-x86_64-rust \
  mingw-w64-ucrt-x86_64-ntldd mingw-w64-ucrt-x86_64-icoutils \
  git
```

`git` is not optional and is easy to miss: `meson setup` stamps a development build with the commit
it was built from and fails outright without it, and Windows' own git is not on the MSYS2 `PATH`.

Then the cargo-installed tooling. **`cargo install` puts binaries in `%USERPROFILE%\.cargo\bin`,
which is not the MSYS2 home directory and is not on the MSYS2 `PATH`**, so every one of these looks
missing until it is added — after `/ucrt64/bin`, never before, because that directory also holds
rustup shims for the MSVC cargo which must not win:

```sh
export PATH="$PATH:$(cygpath -u "$USERPROFILE")/.cargo/bin"

cargo install --locked cargo-deny cargo-machete cargo-sort typos-cli grass cargo-nextest
rustup toolchain install nightly --profile minimal --component rustfmt
```

`rumdl` is the exception and must be installed from a **PowerShell** prompt, with the MSVC
toolchain, for the reason in [What bit us](#what-bit-us):

```powershell
cargo install --locked rumdl
```

Finally, long paths. `LongPathsEnabled` under
`HKLM\SYSTEM\CurrentControlSet\Control\FileSystem` should be `1` (it already was on this machine),
and git needs its own opt-in:

```sh
git config --global core.longpaths true
```

`sh build-aux/windows/probe-env.sh` reports on all of the above and ends with a list of what is
still missing. Run it before asking why a build fails.

## Building

From a UCRT64 shell, with `PATH` extended as above:

```sh
meson setup _build -Dprofile=development --prefix=$MINGW_PREFIX
ninja -C _build
meson install -C _build
commune.exe
```

Installing into the UCRT64 prefix is ordinary MSYS2 practice and puts the app's GSettings schema
and its icons where GLib and GTK already look, so a development run needs no environment variables
at all. The app aborts at startup without its schema, so the install is not optional.

For bare cargo, outside meson:

```sh
export CARGO_TARGET_DIR=$PWD/_build/cargo-target
export GETTEXT_SYSTEM=1 GETTEXT_DIR=$MINGW_PREFIX
cargo clippy --all-targets -- -D warnings
```

`src/config.rs` is generated by `meson setup`, so a bare `cargo` command needs meson to have run at
least once.

To format, which needs the nightly rustfmt rather than the stable one MSYS2 ships — and note that
`cargo +nightly` does **not** work, because the cargo on `PATH` is MSYS2's and has no rustup shim
to interpret the `+`:

```sh
RUSTFMT="$(rustup which --toolchain nightly rustfmt)" cargo fmt --all
```

## Packaging

```sh
meson compile -C _build windows-bundle    # the folder
meson compile -C _build windows-zip       # ... and a .zip beside it
```

and then, from **PowerShell** rather than the MSYS2 shell, because WiX and signtool are Windows
tools:

```powershell
pwsh -File build-aux\windows\build-msi.ps1 -BundleDir "_build\windows\Commune Devel" -Profile Devel
```

The result is `_build\windows\Commune-Devel-<version>-x64.msi`, about 97 MB.

### The folder relocates itself

There is nothing here corresponding to the environment variables `src/utils/app_bundle.rs` has to
set on macOS, and that is not an oversight. GLib on Windows works out where it was installed by
asking the loader where `libglib-2.0-0.dll` came from and taking the parent of its directory;
GdkPixbuf, GIO and GStreamer all follow the same convention. So a tree of

```text
Commune/bin/*.dll   Commune/lib/...   Commune/share/...
```

finds its own data wherever it is moved to, with nothing set and nothing rewritten.

What is genuinely ours to do is the **DLL closure**, because Windows has no rpath and resolves an
import by name in the loading module's own directory, and the **gdk-pixbuf loader cache**, which
records absolute paths and would otherwise point back into MSYS2.

The closure is a worklist rather than repeated passes: each binary is walked exactly once, when it
first arrives. The obvious implementation — sweep everything, repeat until nothing new appears —
adds one layer of the dependency graph per sweep and re-reads every file each time, which took
the better part of an hour where this takes half a minute.

### The audit is the point

The script ends by asking of every import of every binary whether it resolves inside the bundle.
This is not a formality: a bundle that is missing a DLL works perfectly on the machine that built
it, because MSYS2 is on that machine's `PATH`, and fails on every other.

The one subtlety is what "not found" means. Much of it is Windows' own API sets — the
`api-ms-win-*` and `ext-ms-win-*` names — which are virtual: the loader resolves them through a
schema and there is no file, so they always report as missing. Rather than keep a list of names to
forgive, the audit asks the question that actually matters: **is this something the prefix could
have given us?** If the name is in `$MINGW_PREFIX/bin`, we failed to carry it. If it is not, it is
Windows' to provide and never was ours.

### The executable is stripped

`x86_64-pc-windows-gnu` emits DWARF into the PE, and the release profile asks for debug
information deliberately, so the binary arrives at **1.2 GB** — of which the program is about
forty megabytes. `bundle.sh` runs `strip --strip-debug` on the copy that ships, which keeps the
symbol table, so a panic still names the functions in its backtrace; what is lost is the file and
line beside each frame. The unstripped binary stays at `_build/cargo-target/*/commune.exe`, which
is what to reach for when a backtrace has to be read properly, and `--no-strip` keeps the bundled
copy whole as well.

### The MSI is per-user

It installs to `%LOCALAPPDATA%\Programs\Commune[ Devel]` and needs no administrator, so there is
no UAC prompt to install and none to update. For a chat client that is the right way round: it is
one person's application, not the machine's.

It registers the two things a folder on its own cannot:

* **The Start Menu shortcut, carrying `System.AppUserModel.ID`.** Windows identifies an unpackaged
  application to the notification system by an AUMID and will only accept one that a Start Menu
  shortcut declares, so the toasts of M5 depend on this shortcut existing. Setting it now means
  that milestone changes no installer.
* **The `matrix:` scheme**, under `HKCU\Software\Classes` because a per-user install may not write
  machine-wide keys.

Verified by installing it: 1100 files, the shortcut with
`System.AppUserModel.ID = io.github.steeb_k.Commune.Devel`, the scheme registered, and a
`matrix:` link opening the app from its installed location — cold, and warm into the instance
already running. Uninstall removes the files, the shortcut and the registry key.

**User data is deliberately left behind** on uninstall: `%LOCALAPPDATA%\commune[-Devel]` holds the
account databases, and removing them would mean an uninstall-reinstall silently logs the user out
of everything. (The uninstall above did not have any to leave — nothing had logged in.)

### Signing

`sign.ps1` wraps signtool with Azure Trusted Signing, and `build-msi.ps1` calls it twice: on the
executable **before** the MSI is built, since the MSI embeds a copy and a signature applied
afterwards would not reach it, and on the MSI itself.

With no signing metadata — `artifact-signing-metadata.json`, or `$env:ARTIFACT_SIGNING_METADATA` —
signing is **skipped and the build still succeeds**. That is deliberate: anyone should be able to
build Commune for Windows, and only whoever holds the certificate can sign it. The artifacts above
were built that way and are unsigned.

This matters more here than the equivalent did on macOS. There, an ad-hoc signature was enough to
run and a real identity was out of reach, so the port shipped a tarball to route around Gatekeeper.
Here there is a certificate, so the `.msi` can be something a stranger is willing to run: unsigned,
SmartScreen shows an unknown-publisher warning that most people are right to obey.

## What bit us

Six things, none of them ours, all of them ours to work around. They are recorded here because each
one presents as something other than its cause.

**blueprint-compiler reads `.blp` files in the locale encoding.** It calls `open()` without naming
one, so Python uses cp1252 on Windows, and every blueprint containing a typographic quote fails to
decode. It is reported as `***** COMPILER BUG *****` with a `UnicodeDecodeError`, several screens
away from the cause. `build-aux/compile-blueprints.sh` now exports `PYTHONUTF8=1`, which is a no-op
where the locale was already UTF-8.

**Meson does not run a `custom_target` command through a shell.** The `cargo build && cp …` that
built the binary passed `&&` to cargo as an argument, which cargo rejects. This is tolerated
elsewhere but was never right; both steps now live in `build-aux/cargo-build.sh`, alongside the
blueprint script that exists for a similar reason.

**The 260-character path limit, through cargo's libgit2.** `matrix-rust-sdk` has a test fixture
whose file name alone is 96 characters, and rooting the cargo registry at
`_build/cargo-home` — three directories deeper than the default — pushes the checkout past the
limit. `core.longpaths` does not help, and neither does `net.git-fetch-with-cli`, which changes how
cargo _fetches_ but not how it checks out. So `meson.build` does not set `CARGO_HOME` on Windows and
the user's own is used, which costs the build its self-containment on this platform only.

**`rumdl` cannot be built with the mingw toolchain.** It depends on `tikv-jemalloc-sys`, whose
build shells out to `mingw32-make` and then fails to produce a library the linker can find. Under
MSVC, jemalloc is gated out of the build entirely and it compiles in eighty seconds. rumdl is a
standalone markdown linter that links nothing of ours, so which ABI built it does not matter.

**`cargo sort` needs the project's `--order`.** Run bare, the version in use wants to move every
`[target.…]` table to the top of `Cargo.toml` and every `[profile.…]` to the bottom, a 400-line diff
that contradicts the file as committed. The pre-commit hook passes the right order; a manual run
must too:

```sh
cargo sort --check --grouped --order \
  workspace,package,lib,profile,features,dependencies,target,dev-dependencies,build-dependencies
```

**A Windows checkout gets CRLF, and `.rustfmt.toml` asks for Unix newlines.** Every file in the tree
then fails the formatting check at once, which reads as the tree being unformatted rather than as
the checkout disagreeing with the formatter. `.gitattributes` now pins the working tree to LF for
everyone.

**Calls fail over Remote Desktop, and it is not the port.** A call sets up correctly — webrtcbin
gathers its candidates and an `m.call.answer` goes out — and then dies with
`The call pipeline failed: Could not open device`, blamed on `GstOpenalSrc`. OpenAL is a red
herring: `autoaudiosrc` tries sources in rank order, and openalsrc at 128 is simply the last one
left after `wasapi2src` (257) and `wasapisrc` (256) have both failed. Asked directly, `wasapisrc`
says what is actually wrong:

```text
Failed to get default device
```

There is no capture device, because **an RDP session does not get the machine's audio hardware**.
`$SESSIONNAME` is `RDP-Tcp#0`, the only endpoint the session can see is "Remote Audio", and the
`Microphone` the registry lists as active belongs to the console session. Microphone redirection
is off by default in the RDP client, and webcams are not redirected at all without being asked
for.

So calls have to be tested **from the physical console**, or from an RDP client with audio
recording and camera redirection both turned on. Nothing about the pipeline has been shown to be
wrong, and nothing about it has been shown to be right either — this test says neither.

Worth fixing regardless: what reaches the user is "Could not open device", naming a backend nobody
chose. "No microphone was found" is what happened.

**The message search index cannot live on disk, because a room ID is not a legal file name.**
Every sync used to log, once per room:

```text
Failed to handle events for indexing: IoError 'Os { code: 123, kind: InvalidFilename … }'
  while create directory in: '…\cache\<session>\search_index\!kTpl…:matrix.kzenjak.com'
```

The index gives each room a directory named after its room ID —
`self.path.join(self.room_id.as_str())` in `matrix-sdk-search` — and a room ID contains a colon,
which Windows will not accept in a file name. There is no hook for us in between: the SDK is
handed a base directory and appends the room ID itself.

So on Windows `search_index_store()` in `src/utils/matrix/mod.rs` returns
`SearchIndexStoreKind::InMemory` instead of the encrypted directory the other platforms use. The
index is then built from the event cache as events arrive, so search covers what has been synced
this run. Two things are given up: the index does not survive a restart, and with it goes the
encryption at rest that a stored index needed in the first place — an in-memory index has nothing
at rest to protect. It should go away if the SDK ever names those directories with something legal
everywhere.

**GLib does have a Windows notification backend**, which the plan assumed it did not, and it is a
modern one. `strings libgio-2.0-0.dll` names `GWin32NotificationBackend`, and beside it only
`RoActivateInstance` and `api-ms-win-core-winrt-l1-1-0.dll` — so it is WinRT toasts, the same API
M5 was going to reach for. That is the opposite of macOS, where a backend existed but was
deprecated past usefulness.

It will not carry an action, though:

```text
GLib-GIO-WARNING: Notification actions are unsupported by this Windows backend
```

which is precisely the half Commune depends on — every notification it sends sets a default action
with a `GVariant` target and exists to be clicked.

And nothing is delivered. After a test notification, `HKCU\…\Notifications\Settings` lists 34
applications and Commune is not among them; that key is what Windows writes when an application
first delivers a toast. Installing the MSI and launching from its AUMID shortcut did not change
it.

**The reason is not the shortcut.** An unpackaged process has no AUMID of its own —
`GetApplicationUserModelId` returns `APPMODEL_ERROR_NO_APPLICATION` — and nothing in Commune ever
gives it one. A shortcut declaring an AUMID makes the ID valid to register against; the process
still has to claim it, and ours never does.

There is a working precedent on this machine, in a sibling project by the same author:
`~/irohdp/crates/ipn-gui/src/notify.rs`. Nullgate **is** in that registry key, and it does two
things Commune did not — `SetCurrentProcessExplicitAppUserModelID(APP_ID)` from `shell32`, and an
`HKCU\Software\Classes\AppUserModelId\{APP_ID}` key carrying the `DisplayName` that Windows shows
in its notification settings. It then sends toasts through `tauri-winrt-notification` rather than
GLib, using `on_activated` for the click.

**Commune now does the first two**, in `src/utils/windows_app_id.rs`, called from `main()` before
anything can try to notify. That alone was enough for banners to start arriving — and to arrive
from the `.zip` as much as from the installer, since the process claims the ID itself rather than
needing a shortcut to have declared it.

**The sending moved off GLib entirely**, to `src/utils/windows_notifications.rs`, which talks to
`ToastNotificationManager`. Not because GLib's backend is the wrong API — it is the same one — but
because reaching it directly is the only way to the parts GLib does not expose: the `launch`
payload that survives a click, `<actions>` for buttons, `appLogoOverride` for the sender's avatar,
and `ToastNotificationHistory` for withdrawing a notification when its room is read.

That last one decided the route. `tauri-winrt-notification`, which is otherwise a shorter path and
is what Nullgate uses, **cannot withdraw a notification at all** — and Commune withdraws in five
places. The `windows` crate was already a dependency, so going direct added none.

## What differs from Linux

**Data lives in one directory.** `%LOCALAPPDATA%\commune-Devel\data` and `…\cache`, rather than the
separate data and cache directories the XDG platforms have — Windows has no system cache location
to pair with `%LOCALAPPDATA%`, so the two are told apart by a subdirectory. `%LOCALAPPDATA%` rather
than `%APPDATA%` because the latter roams to the user's other machines and our databases are far
too large for that.

**The clock format comes from a setting, not from the locale.** Windows lets somebody choose
12- or 24-hour independently of their region, and the locale does not reflect that choice, so
`src/system_settings/windows.rs` reads `sShortTime` directly. Linux watches the portal and updates
live; this is read once at startup, as macOS is.

**Sessions live in the Credential Manager**, one generic credential per session, named
`{APP_ID}/{session id}`. The design is the macOS Keychain's, for the same reason: neither store can
be searched on free-form attributes, so the session metadata is serialised into the credential blob
next to the passphrase, and the session ID lives in the name that addresses it. The application ID
already carries the profile, so a development build never sees a stable build's sessions.

**Media plays through our own `GtkMediaStream`**, because MSYS2's GTK has no media backend. Same
code as macOS, same reason.

**Images decode with the `image` crate** rather than glycin, shared with macOS through
`cfg(not(target_os = "linux"))`. The same formats are unsupported: **SVG, HEIC, AVIF and JXL** in
the timeline report "Image format not supported" per image.

**Notifications are ours rather than GLib's**, and clicking one works whether or not Commune is
running. Three registry entries make that true, all written by the application itself at startup
so that an unpacked `.zip` behaves like an installed copy:

```text
HKCU\Software\Classes\AppUserModelId\{APP_ID}
    DisplayName     the name System Settings shows beside our switch
    CustomActivator the CLSID below — what ties the class to our notifications

HKCU\Software\Classes\CLSID\{7DC899BF-…}\LocalServer32
    (Default)       "…\commune.exe" -Embedding
```

A click while Commune runs is delivered to the toast's `Activated` event. A click that has to
start it first goes through COM: Windows reads `LocalServer32`, starts the executable with
`-Embedding`, and calls `INotificationActivationCallback::Activate` once the process registers its
class factory. `Application::run` drops `-Embedding` before `GApplication` sees it, since
`HANDLES_OPEN` would otherwise take it for a file to open and refuse to start.

One wrinkle from all of this living under one AUMID per profile: `LocalServer32` is rewritten from
`current_exe()` on every launch, so **whichever build ran last is the one a cold click starts**.
Run the development build and then click a notification from the installed one, and the
development build opens. Harmless, and only confusing if you have both.

**A release build has no console.** `src/main.rs` sets `windows_subsystem = "windows"` only when
`debug_assertions` is off, so a development build keeps the console that `tracing` writes to.

**Shortcuts are unchanged.** `key_bindings::PRIMARY_MASK` is `CONTROL_MASK` everywhere except
macOS, so the Control-key bindings the Linux build has are already right here.

## Not done yet

* **The rest of M1.** Logging in, syncing and session restore are all done — the last completing
  the Credential Manager path end to end, since restoring is the one part the round-trip test
  could not cover. Still owed: image thumbnails and animated GIFs, and video and voice-message
  playback.
* **Calls, from a machine that has a microphone.** Every element they need is present, and the
  signalling half already works; the pipeline half has never had a capture device to open. See
  [What bit us](#what-bit-us). Run against Element per `doc/calls.md`, from the console.
* **Whether search actually finds anything.** The indexing errors are gone and the index is built
  in memory, but no search has been run against it from the UI.
* **Signing an actual artifact.** The pipeline is written and skips cleanly without metadata, but
  nothing has yet been signed, so neither the signtool invocation nor what SmartScreen makes of
  the result has been seen. That needs `artifact-signing-metadata.json` and the Trusted Signing
  client tools.
* **A release-profile bundle.** Everything so far is a development build; the release profile has
  never been packaged, and it is what the size and the startup time should actually be judged on.
* **Windows Sandbox.** The bundle was proven self-contained by cutting `PATH` and checking every
  loaded module, which is strong evidence but not the same as a machine that has never had MSYS2
  on it.
* **M4**: the rest of polish. Dark mode already follows the system with no work and the clock
  format is read from the setting Windows keeps for it; drag and drop and IME are unverified, and
  the embedded icon has been confirmed present in the executable but not seen in a taskbar.
* **The notification details nobody has looked at yet.** Buttons are emitted into the toast's
  `<actions>` but no notification that carries any — a ringing call — has been raised on Windows.
  Neither has a withdrawal: read a room with a notification pending and watch it leave the
  notification centre. And the toast is attributed with our name but no icon, because the
  `IconUri` value that would give it one is not written.
* **M6**: camera QR scanning. `mfvideosrc` and `mfdeviceprovider` are both present.

Unverified beyond that: GTK's win32 backend for input methods and drag and drop, which renderer GSK
picks, and whether the popover-on-a-separate-surface problem that troubles the macOS sticker picker
has a win32 sibling.

## Rebasing

Upstream Fractal has no Windows support and will not grow any. The changes most easily lost on a
rebase, in rough order:

* `src/utils/mod.rs` — the `dir_path()` split, which is easy to lose because the macOS branch sits
  next to it and looks like the whole story.
* `src/main.rs` — the `windows_subsystem` attribute at the top of the file.
* `src/meson.build` — the `exe_name` suffix and the `cargo-build.sh` command, both of which sit in
  a target upstream edits.
* `meson.build` and `data/meson.build` — the gates that read `== 'linux'` where upstream has
  nothing and the macOS port had `!= 'darwin'`.
* `src/components/media/{mod.rs,audio_player/mod.rs,content_viewer.rs}` — the `any(macos, windows)`
  cfgs, which a rebase will happily narrow back to macOS.
* `src/application.rs` — the `-Embedding` filter in `run()`, which is three lines inside a method
  upstream owns and which nothing will fail loudly without: a cold notification click simply stops
  working.
* `src/session/notifications/mod.rs` — the Windows arm of the two `cfg_if!` dispatches, which a
  rebase will narrow back to macOS-or-GLib.
* `build-aux/compile-blueprints.sh` — one `export` line in a file upstream owns.
* `build.rs` — a file upstream does not have at all, so a rebase will not conflict with it, but
  the `Cargo.toml` build-dependency and the cargo-machete ignore that go with it are in files
  upstream edits constantly.

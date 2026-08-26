# Building and running on macOS

The ledger for the macOS port. `doc/macos-plan.md` is the route that was planned before any of it
was built; this file records what actually exists, what is stubbed, and what bit us on the way.

## Contents

<!-- toc -->
* [State today](#state-today)
* [The GTK environment](#the-gtk-environment)
* [Setting the environment up](#setting-the-environment-up)
* [Building](#building)
* [Packaging](#packaging)
* [Testing by hand](#testing-by-hand)
* [What differs from Linux](#what-differs-from-linux)
* [Not done yet](#not-done-yet)
* [Rebasing](#rebasing)
<!-- /toc -->

## State today

M0, M1, M2 and M5 are done, and M3 is written and partly exercised: the menu bar, the hidden
hamburger, the `matrix:` URL scheme warm and cold, session restore and the Keychain have all been
seen working on a bundle, and so has a notification: banner, avatar, and a click that opens the
room it names. **The Command shortcuts have not been**, and the rows in
[Testing by hand](#testing-by-hand) that are not marked verified are the ones still owed — for
notifications that means a real incoming message, an identity verification, and withdrawal. The tree
builds for `aarch64-apple-darwin`, and `cargo check`, `cargo
clippy --all-targets -- -D warnings`, `cargo +nightly fmt --check`, `cargo deny`, `cargo machete`,
`cargo sort`, `typos`, `rumdl`, `cargo nextest run` and `meson test` all pass. `meson compile` and
`meson install` work, and **the app runs**: logging in with a password, syncing, the timeline,
image thumbnails, animated GIFs, the GIF search, searching the messages of a room, video in the
media viewer and restoring the session from the Keychain after a quit were all exercised on
macOS 26.

There is now a **relocatable `Commune.app`**, and a `.dmg` and a `.tar.gz` around it. It launches
from a shell with nothing exported, and loads no library from outside itself.

**The Windows port and the feature rounds behind it merged clean under this port.** The first
macOS build after the merge (`cdc9b942` and what followed) tripped over exactly two things, both
fixed since: `build-aux/cargo-build.sh` — new, shared, and executed directly by `src/meson.build` —
arrived without its executable bit, which only NTFS forgives (`6d3448e4`), and this machine's
clippy raised six lints no machine in the merge saw (`702526e3`). With those settled the whole bar
passes again on macOS 26: `cargo check`, clippy, fmt, `cargo nextest run` (148 tests),
`meson test`, `meson compile` through the new wrapper, and `macos-bundle` — a 339 MB development
bundle, deployment floor still 11.0, reference audit still clean, and every GStreamer plugin a
call needs inside it (`sctp` is absent, and only data channels — which Matrix 1:1 calls never
open — would miss it). What the merge owes this platform is eyeball time, not code: none of the
200 checks in `doc/eyeball-tests.md` has been run here — see [Not done yet](#not-done-yet).

The environment it all needs is created by a script in `build-aux/macos/`.

| Area | State |
| --- | --- |
| Runtime paths | `src/utils/app_bundle.rs`, relative to the executable inside a bundle |
| Image decoding | Rewritten behind `src/utils/media/image/decoder/`, `image` crate on macOS |
| Video and audio playback | Own `GtkMediaStream`, `src/components/media/gst_media_stream.rs` |
| Secrets | macOS Keychain, `src/secret/macos.rs` |
| Data directories | `~/Library/Application Support` and `~/Library/Caches` |
| `.app` bundle, `.dmg`, `.tar.gz` | `build-aux/macos/bundle.sh` and its two wrappers |
| Location sharing | Stubbed, `is_available()` is false and the UI hides it |
| System 12/24h clock | Locale-derived at startup, never updates live |
| Camera QR scanning | Stubbed, returns no cameras |
| Application icon | Its own artwork, `assets/macos-legacy-bevel.svg` |
| Menu bar | `src/macos_menu_bar.blp`, with `win.` forwarders on `Window` |
| Keyboard shortcuts | `<Primary>` throughout, so Command rather than Control |
| `matrix:` URLs | Our own Apple Event handler, `src/utils/macos_url_events.rs` |
| Notifications | `UNUserNotificationCenter`, `src/utils/macos_notifications.rs` |

## The GTK environment

The GTK stack comes from a **conda-forge** environment created with `micromamba`, never from
Homebrew. This is the one decision that everything else rests on, so it is worth stating why.

conda-forge builds its `osx-arm64` packages against the macOS 11 SDK and its `osx-64` packages
against roughly 10.13. Every dylib we bundle therefore carries a `minos` of 11.0 no matter which
macOS built it. Homebrew instead stamps the **build host's** OS version into what it builds, so a
bundle produced on a current machine demands that same OS from everyone who installs it. The same
mistake was already shipped once in the sibling SEED Sync project and had to be withdrawn.

Check any prefix before trusting it:

```sh
otool -l "$GTK_PREFIX/lib/libgtk-4.dylib" | grep -A3 LC_BUILD_VERSION   # expect minos 11.0
```

`build-aux/macos/probe-env.sh` does this, and much else, in one pass.

There is a second trap on this machine specifically. Two Homebrew installations exist:
`/usr/local` is **x86_64** and comes first on `PATH`, and `/opt/homebrew` is arm64. So the default
`pkg-config`, `glib-compile-resources` and `msgfmt` are the Intel ones on an Apple Silicon
machine, and an unpinned build silently produces a Rosetta binary. Setting `PKG_CONFIG_LIBDIR` as
well as `PKG_CONFIG_PATH` is what prevents this: `PKG_CONFIG_LIBDIR` _replaces_ the default search
path rather than adding to it, so nothing can leak in.

Not everything Commune needs is packaged by conda-forge. These are built from source into the
environment by the setup script:

* **`blueprint-compiler`** — there are 154 `.blp` files and `meson.build` requires the compiler
  unconditionally, so nothing configures without it. Built from the GNOME repository at v0.18.0.
* **`gst-plugin-gtk4`**, which provides `gtk4paintablesink` — video has no sink without it. Built
  from `gst-plugins-rs` with `cargo-c`.

  **The `gst-plugins-rs` branch is named after the gstreamer-*rs* binding version, not the
  GStreamer C version.** Branch `0.15` is the one that uses the `gstreamer` 0.25 crates that
  `Cargo.toml` pins; there is no `1.28` branch, and asking for one fails with
  `fatal: Remote branch 1.28 not found`. Bump the branch and the crate versions together.

  `cargo-c` is built while the environment is on `PATH`, so it links the environment's `libssl`
  and then cannot find it again from a plain shell (`Library not loaded: @rpath/libssl.3.dylib`).
  The setup script exports `DYLD_FALLBACK_LIBRARY_PATH` rather than fighting over which OpenSSL
  it picks.
* **The `Shumate-1.0` and `GtkSource-5` typelibs.** conda-forge builds both libraries without
  introspection, so neither ships a typelib — and blueprint-compiler resolves `using Shumate 1.0`
  and `using GtkSource 5` through typelibs. Without them not a single `.ui` file compiles, so
  nothing builds at all:

  ```text
  error: Namespace Shumate-1.0 could not be found
  ```

  The script builds both libraries from source at the version conda-forge installed and copies
  **only** the `.typelib` and `.gir` into the environment. The libraries themselves are left as
  conda-forge built them: those carry the macOS 11 floor that is the entire reason for using
  conda-forge, and anything compiled here would be stamped with this machine's SDK instead.

  Two option names in that build are easy to get wrong. libshumate's introspection option is
  `-Dgir`, not `-Dintrospection`, and it has **no** working `vector_renderer` option — as of 1.6
  the vector renderer is always enabled, and conda-forge's build already has it. gtksourceview
  uses `-Dintrospection=enabled`. Both need their sysprof subproject disabled, which does not
  build on macOS (`fatal error: 'config.h' file not found`).
* **`webrtcbin`, and the two libraries under it.** conda-forge's `gst-plugins-bad` carries the
  `libgstwebrtc-1.0` _library_ and the `gstreamer-webrtc-1.0.pc` that `Cargo.toml` links against,
  so the Rust side builds and links perfectly well — but it does **not** carry the `webrtc`,
  `nice` or `srtp` _plugins_, and there is no `libnice` or `libsrtp` package anywhere in the
  channel for it to have built them from. Every other element a call needs is present. The result
  is a client that compiles cleanly and then ends every call with a missing-element error, which
  is exactly the failure `calls.md` warned about under "Runtime requirements": these are plugins
  found at runtime, not Meson dependencies, so nothing fails at build time.

  So `build_webrtc()` builds three things, in the order they depend on each other:

  | Built | Why |
  | --- | --- |
  | libsrtp2 | the ciphers under `srtpenc`, which `dtlssrtpenc` makes **by name** at runtime |
  | libnice | ICE, and the `nice` plugin (`nicesrc`/`nicesink`) that ships with it |
  | gst-plugins-bad | rebuilt at the version conda-forge installed, everything but `webrtc`, `srtp`, `dtls` and `sctp` switched off |

  Both libraries are built against the environment's OpenSSL (`-Dcrypto-library=openssl`) rather
  than their built-in ciphers, which is what the AES-GCM profiles WebRTC negotiates need.

  **libnice must be at least 0.1.23.** That is what `gst-libs/gst/webrtc/nice/meson.build` asks
  for, and an older one fails the check silently: `libgstwebrtcnice` is not built, so
  `libgstwebrtcnice_dep` is not found, so the `webrtc` plugin is quietly dropped from the build
  with no error anywhere.

  **`dtls` and `sctp` are enabled because the `webrtc` option requires them to be**, not because
  their plugins are wanted — meson refuses to configure otherwise, with
  `Feature dtls cannot be disabled: webrtc option is enabled`. conda-forge already ships both, and
  neither is copied out.

  Only three files come out of the staging prefix: `libgstwebrtc.dylib`, `libgstsrtp.dylib` and
  `libgstwebrtcnice-1.0`. gst-plugins-bad also builds a dozen libraries conda-forge already
  ships — `libgstwebrtc-1.0`, `libgstsctp-1.0`, `libgstcodecparsers-1.0` — and overwriting those
  with copies compiled here would replace the macOS 11 deployment floor with this machine's SDK,
  which is the whole reason for using conda-forge. `libgstwebrtcnice-1.0` is the exception
  because it exists nowhere in the environment: it is the half of gst-plugins-bad that needs
  libnice. Everything built here carries `minos 11.0`, because `build_extras()` exports
  `MACOSX_DEPLOYMENT_TARGET`; check with `vtool -show-build`.

  The GStreamer registry caches which plugins exist, so `~/.cache/gstreamer-1.0` is cleared after
  the copy. A stale registry hides a plugin that has just appeared, which looks exactly like a
  build that did not work.

  This is the slow part of the setup and it is the same three projects at the same pinned
  versions on every machine, so it is the obvious candidate for prebuilt artifacts if it ever
  becomes a nuisance.

`shared-mime-info` is also absent from conda-forge. Nothing so far needs it.

Two caches have to be built after the packages are installed, and nothing works without them.
`setup-conda-macos.sh` does both:

* **`gschemas.compiled`.** conda-forge ships the schema XML but never runs `glib-compile-schemas`.
  `src/application.rs` aborts on startup without the app's schema, and the file chooser fails with
  "No GSettings schemas are installed" even if it gets that far.
* **`loaders.cache`.** The librsvg loader is installed as `libpixbufloader_svg.dylib` while every
  other loader is a `.so`, and the cache conda-forge ships lists only the `.so` files — so SVG is
  absent and the whole symbolic icon set fails to load. The fix is to pass both extensions to
  `gdk-pixbuf-query-loaders` explicitly instead of letting it scan.

## Setting the environment up

```sh
build-aux/macos/setup-conda-macos.sh              # osx-arm64
build-aux/macos/setup-conda-macos.sh --universal  # + osx-64, for a universal binary later
```

The script wants a `micromamba`, `mamba` or `conda`. It looks for one vendored at
`.conda-gtk/.bin/micromamba` first, then `COMMUNE_CONDA_BIN`, then `PATH`. Vendoring one keeps the
environment independent of whatever is installed globally. Everything it creates lives under
`.conda-gtk/`, which is gitignored.

Three composition details in that script are not obvious and should not be removed:

* `zlib`, `freetype` and `expat` are installed for their `.pc` files. conda-forge splits the
  runtime libraries (`libzlib`, `libfreetype`) from the development packages, but gio, harfbuzz and
  fontconfig name them in `Requires.private`, so the Rust `*-sys` builds need the `.pc` present.
* `libintl-devel` provides the unversioned `libintl.dylib` symlink that the linker needs for the
  `-lintl` that glib's `.pc` emits, and the `libintl.h` that lets `gettext-rs` link the prefix's
  libintl instead of compiling its own vendored copy.
* `libxml2-devel` is installed for the headers and the `libxml-2.0.pc`, neither of which the
  conda-forge libxml2 runtime package ships. appstream (pulled in by libadwaita) lists
  `libxml-2.0` in `Requires.private`, and pkg-config fails on a missing private dependency even
  for a dynamic build, so the `libadwaita-1` probe fails without the `.pc`; gtksourceview needs
  the headers themselves and fails with
  `no such include directory: '…/include/libxml2' [-Werror,-Wmissing-include-dirs]`. The script
  still synthesizes a minimal `.pc` as a fallback for a prefix that somehow lacks the devel
  package, but a synthesized `.pc` is not enough to build gtksourceview. Pinning libxml2 back
  instead would drag gtk4 down to 4.14.

Then check what you got:

```sh
export PKG_CONFIG_PATH=$PWD/.conda-gtk/arm64/lib/pkgconfig
export PKG_CONFIG_LIBDIR=$PWD/.conda-gtk/arm64/lib/pkgconfig
export PATH=$PWD/.conda-gtk/arm64/bin:$PATH
export GI_TYPELIB_PATH=$PWD/.conda-gtk/arm64/lib/girepository-1.0
export XDG_DATA_DIRS=$PWD/.conda-gtk/arm64/share
export MACOSX_DEPLOYMENT_TARGET=11.0

sh build-aux/macos/probe-env.sh
```

`probe-env.sh` is read-only. It prints one row per dependency as
`name | found | version | required | status` and ends with a list of what still has to be built.

## Building

With the environment exported as above:

```sh
meson setup _build -Dprofile=development
meson compile -C _build
```

Or straight to cargo, which is faster while iterating on Rust — but **export `CARGO_HOME` as well**:

```sh
export CARGO_TARGET_DIR=$PWD/_build/cargo-target
export CARGO_HOME=$PWD/_build/cargo-home
cargo check
cargo clippy --all-targets -- -D warnings
```

`meson.build` sets both for the cargo it runs. Setting only the target directory puts the two
invocations in the same one with different registry paths, which changes the fingerprint of every
dependency, so each rebuilds the whole tree the other just built. It looks like a slow machine
rather than a mistake, and on this one it is about five minutes each way.

To install and run the development build:

```sh
meson install -C _build

DYLD_FALLBACK_LIBRARY_PATH=$PWD/.conda-gtk/arm64/lib \
XDG_DATA_DIRS=$PWD/_install/share:$PWD/.conda-gtk/arm64/share \
GSETTINGS_SCHEMA_DIR=$PWD/_install/share/glib-2.0/schemas \
GDK_PIXBUF_MODULE_FILE=$PWD/.conda-gtk/arm64/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache \
GST_PLUGIN_SYSTEM_PATH_1_0=$PWD/.conda-gtk/arm64/lib/gstreamer-1.0 \
    _install/bin/commune
```

Notifications are off in a run like that, and say so once at startup:

```text
WARN commune::utils::macos_notifications: Not running from an app bundle; notifications are off
```

`UNUserNotificationCenter.currentNotificationCenter` **raises**
`NSInternalInconsistencyException` rather than returning nil when the process has no bundle
identity, and an Objective-C exception unwinding back into Rust takes the whole application down
before the first window is drawn:

```text
*** Terminating app due to uncaught exception 'NSInternalInconsistencyException',
    reason: 'bundleProxyForCurrentProcess is nil: mainBundle.bundleURL file:///…/_install/bin/'
```

So `macos_notifications::center()` asks `NSBundle` for a bundle identifier first, and every one of
the three places that reaches for the center goes through it. Testing notifications needs the
bundle; everything else runs fine without it.

`DYLD_FALLBACK_LIBRARY_PATH` is needed because the binary comes out with `@rpath/…` install names
and **no `LC_RPATH`** — Meson reports "Skipping RPATH fixing" for a cargo-built binary it only
copies into place, and cargo adds no rpath of its own. Without it the launch dies immediately with
`Library not loaded: @rpath/libgstapp-1.0.0.dylib … Reason: no LC_RPATH's found`. This is a
development-run concern only: M2's bundle rewrites every install name to
`@executable_path/../Frameworks`, so a bundled app needs none of these variables.

The pre-commit hook's tools are all cargo-installable, but `cargo-nextest` refuses to build
without `--locked`:

```sh
cargo install cargo-nextest --locked
cargo install cargo-deny cargo-machete cargo-sort typos-cli rumdl grass
```

Two build settings are worth knowing about.

`meson.build` exports `GETTEXT_SYSTEM=1` and `GETTEXT_DIR` into the cargo environment on darwin
when it finds `libintl.h` in the prefix. Without those, `gettext-sys` compiles and statically links
its own vendored libintl, so the binary would carry two of them.

`.cargo/config.toml` carries a `target.'cfg(target_os = "macos")'` table that adds
`-Wl,-headerpad_max_install_names`. Bundling rewrites short `@rpath/…` install names into longer
`@executable_path/../Frameworks/…` ones, and without reserved header space `install_name_tool`
fails with "load commands do not fit". **That table repeats the `ruma_identifiers_storage` cfg on
purpose**: cargo takes rustflags from exactly one source, and a matching `target.<cfg>` table
replaces `[build]` rather than adding to it. Dropping the repeat silently changes how ruma stores
identifiers.

## Packaging

```sh
meson compile -C _build macos-bundle    # Commune.app
meson compile -C _build macos-dmg       # ... and a .dmg
meson compile -C _build macos-tarball   # ... and a .tar.gz
```

All three targets exist only on darwin, and each assembles the bundle from scratch out of a staged
`meson install` of whatever the build directory was configured as. The result lands in
`<build-dir>/macos/`. `build-aux/macos/README.md` has the direct invocation.

**The packaged build is the `default` profile**, not `development`: `meson setup _build-release
-Dprofile=default`. That is what makes it `Commune.app` with the application ID
`io.github.steeb_k.Commune` and the plain icon. A development build is deliberately named
`Commune Devel.app` so that the two can sit in `/Applications` together — on Linux the profiles are
told apart by their application ID and icon, but here the bundle directory name has to differ
because two bundles cannot share one name.

`Contents/Resources` is laid out as a small Unix prefix, and that layout is a contract with
`src/utils/app_bundle.rs`, which reads it back at startup and points GLib, GdkPixbuf, GStreamer and
fontconfig at it. Moving anything in one means moving it in the other.

**`Info.plist` never sees the rc suffix.** Its two version keys accept nothing but
period-separated numbers — LaunchServices shrugs at anything else today, but notarization and the
App Store validate — and Apple's three-integer format cannot even express "before 1.0" as a
suffix, since `1.0.1` would order _above_ a stable `1.0`. That is the problem Debian's `~` solves
and Apple simply does not have, so `bundle.sh` follows Apple's own model instead of fighting it:
`CFBundleShortVersionString` is the marketing version's longest numeric prefix (`c36f52dc`) —
`1.rc1` goes in as `1`, a stable `1` or `1.1` passes through whole — and `CFBundleVersion` is not
a version at all but a **build number**, the commit count of the checkout (`28351aa7`), monotonic
across every rc and release with nothing to remember at release time. Outside a git checkout it
falls back to the numeric prefix. The full `1.rc1` still appears everywhere a human reads a
version: the About dialog, and the artefact names, which take it from the bundle's own
`CommuneVersion` key (`9e840906`) because the Apple keys no longer carry it.

### The release profile does not fit in 8 GB

`Cargo.toml` asks for `debug = true`, `lto = "thin"` and `codegen-units = 1`. On the machine this
port was built on — 8 GB of RAM — compiling the `commune` crate itself with those settings reaches
about **11 GB resident** and then stops making progress: the CPU goes idle, swap fills, and CPU time
advances by fractions of a second per wall-clock second. It is thrashing, not compiling, and it does
not recover.

`codegen-units = 1` is also single-threaded for that crate, so none of the other cores help.

Override the profile for the packaging build rather than editing `Cargo.toml`, which the Flatpak and
Linux builds share:

```sh
export CARGO_PROFILE_RELEASE_DEBUG=false
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
meson compile -C _build-release macos-tarball
```

Dropping the debug info costs no optimization at all and is what makes the bundle a sane size —
debug symbols are why a development bundle comes out at 320 MB. Sixteen codegen units restores
parallelism and cuts peak memory to a few GB; against `codegen-units = 1` it gives up a few percent
of runtime performance, which is the right trade when the alternative is no build.

### There is no dylibbundler

Every dylib conda-forge builds already has an `@rpath/<basename>` install name, so bundling is a
flat copy into `Contents/Frameworks` plus one `LC_RPATH` per Mach-O file — no per-dependency
rewriting, and no dependency on a tool that is not otherwise part of this environment. The rpath is
`@loader_path/<however many ../ it takes>/Frameworks`, computed per file, which works for the
executable, the libraries, the plugins and the out-of-process `gst-plugin-scanner` alike:
`@loader_path` in a main executable means the same thing as `@executable_path`.

The one thing that does get rewritten is the install name of anything built outside conda-forge.
`libgstgtk4.dylib` is built here by `cargo-c` and carries the absolute path it was built at, which
would otherwise be indistinguishable from a real leak in the audit below.

### The `otool -L` audit is the point

A bundle that still names a path on the machine that built it works perfectly there and fails on
the first machine it is copied to. That is the hardest kind of failure to notice, so `bundle.sh`
ends by walking every Mach-O file in the bundle and asserting that nothing outside `/usr/lib`,
`/System`, `@executable_path`, `@rpath` and `@loader_path` remains.

Three more guards exist because each of these went wrong once:

* **`loaders.cache` must be generated against the prefix, never against the staging tree.** Run
  against the prefix, `gdk-pixbuf-query-loaders` emits paths relative to it, and gdk-pixbuf resolves
  them against whatever directory it finds the cache in — so the same file works from
  `Contents/Resources` wherever the user drags the app. Run against a staging directory it emits
  absolute paths and pins the bundle to this machine. The script checks and refuses.

  When writing that check, note that a bare `^"/` also matches `"/*"`, which is not a path but the
  magic signature the XPM loader recognises files by. Match the module-path lines, which are the
  ones ending in `.so"` or `.dylib"`.

* **fontconfig's `conf.d` has to be copied with symlinks dereferenced.** Every file in it is a link
  to `../../../share/fontconfig/conf.avail/`, which is outside the directory being copied, so a
  plain `cp -R` leaves two dozen dangling symlinks and silently loses most of the font
  configuration. `cp -RL` is the fix.

  A dangling symlink is also invisible to `otool`, and it makes `codesign --verify --strict` fail
  with a bare `No such file or directory` that names only the bundle and gives no hint which file is
  at fault. `bundle.sh` therefore checks for dangling symlinks _before_ it signs, so that the error
  says something useful.

* **The deployment target is measured, not declared.** `LSMinimumSystemVersion` is the highest
  `minos` of anything the bundle carries. It should come out at 11.0; if it comes out as this
  machine's OS version then something Homebrew-built has crept in, which is the whole reason for
  [the conda-forge rule](#the-gtk-environment).

### Signing, and why the Keychain keeps asking

arm64 refuses to run unsigned code at all, so even a bundle nobody is going to distribute has to be
signed with something. `bundle.sh` signs nested code first and the bundle last — `--deep` would do
it in one call but is deprecated and signs in an order `codesign` itself warns about.

The default is an ad-hoc signature (`-`), which is a **new identity on every build**. The Keychain
binds an item's access control to the signing identity, so every rebuild makes macOS ask again
whether the app may read the session it stored last time. The fix is a stable identity: make a
self-signed **Code Signing** certificate in Keychain Access (Certificate Assistant → Create a
Certificate, type "Code Signing", self-signed), then

```sh
CODESIGN_IDENTITY="Commune Dev" meson compile -C _build macos-bundle
```

### `.dmg` or `.tar.gz`

Both are built, and the difference matters more than it looks.

A `.dmg` is the familiar shape — open it, drag the icon onto `Applications`. But anything a browser
downloads is tagged `com.apple.quarantine`, and Gatekeeper will not accept an ad-hoc or self-signed
signature for a quarantined app, so the **first launch is refused outright**. Until there is a
Developer ID to sign and notarize with, the `.dmg` is the artefact that will not open.

Files extracted from a tarball on the command line are never quarantined in the first place, which
is why the sibling SEED Sync project ships a `curl | sh` tarball. **`make-tarball.sh` produces the
artefact to actually hand to somebody today**; `make-dmg.sh` exists because it is the right shape
once the signing story is sorted, and because a locally built `.dmg` is a fine way to test the
install itself.

## Testing by hand

There is no macOS CI and there will not be one for a while, so this is the list. Everything here
has to be exercised from a **bundle** unless the row says otherwise: `matrix:` URLs and
notifications both need a `CFBundleIdentifier`, and a build run out of a prefix does not have one.

**Check that the bundle you are testing is the one you just built.** There are up to four copies of
this application on a development machine — `_build/macos/Commune Devel.app`,
`_build-release/macos/Commune.app`, whatever was dragged into `/Applications`, and whatever a `.dmg`
or `.tar.gz` was unpacked to — and each build target only rebuilds its own. A whole milestone was
once reported as "completely unchanged" because the copy in `/Applications` came from a
`_build-release` that predated it. `grep` the binary for something the change introduced:

```sh
strings /Applications/Commune.app/Contents/MacOS/commune | grep -c macos_menu_bar
```

```sh
meson compile -C _build macos-bundle && open _build/macos/"Commune Devel.app"
```

To see what the app is saying while it runs, launch the binary inside the bundle directly instead
of with `open`; it is a normal executable and needs nothing exported:

```sh
RUST_LOG=commune=debug _build/macos/"Commune Devel.app"/Contents/MacOS/commune
```

| # | Area | Do this | Expect |
| --- | --- | --- | --- |
| 1 | Menu bar | Look at it | Commune, File, Edit, View, Window, Help — **verified** |
| 2 | Menu bar | Commune → About Commune | The About dialog, named Commune, not `commune` |
| 3 | Menu bar | Commune → Preferences, and ⌘, | The account settings of the visible session |
| 4 | Menu bar | Preferences while logged out | Greyed out |
| 5 | Menu bar | Commune → Hide, Hide Others, Show All, Quit | The usual macOS behaviour |
| 6 | Menu bar | File → each of the five items | The same dialogs the old hamburger menu opened — **verified** |
| 7 | Menu bar | File and View on the login page | Greyed out; sensitive again once a session is up |
| 8 | Menu bar | Edit → Cut, Copy, Paste, Select All | Greyed, but showing ⌘X ⌘C ⌘V ⌘A |
| 9 | Menu bar | ⌘X, ⌘C, ⌘V, ⌘A in the composer | They work, greyed menu items notwithstanding |
| 10 | Menu bar | View → the five room items, Full Screen | Selection moves; the window goes full screen — **verified** |
| 11 | Menu bar | Window | Minimize, Zoom and the window list, from AppKit |
| 12 | Menu bar | Help → Keyboard Shortcuts | The shortcuts dialog |
| 13 | Sidebar | Look at the header bar | No hamburger button — **verified** |
| 14 | Shortcuts | ⌘Q, ⌘W | Quit; close window |
| 15 | Shortcuts | ⌘K, ⌘L, ⌘, | Room search; join room; account settings |
| 16 | Shortcuts | ⌘Page Up, ⌘Page Down, and both with ⇧ | Previous/next room, then the unread ones |
| 17 | Shortcuts | ⌘⇧8 (⌘\*) | Jumps to the first room with unread messages |
| 18 | Shortcuts | ⌘V into the composer | Pastes, including an image |
| 19 | Shortcuts | Help → Keyboard Shortcuts, read the list | ⌘ glyphs throughout, no ⌃ |
| 20 | `matrix:` URL | Running: `open -a <bundle> 'matrix:r/<room>:<server>'` | Comes forward and opens the room — **verified** |
| 21 | `matrix:` URL | Quit first, then the same command | It launches, restores the session and opens the room — **verified** |
| 22 | `matrix:` URL | Bare `open 'matrix:…'`, no `-a` | Goes to whichever app owns the scheme; see below |
| 23 | `matrix:` URL | Click a `matrix:` link in another app | Same as 20 |
| 24 | Notifications | `COMMUNE_TEST_NOTIFICATION=1`, see below | A banner: room name, body, and the room's avatar — **verified** |
| 25 | Notifications | Click that banner | The room it names opens, in the right session — **verified** |
| 26 | Notifications | Background the app, have somebody send a real message | The same, for the real event |
| 26a | Notifications | Same for an identity verification request | The verification opens |
| 26b | Notifications | Read a room that has a notification pending | The banner leaves Notification Center |
| 26c | Notifications | Rebuild with the same `CODESIGN_IDENTITY`, relaunch | No second permission prompt — **verified** |
| 27 | Keychain | Log out of a session | Its item is gone from Keychain Access — **verified** |
| 28 | Keychain | Then look in `~/Library/Application Support/commune-Devel/` | The session's directory is gone |
| 29 | Regression | Log in with a password, and with SSO | Both work; SSO may raise a firewall prompt |
| 30 | Regression | Send and receive text; open a room's history | Nothing unusual |
| 31 | Regression | An image thumbnail, an animated GIF, the GIF search | All render, animation included |
| 32 | Regression | A video in the media viewer | Plays, with working controls |
| 32a | Regression | An audio clip, in the timeline and the viewer | Plays, with a moving waveform — **verified** |
| 32b | Icons | Look at the app in Finder and the Dock | The macOS plate, not the GNOME icon; see the cache note below |
| 33 | Regression | Quit and relaunch | The session comes back without a login — **verified** |
| 34 | Regression | Launch the bundle from a shell with nothing exported | It runs |
| 35 | Search | ⌘F in a room | The search bar opens, entry focused, composer hidden — **verified** |
| 36 | Search | Type a word in an unencrypted room | Results newest first, with sender, avatar and date — **verified** |
| 37 | Search | Activate a result | A timeline centred on that message, and a "Back to Latest" pill — **verified** |
| 38 | Search | Click "Back to Latest" | The live timeline again — **verified** |
| 39 | Search | Escape, or the × in the search bar | The search closes and the composer comes back — **verified** |
| 40 | Search | Search an encrypted room, then "Re-index This Room" | Messages loaded in the room become findable |
| 41 | Search | `ls ~/Library/Caches/commune-Devel/<session>/search_index` | One directory per room, and no plain text in them — **verified** |

**The "Back to Latest" pill is the row worth watching in a bundle.** Its `go-last-symbolic` is not
one of the icons in `data/resources/icons/`, so unlike everything else in the search UI it has to
come out of the Adwaita theme the bundle carries, through the `loaders.cache` that
[the GTK environment](#the-gtk-environment) section exists to get right. An icon that renders in a
prefix run proves nothing about the bundle here.

Row 40 is the one that will look broken and is not. The local index is only fed as the event cache
stores an event, so every message that was already stored before the index existed — which is all of
them, on a machine that ran a build without `experimental-search` — is missing from it, and no
search finds it. Paginating fetches events the cache does not have and those get indexed, which is
why scrolling back makes old messages findable while recent ones stay invisible. "Re-index This
Room" hands the events the room has loaded to the index after the fact. None of this is macOS
specific; a Linux install that predates the feature has the same hole.

Both `matrix:` rows have been run, and the cold one is the interesting result: a cold launch queues
the event before the handler exists, and `AppKit` still delivers it afterwards. Launching a quit app
with `open -a … 'matrix:r/matrix:matrix.org'` brought it up, restored the session from the Keychain,
and opened the room preview dialog for a room the account is not in — the whole path, on the release
bundle, in one go.

**A changed icon will not look changed.** macOS caches application icons hard, and replacing a
bundle in place is exactly the case it gets wrong: Finder and the Dock keep showing the old icon
long after the new one is installed. Check the bundle rather than the screen before believing
anything is broken —

```sh
md5 /Applications/Commune.app/Contents/Resources/commune.icns   # against a fresh make-icns.sh run
```

— and if it matches, it is the cache. `touch` the bundle, re-register it, and restart both:

```sh
touch /Applications/Commune.app
lsregister -f /Applications/Commune.app
killall Dock; killall Finder
```

**Pass `-a` and the bundle.** More than one application on a developer's machine claims the
`matrix:` scheme — `lsregister -dump | grep matrix:` will list them — so a bare `open 'matrix:…'`
proves nothing about Commune unless Commune happens to have won the scheme. Naming the bundle takes
LaunchServices' choice out of it.

**Notifications need the app to be authorized, once.** The first launch of a bundle macOS has not
seen before raises the system's permission prompt, and until it is answered nothing is delivered.
The log says which way it went, at debug level:

```sh
RUST_LOG=commune=debug _build/macos/"Commune Devel.app"/Contents/MacOS/commune 2>&1 \
    | grep macos_notifications
# Listening for notification taps
# Allowed to show notifications
```

If instead it says _not allowed_, look in System Settings → Notifications before touching any code.
And if the prompt never appears at all, the cause is more likely LaunchServices than the
notification code — see [Signing](#signing-and-why-the-keychain-keeps-asking) below.

**Sending one on demand.** A real notification needs somebody else to send a message, which makes
the parts most likely to break — the avatar, the payload, and what a click does — the parts
hardest to get at. Development builds therefore send one on request:

```sh
COMMUNE_TEST_NOTIFICATION=1 RUST_LOG=commune=debug \
    _build/macos/"Commune Devel.app"/Contents/MacOS/commune
```

A couple of seconds after a session goes ready, this sends a notification for a room through the
same `Notifications::send_notification` everything else uses — the room's real avatar, a real
`SessionIntent::ShowMatrixId`, the lot. Clicking it should open the room it names. The log carries
both ends:

```text
commune::session::notifications: Sending a test notification id="…//matrix:roomid/…//test"
commune::utils::macos_notifications: Opening a tapped notification action="show-matrix-id"
commune::application::imp: `app.show-matrix-id` action activated
commune::application::imp: Processing session intent… intent=ShowMatrixId(Room(…))
```

It is `#[cfg(debug_assertions)]`, so a release bundle does not carry it. **Which room it picks is
not worth reading anything into** — it takes the one on screen if there is one and the first in the
list otherwise, and a restart does not reliably restore a selection, so it often is not the room
you were last looking at. What matters is that the room the banner names is the room the click
opens.

There is also an `app.test-notification` action behind the same `cfg`. Nothing reaches it today:
a key binding was tried first and `AppKit` swallows every Command combination that is not in the
menu bar, which is why the environment variable exists at all.

Rows 1 and 13 are done: the menu bar comes up as Commune, File, Edit, View, Window, Help, and the
sidebar header carries nothing but the account switcher and the search toggle. What is left in
between is the behaviour of the individual items and the Command keys, rows 2 to 12 and 14 to 19.

## What differs from Linux

**Image decoding.** glycin is Linux-only — it is a set of C libraries that decode in a sandboxed
subprocess over D-Bus. `src/utils/media/image/decoder/` puts a small API in front of it
(`Loader`, `Image`, `Frame`, `FrameRequest`, `Error`) and picks a backend with `cfg_if!`. Linux
still gets glycin; macOS gets the pure-Rust `image` crate, which is the closest thing to glycin's
safety property available in-process.

The macOS backend never decodes on the main thread. A still image goes to the Tokio blocking pool.
An animated image gets a dedicated thread that owns its decoder and hands out one frame per
request, so a long animation is streamed rather than held in memory — the same shape as glycin's
subprocess, without the process boundary. The thread exits when the last `Image` handle drops.

It reads one frame ahead when it starts, because a GIF container has no cheap frame count and a
single-frame GIF has to be reported as a still image rather than a one-frame animation.

**Formats not supported on macOS: HEIC, AVIF and JXL.** They surface in the UI as "Image
format not supported". BMP, GIF, ICO, JPEG, PNG, APNG, TIFF and WebP all work, animated where the
format allows. SVG came off this list with the Windows port: anything the `image` crate does not
recognise is now handed to GdkPixbuf, which brings whatever loaders the platform ships, and the
conda-forge prefix ships librsvg's — so an SVG renders as a still instead of an error. It is the
only format the fallback gains here, because conda-forge packages no HEIC or AVIF loader, where
MSYS2 on Windows has both. Two caveats. A dev prefix created before the fallback existed has a
`loaders.cache` without the SVG loader — re-run the setup script (or its
`gdk-pixbuf-query-loaders` line) before concluding the fallback is broken. And the fallback's
tests are `#[gtk::test]`, which macOS cannot run, so this path is eyeball-only here. An
ImageIO-backed decoder would close the remaining gap and is the obvious later option.

**Attachments used to send as plain files.** GIO's content types are MIME types only on Linux:
macOS reports UTIs (`public.png`) and Windows reports registry extensions (`.png`), neither of
which parses as a MIME type, so every attachment fell back to `application/octet-stream` and
arrived in the timeline as a downloadable file row rather than a picture.
`FileInfo::try_from_file` now asks GIO to translate its own platform's type
(`g_content_type_get_mime_type`, which is `public.png` → `image/png` here — measured against the
prefix's GLib) before parsing it (`cd668da1`). The bug was shared with Windows and the fix is
shared too; only Linux never saw it. A test in `utils/media` pins the behaviour on all three
platforms, since it needs no GTK and so is not caught by the `#[gtk::test]` exclusion.

**Dropping and pasting files was broken by GTK, and is repaired on the way in.** GTK 4.22's macOS
backend builds the `text/uri-list` for a file dropped on the window — or copied in the Finder —
by percent-encoding the whole assembled `file://…` string, and the scheme's own colon comes out
as `%3A`. GIO cannot parse a scheme from `file%3A//…`, so every drop and every pasted file
surfaced a file with no path and failed with "Error reading file", no special characters in the
name required. Upstream introduced this in `8d3e15b8` (in every 4.22 release) and fixed it on
`main` in `2a8a2895` (May 2026), which no 4.22 tag carries. Measured against AppKit directly: the
broken and the fixed backend produce byte-identical URIs except for that one `%3A`. So
`utils::repair_pasteboard_file` (`afe47e75`) puts the colon back — `send_file_inner` runs every
incoming file through it, covering the drop target and the clipboard alike — and the repair stops
matching the
day a fixed GTK arrives, because a healthy local file has a path and is passed through untouched.

**Video and audio playback.** `GtkVideo` plays a file with `GtkMediaFile`, and `GtkMediaFile` has
no backend of its own: GTK 4.22 compiles a GStreamer one into `libgtk`, but only when the
GStreamer libraries happen to be found while GTK itself is built, and conda-forge's `gtk4` is
built without them. `nm` on their `libgtk-4.1.dylib` finds not one `gst` symbol. Nothing reports
this — the viewer just shows an empty frame, a duration of zero and no error on the bus.

Nothing can be dropped in to fix it either. GTK moved those sources into `libgtk` in 4.14; before
that they were a loadable module under `lib/gtk-4.0/4.0.0/media/`, and while `gtkmediafile.c`
still scans that directory, the backend is no longer built as something that could be put there.

So `src/components/media/gst_media_stream.rs` is our own, and only exists on macOS.
It is the same shape as GTK's `GtkGstMediaFile`, built out of the pieces the timeline already
plays video with: a `GstPlay` rendering into `gtk4paintablesink`, delegating `GdkPaintable` to
the sink's paintable. `GtkVideo` and its `GtkMediaControls` need no changes, since all they want
is a media stream that is also a paintable. `content_viewer.rs` picks between the two in
`set_video_file()` and `clear_video()`.

**Audio clips had the same bug and the same fix**, found later because the symptom is quieter: the
player showed the "not playable" glyph and made no sound. `audio_player/mod.rs` asked for a
`GtkMediaFile` too, so it now holds a `gtk::MediaStream` and picks a backend with the same pair of
`cfg`'d constructors. Nothing between them changed — every notification it listens to, duration,
timestamp, playing, prepared and error, belongs to `GtkMediaStream` rather than to `GtkMediaFile`.
Tearing down needed its own pair, because `clear()` is `GtkMediaFile`'s alone; on macOS there is
nothing to do, since `GstMediaStream` stops its pipeline when it is disposed.

That swap uncovered a second half to the same bug: clips stopped refusing to play and started
spinning forever instead. The audio player waits to be told a stream is **prepared** before playing
it, since that is when `GtkMediaFile` would have opened the file and learned its duration, while
`GstPlay` reports nothing at all — no media info, no duration, no position — until it is playing or
paused. Each waited for the other. `GstMediaStream` now prerolls when it is given a file, which is
what `GtkMediaFile` does, and the media info that arrives at `PAUSED` is what prepares the stream.
Video never showed this, because `GtkVideo` autoplays and starts the pipeline itself; only a caller
that waits on `prepared` first can deadlock.

Note that the timeline's own video previews were never affected: `VideoPlayer` drives `GstPlay`
directly and never touches `GtkMediaFile`.

**The application icon.** The Mac builds carry artwork of their own rather than the GNOME icon the
Linux builds ship, because the shape rules differ: the GNOME icon is drawn full-bleed with a
silhouette of its own, while a Mac icon is a square plate that the system encloses.

`macos-legacy-bevel.svg` is that plate, and it is used on **every** version of macOS. Tahoe
re-shapes and lights an application icon itself, so a flat plate meant to let it do that exists as
`macos-tahoe-flat.svg` — but what Tahoe made of it did not look good enough to be worth carrying two
plates and a switch to choose between them. The flat one is kept in `assets` and referred to by
nothing.

That is worth knowing before anyone tries to make the choice automatic: **one `.icns` cannot serve
both styles anyway.** Nothing in a bundle lets the system pick an icon by OS version — that wants an
asset catalogue built with Apple's own tooling, which is not part of this environment — so a build
that wanted both would have to choose at build time regardless.

**The plate carries its own corners.** It did not at first, and the failure was invisible on the
machine that built it. Both plates were drawn full bleed with a comment saying the system masks the
enclosure — which is true of macOS 26 and of nothing before it. Every earlier release draws an
icon exactly as handed over, so the shipped `.icns` was a hard square there: the images inside it
came out colortype 2, RGB with no alpha at all, the corner pixel the same `#241f31` as the middle
of the plate.

`macos-legacy-bevel.svg` now clips itself to a rounded rectangle, full bleed, radius 230 of 1024 —
the 0.225 proportion Apple's own icon grid uses. Tahoe then masks a shape it already agrees with,
and older releases get corners they were never going to add. Two things follow from choosing full
bleed over Apple's inset template: on older macOS the icon fills its slot edge to edge rather than
sitting ~10% inside one, and it has no drop shadow. The alternative — inset and shadowed — is
correct there and comes out visibly undersized once Tahoe masks it again.

It is a circular corner where the system's is a continuous curve. At icon sizes that is not
visible, and it keeps the artwork hand-editable. To check the result, render it and look at the
alpha rather than the picture:

```sh
rsvg-convert -w 512 -h 512 assets/macos-legacy-bevel.svg -o /tmp/i.png
# colortype 6 and a transparent corner pixel; colortype 2 means the rounding was lost
```

A development build keeps the GNOME devel icon. There is no devel variant of the plate, and the
bundle name would otherwise be the only thing telling the two apart in the Dock.

**The menu bar.** Everything the main menu offers used to be reachable only from the hamburger
button in a sidebar that is already short of room, while the menu bar every other Mac application
uses sat empty. `src/macos_menu_bar.blp` fills it and `Application::startup` installs it with
`set_menubar()`; the hamburger is hidden there in exchange.

Three things about GTK's quartz backend shape that file, and none of them are obvious:

* GTK builds the **application menu** — About, Preferences, Services, Hide, Quit — itself, from
  `gtk/ui/gtkapplication-quartz.ui`, and keeps it ahead of whatever `set_menubar()` is given. It
  names `app.preferences` unconditionally and binds Command-comma to it, so that item was dead
  until `src/application.rs` grew one. It opens the account settings of the visible session, and
  is insensitive when there is no session to configure.
* **Setting a menubar replaces GTK's fallback one**, which is where Edit and Window come from.
  Ours carries both. Window is nothing but a submenu with
  `gtk-macos-special: "window-submenu"`; AppKit fills in Minimize, Zoom and the window list.
* **Only `app.`, `win.` and `gtkinternal.` resolve there.** The menu is backed by a muxer holding
  the application's actions and the active window's, and nothing else, while every item the
  hamburger offered is `klass.install_action`ed on the `SessionView` **widget**. So `Window` grew
  a `win.` forwarder per item — `FORWARDED_SESSION_ACTIONS` in `src/window.rs` — each enabled only
  while a session is actually on screen.

  The Edit items are the exception. They are copied verbatim from GTK's fallback menu, are not in
  that muxer either, and so are drawn insensitive — but their key equivalents work, because
  `GtkMacosContentView` dispatches Command-X, C, V and A to the focused widget itself. Every GTK
  application on macOS looks like this; reproducing it beats leaving Edit out.

**`matrix:` URLs.** On Linux the desktop file claims the scheme and GIO hands the URI to
`Application::open()`. On macOS `LaunchServices` reads `CFBundleURLTypes` and sends a `'GURL'`
Apple Event, which `AppKit` would forward to `-application:openURLs:` on the application
delegate — except the delegate is GTK's, and `GtkApplicationQuartzDelegate` implements only
`-applicationShouldTerminate:` and `-application:openFiles:`. The event is dropped in silence.

`src/utils/macos_url_events.rs` takes it instead, straight from the Apple Event Manager, and calls
the same `Application::open()` the Linux path ends at. It needs no new dependency: the Apple Event
Manager is a C API, so this is two `extern "C"` declarations against `CoreServices` rather than an
Objective-C class to declare. It is installed after `GtkApplication` has started up, which is where
GTK sends `-finishLaunching` and `AppKit` installs the handlers this one replaces.

Both paths work. The cold one was the doubtful half — the event is queued before the handler is
installed there — and `AppKit` delivers it afterwards regardless, so installing the handler
immediately after `GtkApplication` starts up is early enough.

**Notifications.** On Linux `Application::send_notification()` is `GNotification` and GIO finds a
backend for it. On macOS the backend it finds is `GCocoaNotificationBackend`, built on
`NSUserNotification`, deprecated in 10.14 — and on macOS 26 it delivers nothing at all. This was
measured rather than assumed: a `GApplication` in a bundle of its own sent the same notification
the real code sends, twice, and no banner ever appeared. The app ends up registered in usernoted's
database but never in `com.apple.ncprefs`, where an authorization record actually lives, so it
never appears in System Settings → Notifications to be switched on either. A GLib upgrade does not
help; 2.88's `libgio` still names `NSUserNotification`, links no `UserNotifications.framework`, and
mentions `UNUserNotificationCenter` nowhere.

`src/utils/macos_notifications.rs` talks to `UNUserNotificationCenter` instead, and unlike the
Apple Event Manager this one genuinely needs `objc2` — the API is Objective-C only, and receiving a
tap means declaring a delegate class with `define_class!`. `src/session/notifications/mod.rs`
chooses between the two behind a `cfg_if!`, so Linux is untouched.

Three things about it are worth knowing:

* **Authorization is asked for once**, at startup, from `main()` rather than from
  `Application::startup` — a tap that launched the app is delivered right after launching
  finishes, and startup is already past that. The prompt is bound to the bundle ID _and_ the
  signing identity, so an ad-hoc signature asks again on every rebuild, exactly as the Keychain
  does. Use a stable `CODESIGN_IDENTITY`.
* **The intent rides in `userInfo`** as `glib::Variant::print(true)` of the same variant the
  action already takes, read back with `glib::Variant::parse()` against the type the action
  declares. One representation, and `SessionIntent` stays the only description of what a
  notification means.
* **The avatar has to reach the disk first**, because `UNNotificationContent` takes attachments as
  file URLs. It goes to `<cache>/notification-icons/`, which `init()` empties at startup; macOS
  takes the file over when the request is added, so whatever is still there belongs to a request
  that failed.

**Text size.** GTK resolves a font's point size against `gtk-xft-dpi`, and the two platforms do not
agree on it. Linux uses 96 dots per inch by a convention old enough that nothing measures a real
screen with it. macOS uses 72, which is not a fudge: its coordinate space really is 72 units to the
inch, so a point is one logical pixel, and the backend reports that honestly.

The result is that the same stylesheet renders about a fifth smaller here. Measured with a GTK4
label on this machine:

| | macOS, as GTK reports it | Linux/GNOME |
| --- | --- | --- |
| Default label | 12 px (`.AppleSystemUIFont 12` @ 72 dpi) | 14.67 px (Cantarell 11 @ 96 dpi) |
| CSS `15pt`, a Markdown `h1` | 15 px | 20 px |

The headings are the part that makes this a bug rather than a preference. `_room_history.scss`
sizes `h1`–`h6` in `pt`, chosen to sit above body text at 96 dpi; at 72 they compress toward it,
and `h6` — 11 pt, so 11 px — comes out **smaller than the 12 px body it heads**. Raising the
resolution fixes the whole scale at once, where changing only the default font size would leave the
headings wrong.

`src/utils/macos_text_scale.rs` therefore sets `gtk-xft-dpi` to 96 dpi at startup, and only when it
finds the 72 the quartz backend reports — anything else is somebody's `settings.ini` and is left
alone. Commune's text is then larger than a typical Mac application's, which is the deliberate
trade: the alternative is a window that does not match the same application on every other
platform. If it ever wants tuning, 84 dpi (body 14 px) leans native and 88 dpi (body 14.67 px)
matches Linux body text exactly.

Note that only text moves. Padding and icon sizes are in pixels, so the layout stays where it was
and is a little tighter than on Linux, where the design is drawn for 96 dpi throughout.

**Secrets.** `src/secret/macos.rs` stores one generic password item per session, service `APP_ID`
and account = session ID. The Keychain cannot be searched on free-form attributes the way the
Secret Service can, so the session metadata is serialised into the secret next to the passphrase
rather than living in item attributes. Access and refresh tokens are unaffected: they were always
in an encrypted file under the session directory.

Restoring sessions takes **two** Keychain queries, and has to. `SecItemCopyMatching` rejects
`kSecReturnData` together with `kSecMatchLimitAll` — asking for the secrets of every matching item
at once fails with `errSecParam`, surfaced in the UI as "One or more parameters passed to a
function were not valid". So the first query lists the accounts with `load_attributes`, and the
second fetches each secret by name with `get_generic_password`.

**Data directories.** GLib follows the XDG specification everywhere, so it would have used
`~/.local/share` and `~/.cache`. `DataType::dir_path()` overrides that on macOS to
`~/Library/Application Support/commune[-Devel]` and `~/Library/Caches/commune[-Devel]`.

**Location sharing** is stubbed. `is_available()` returns false, and the message toolbar already
gates on that, so the button simply is not offered.

**The 12/24-hour clock** is read from the locale once at startup and never updates. That was
already the behaviour on any non-Linux platform; the value itself is correct, it just does not
follow a change made while the app is running.

**The `#[gtk::test]` tests do not run.** That attribute runs the body of a test on a
`GThreadPool` thread, where it initializes GTK, and on macOS GTK will only initialize on the
process main thread — which is not where any harness we use runs a test body. `cargo nextest`
gives each test its own process but still runs it on a spawned thread, and `cargo test
--test-threads=1` does run on the main thread but `#[gtk::test]` reroutes to the pool anyway. The
failure is an abort rather than a failed test, because the panic happens inside an `extern "C"`
interface init and cannot unwind, so it takes the rest of the suite with it.

Only `visual_media_row_model` was affected, because registering `GtkSectionModel` asserts that
GTK is initialized. Its tests are `#[cfg(not(target_os = "macos"))]`; the model has nothing
platform-specific in it, so the Linux runs cover it. The other `#[gtk::test]` in the tree,
`login::local_server`, never touches a GTK type and passes: the pool thread's failed
`gtk::init()` is swallowed by the `catch_unwind` inside `glib::ThreadPool::push`.

## Not done yet

* **Nothing from the Windows merge has been eyeballed here.** The merge brought spaces, peeking,
  access requests, pinned messages, presence, in-app registration and working calls, and all 200
  checks in `doc/eyeball-tests.md` were struck on Linux — none on macOS. The checklist is not
  platform-tagged, so a macOS pass is a genuinely fresh run. The rows that exercise macOS-only
  machinery deserve to go first:

  * **The startup reorder** (`033e6b04`) moved GTK's own init, the gresources, the colour scheme
    and `macos_text_scale` out of `main()` into `Application::startup`, which only the winning
    instance runs. It was verified on Windows and Arch, not here. The launch itself, a second
    launch handing off to the first, the `matrix:` URL warm and cold, a notification tap, the
    menu bar and the text size are all downstream of it.
  * **SVG through the pixbuf fallback** — see [What differs from Linux](#what-differs-from-linux);
    the tests for it cannot run on macOS, so a sticker or timeline SVG has to be looked at.
  * **A call, both directions.** The webrtc plugins this environment builds have never carried a
    real call; the RGBA capsfilter in the pipeline exists for `avfvideosrc`, which makes a Mac the
    machine that can regress it.
* **Calls do not ring on macOS**, by omission rather than decision: `ringtone.rs` resolves the
  freedesktop sound theme event `phone-incoming-call` through the XDG data directories, and a Mac
  has no such theme, so `sound_file()` finds nothing and the call is silent until the notification
  is noticed. Bundling an `.oga` under `Contents/Resources/share/sounds/` would satisfy the
  existing lookup without a platform branch.
* **A call notification carries no Answer and Decline buttons on macOS.** The new
  `send_notification_with_buttons` drops the buttons on the macOS arm —
  `UNNotificationAction` is the missing wiring — so only the banner click works, which shows the
  call rather than answering it.
* **The local test homeserver has not run on macOS.** `testing/local-homeserver.sh` uses GNU
  `sed -i` (BSD sed wants `sed -i ''`), and it gives coturn `--network=host`, which inside
  podman's Linux VM is the VM's network, not the Mac's — the split-horizon failure the script
  itself warns about. Synapse's own container uses `-p` and should be fine. The eyeball run
  depends on this script, so it is first in line. `hooks/doc-freshness` has the same GNU habits
  (`sed -i`, `sha256sum`, bash-4 `mapfile`) — its `--staged` pre-commit path survives on macOS,
  but `--fix` and `--published` do not.
* **M3 is mostly proven.** The menu bar, the File and View items, the hidden hamburger, the
  `matrix:` scheme warm and cold, session restore, the Keychain, video and audio have all been
  seen working. What is left on the [Testing by hand](#testing-by-hand) list is the Command keys,
  Preferences, and the Edit menu.

  Left out of M3 deliberately: a File → Close Window item, which the muxer cannot reach, so it
  would be drawn insensitive next to a ⌘W that works. The media viewer's own close button was on
  this list too and has come off it — looked at on a Mac, it reads as native as it stands.
* **M4** — camera QR scanning through `avfvideosrc`. None of it exists.
* **The sticker picker sometimes will not close on a click outside it.** Reported from a bundle,
  intermittent, and not reproducible on demand — Escape closes it, and so does changing room, but
  a click outside it sometimes does nothing. Nothing has been changed, because a fix that cannot
  be triggered cannot be tested, and a wrong one closes the picker while somebody is using it.

  What is known. A `GtkPopover` dismisses by taking a **grab**: GTK's own documentation says it
  performs one "so the popover is dismissed in the expected situations (clicks outside the popover,
  or the Escape key being pressed)". Escape working while the click does not is a grab that still
  routes the keyboard and is not receiving the pointer.

  Why that would be macOS-only and intermittent: the picker is 400×420 on a `MenuButton` that opens
  `up` from the bottom of the window, so it often does not fit inside the window, and GDK promotes
  a popover that does not fit into a separate popup surface — which on macOS is another `NSWindow`.
  A click back onto the main window is then a window-activation click, which macOS may consume
  rather than deliver. Whether it overflows at all depends on the window size and where the toolbar
  sits, which would explain why it comes and goes. The `GtkSearchEntry` on the GIF tab holding
  keyboard focus fits the same picture.

  The candidate fix, when there is something to test it against: a focus controller on the picker
  that calls `popdown()` when focus leaves its subtree, belt-and-braces beside the grab.

Still unverified: GTK's macOS backend for input methods. Drag and drop has now been exercised —
and found broken upstream, then repaired; see
[What differs from Linux](#what-differs-from-linux). A drop and a Finder-copy paste both want a
fresh eyeball with the repair in place.

Two cosmetic things a run turns up that are nobody's bug in particular. GTK's macOS backend
reports the system font as `.AppleSystemUIFont`, and libadwaita's stylesheet feeds that
leading-dot name to the CSS parser unquoted, so every window logs a run of
`Theme parser error: gtk.css:1:1-2: Junk at end of font-family value`. It is genuinely only noise:
the properties it fails to expand are libadwaita's own `--document-font-family` and
`--document-font-size`, and nothing in `data/resources/stylesheet/` refers to either, so no text in
Commune is styled by them. It is not the reason text is small on macOS — see
[Text size](#what-differs-from-linux) for that. And image packs whose
images are in a format the `image` crate does not decode report "Image format not supported" per
image; see the format list above.

## Rebasing

Upstream Fractal has no macOS support and will not grow any, so every one of these is a conflict
waiting to happen on a rebase. Re-apply, in rough order of how easily they are lost:

* `src/utils/media/image/mod.rs` and `src/components/media/animated_image_paintable.rs` refer to
  `decoder::` where upstream refers to `glycin::`. Any upstream change to image loading will
  conflict. The `GlycinFrameExt` trait was deleted; its three methods are now inherent methods on
  `decoder::Frame`, and `has_delay()`/`delay_duration()` collapsed into `delay() -> Option`.
* `Cargo.toml`: `glycin` and `glycin-gtk4` moved into the Linux target table, and `image` is in a
  `cfg(not(target_os = "linux"))` table. `cargo-sort --grouped` fixes the ordering if it drifts.
* `meson.build` and `data/meson.build` gate the glycin dependencies, the desktop file, the D-Bus
  service and `gnome.post_install`'s `update_desktop_database` on
  `host_machine.system() != 'darwin'`.
* `.cargo/config.toml`, for the reason given under [Building](#building).
* `src/config.rs.in` has `#[cfg(target_os = "linux")]` on `DISABLE_GLYCIN_SANDBOX`.
* `src/main.rs` loads its gresources and binds its text domain from `app_bundle::init()` rather
  than from the `config` constants. Any upstream change to startup will conflict.
* `Application::run()` takes the `RuntimePaths` and logs the directory the app **actually** loaded
  its data from. Upstream logs `config::PKGDATADIR`, which inside a bundle is a path on the machine
  that built it and need not exist at all on the machine running it. That was the constant's only
  use, so `src/config.rs.in` no longer defines `PKGDATADIR`; `RESOURCES_FILE` and
  `UI_RESOURCES_FILE` still interpolate Meson's `@PKGDATADIR@` and are unchanged.
* `src/macos_menu_bar.blp` and `src/utils/macos_url_events.rs` are ours alone, but the seams they
  are wired into are not: `Application::startup` installs both, `Application` gained an
  `app.preferences` action, `Window` gained the `FORWARDED_SESSION_ACTIONS` table, and `Sidebar`
  hides its `appmenu_button`.
* `src/components/media/content_viewer.rs` calls `set_video_file()` and `clear_video()` instead
  of `GtkVideo::set_file()` directly, for the media backend reason above.
* `src/session_view/room_details/history_viewer/file_row.rs` opens files with
  `gtk::FileLauncher`. `gio::AppInfo` has no backend outside of Linux. This one is not
  macOS-specific and is worth sending upstream.

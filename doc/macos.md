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

M0, M1 and M2 are done, and M3 is written but not yet exercised by hand — see
[Testing by hand](#testing-by-hand) for the list that has to be worked through, and treat the menu
bar, the `matrix:` URL handler and notifications as unproven until it has been. The tree builds for
`aarch64-apple-darwin`, and `cargo check`, `cargo
clippy --all-targets -- -D warnings`, `cargo +nightly fmt --check`, `cargo deny`, `cargo machete`,
`cargo sort`, `typos`, `rumdl`, `cargo nextest run` and `meson test` all pass. `meson compile` and
`meson install` work, and **the app runs**: logging in with a password, syncing, the timeline,
image thumbnails, animated GIFs, the GIF search, video in the media viewer and restoring the
session from the Keychain after a quit were all exercised on macOS 26.

There is now a **relocatable `Commune.app`**, and a `.dmg` and a `.tar.gz` around it. It launches
from a shell with nothing exported, and loads no library from outside itself.

The environment it all needs is created by a script in `build-aux/macos/`.

| Area | State |
| --- | --- |
| Runtime paths | `src/utils/app_bundle.rs`, relative to the executable inside a bundle |
| Image decoding | Rewritten behind `src/utils/media/image/decoder/`, `image` crate on macOS |
| Video in the media viewer | Own `GtkMediaStream`, `src/components/media/gst_media_stream.rs` |
| Secrets | macOS Keychain, `src/secret/macos.rs` |
| Data directories | `~/Library/Application Support` and `~/Library/Caches` |
| `.app` bundle, `.dmg`, `.tar.gz` | `build-aux/macos/bundle.sh` and its two wrappers |
| Location sharing | Stubbed, `is_available()` is false and the UI hides it |
| System 12/24h clock | Locale-derived at startup, never updates live |
| Camera QR scanning | Stubbed, returns no cameras |
| Menu bar | `src/macos_menu_bar.blp`, with `win.` forwarders on `Window` |
| Keyboard shortcuts | `<Primary>` throughout, so Command rather than Control |
| `matrix:` URLs | Our own Apple Event handler, `src/utils/macos_url_events.rs` |
| Notifications | GLib's Cocoa backend, unchanged and unproven |

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
| 1 | Menu bar | Look at it | Commune, File, Edit, View, Window, Help |
| 2 | Menu bar | Commune → About Commune | The About dialog, named Commune, not `commune` |
| 3 | Menu bar | Commune → Preferences, and ⌘, | The account settings of the visible session |
| 4 | Menu bar | Preferences while logged out | Greyed out |
| 5 | Menu bar | Commune → Hide, Hide Others, Show All, Quit | The usual macOS behaviour |
| 6 | Menu bar | File → each of the five items | The same dialogs the old hamburger menu opened |
| 7 | Menu bar | File and View on the login page | Greyed out; sensitive again once a session is up |
| 8 | Menu bar | Edit → Cut, Copy, Paste, Select All | Greyed, but showing ⌘X ⌘C ⌘V ⌘A |
| 9 | Menu bar | ⌘X, ⌘C, ⌘V, ⌘A in the composer | They work, greyed menu items notwithstanding |
| 10 | Menu bar | View → the five room items, Full Screen | Selection moves; the window goes full screen |
| 11 | Menu bar | Window | Minimize, Zoom and the window list, from AppKit |
| 12 | Menu bar | Help → Keyboard Shortcuts | The shortcuts dialog |
| 13 | Sidebar | Look at the header bar | No hamburger button |
| 14 | Shortcuts | ⌘Q, ⌘W | Quit; close window |
| 15 | Shortcuts | ⌘K, ⌘L, ⌘, | Room search; join room; account settings |
| 16 | Shortcuts | ⌘Page Up, ⌘Page Down, and both with ⇧ | Previous/next room, then the unread ones |
| 17 | Shortcuts | ⌘⇧8 (⌘\*) | Jumps to the first room with unread messages |
| 18 | Shortcuts | ⌘V into the composer | Pastes, including an image |
| 19 | Shortcuts | Help → Keyboard Shortcuts, read the list | ⌘ glyphs throughout, no ⌃ |
| 20 | `matrix:` URL | Running: `open -a <bundle> 'matrix:r/<room>:<server>'` | Comes forward and opens the room — **verified** |
| 21 | `matrix:` URL | Quit first, then the same command | It launches and lands in the room |
| 22 | `matrix:` URL | Bare `open 'matrix:…'`, no `-a` | Goes to whichever app owns the scheme; see below |
| 23 | `matrix:` URL | Click a `matrix:` link in another app | Same as 20 |
| 24 | Notifications | Background the app, have somebody send a message | A notification appears |
| 25 | Notifications | Click it | The right session and the right room open |
| 26 | Notifications | Same for an identity verification request | The verification opens |
| 27 | Keychain | Log out of a session | Its item is gone from Keychain Access, under the application ID |
| 28 | Keychain | Then look in `~/Library/Application Support/commune-Devel/` | The session's directory is gone |
| 29 | Regression | Log in with a password, and with SSO | Both work; SSO may raise a firewall prompt |
| 30 | Regression | Send and receive text; open a room's history | Nothing unusual |
| 31 | Regression | An image thumbnail, an animated GIF, the GIF search | All render, animation included |
| 32 | Regression | A video in the media viewer | Plays, with working controls |
| 33 | Regression | Quit and relaunch | The session comes back without a login |
| 34 | Regression | Launch the bundle from a shell with nothing exported | It runs |

Row 20 has been run: the Apple Event arrives, `Application::open()` is reached, and the URI is
parsed into the right intent — the whole path, stopping only at "Cannot process intent with no
logged in session", which is what a build with no session should say. Row 21 is the one that is
still open, because a cold launch queues the event before the handler exists and it is `AppKit`
that decides when to deliver it.

**Pass `-a` and the bundle.** More than one application on a developer's machine claims the
`matrix:` scheme — `lsregister -dump | grep matrix:` will list them — so a bare `open 'matrix:…'`
proves nothing about Commune unless Commune happens to have won the scheme. Naming the bundle takes
LaunchServices' choice out of it.

Rows 24 to 26 are the ones most likely to fail. GLib's Cocoa notification backend is built on
`NSUserNotification`, which Apple deprecated in 10.14, and whether it still does anything at all on
a current macOS is exactly what has not been tried. If nothing appears, that is the first thing to
suspect rather than anything in `src/session/notifications/`.

Rows 1 to 19 could not be run here either: reading the menu bar needs Accessibility or Screen
Recording permission, which is the user's to grant. All that is known is that the menu model loads
and `set_menubar()` is reached without complaint.

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

**Formats not supported on macOS: SVG, HEIC, AVIF and JXL.** They surface in the UI as "Image
format not supported". BMP, GIF, ICO, JPEG, PNG, APNG, TIFF and WebP all work, animated where the
format allows. An ImageIO-backed decoder would close the gap and is the obvious later option.

**Video in the media viewer.** `GtkVideo` plays a file with `GtkMediaFile`, and `GtkMediaFile` has
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

Note that the timeline's own previews were never affected: `VideoPlayer` drives `GstPlay`
directly and never touches `GtkMediaFile`.

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

This one has been exercised against a running bundle and works. What has not been exercised is a
**cold** launch, where the event is queued before the handler is installed and it is `AppKit` that
decides when to deliver it.

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

* **M3 is written but unproven.** Nothing in it has been through the
  [Testing by hand](#testing-by-hand) list, which is where it has to go before any of it can be
  called done. Notifications are the part most likely to be broken and the only part with no code
  of ours behind it: GLib's Cocoa backend is built on `NSUserNotification`, deprecated since
  10.14.

  Left out of M3 deliberately: the media viewer's own close button, which sits where macOS puts
  the traffic lights and wants rethinking rather than moving; and a File → Close Window item,
  which the muxer cannot reach, so it would be drawn insensitive next to a ⌘W that works.
* **M4** — camera QR scanning through `avfvideosrc`. None of it exists.

Still unverified: GTK's macOS backend for input methods and drag and drop.

Two cosmetic things a run turns up that are nobody's bug in particular. GTK's macOS backend
reports the system font as `.AppleSystemUIFont`, and libadwaita's stylesheet feeds that
leading-dot name to the CSS parser unquoted, so every window logs a run of
`Theme parser error: gtk.css:1:1-2: Junk at end of font-family value`. And image packs whose
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

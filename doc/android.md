# Commune on Android

This is the ledger for the Android port: what exists, what was measured, and what bit us on the
way. `doc/android-plan.md` is the route that was intended; this file is what actually happened.

The port is **exploratory**. Nothing of Commune runs on Android yet. S0 established that the
toolchain works and that GTK is in better shape on Android than expected; S1 established that a
GTK application written in Rust can be an Android package at all, which was the open question the
whole route hung on.

## Contents

<!-- toc -->
* [State today](#state-today)
* [The build host](#the-build-host)
* [Setting the environment up](#setting-the-environment-up)
* [Building a demo APK](#building-a-demo-apk)
* [What S0 measured](#what-s0-measured)
* [Findings that change the plan](#findings-that-change-the-plan)
* [S1 — Rust on Android works](#s1--rust-on-android-works)
* [Known gaps](#known-gaps)
<!-- /toc -->

## State today

| Spike | State |
| --- | --- |
| S0 — pixiewood baseline | **done**, gtk4-demo runs on the emulator; the libadwaita demo does **not** build on this host (see below) |
| S1 — Rust hello-world APK | **done** — a Rust GTK app runs as an APK |
| S2 — Commune `cargo check` for Android | next |
| S3 — Commune login on the emulator | not started |
| S4 — GStreamer | not started |
| S5 — keystore, notifications, SSO, push | not started |

## The build host

pixiewood, meson and the NDK are Linux tools, so the build host is **WSL Ubuntu-24.04**, while the
emulator and `adb` stay on the Windows side. The APK crosses over `/mnt/c`.

The Windows Android SDK at `C:\Android\Sdk` (the one seed-sync uses) **cannot be shared with WSL**.
Its NDK carries only `toolchains/llvm/prebuilt/windows-x86_64` and its build tools are `.exe`, so
WSL needs its own SDK. That is 2.4 GB at `~/android/sdk` and the duplication is unavoidable. The
two SDKs deliberately hold the same NDK version, `27.2.12479018` — the one GTK's own CI pins in
`.gitlab-ci/android-sdk.sh`.

Ubuntu 24.04's meson is 1.3.2, which is too old: `android_exe_type` landed in **meson 1.9**. It is
installed with pipx instead. Stock meson is enough — pixiewood's README still offers
`--meson <forked/meson/meson.py>`, but that predates the kwarg being upstream.

## Setting the environment up

Run `sh build-aux/android/probe-env.sh` in WSL first; it prints what is missing. The full setup:

```sh
sudo apt-get install -y \
  libglib-perl libglib-object-introspection-perl libipc-run-perl libjson-perl \
  libset-scalar-perl libxml-libxml-perl libxml-libxslt-perl gir1.2-appstream-1.0 \
  openjdk-17-jdk-headless build-essential glslc gobject-introspection \
  libglib2.0-dev-bin libxml2-utils ninja-build sassc gettext python3-pip pipx git unzip curl
pipx install meson           # >= 1.9; Ubuntu's 1.3.2 is too old

# Linux SDK, separate from the Windows one
SDK=$HOME/android/sdk
curl -fsSL -o /tmp/clt.zip \
  https://dl.google.com/android/repository/commandlinetools-linux-13114758_latest.zip
unzip -q /tmp/clt.zip -d /tmp/clt && mkdir -p $SDK/cmdline-tools
mv /tmp/clt/cmdline-tools $SDK/cmdline-tools/latest
yes | $SDK/cmdline-tools/latest/bin/sdkmanager --sdk_root=$SDK --licenses
$SDK/cmdline-tools/latest/bin/sdkmanager --sdk_root=$SDK \
  "platforms;android-35" "build-tools;35.0.0" "ndk;27.2.12479018"

git clone https://github.com/sp1ritCS/gtk-android-builder.git ~/src/gtk-android-builder
make -C ~/src/gtk-android-builder prefix=$HOME/.local install
```

### Svg2Avd wants Android Studio, and does not get it

`pixiewood generate` converts the app icon with a bundled Java tool, `Svg2Avd`, whose classpath is
assembled from an **Android Studio installation** (`-a/--android-studio-dir`, jars under
`lib/` and `plugins/android/lib/`). Without it, `generate` dies with

```text
java.lang.NoClassDefFoundError: com/android/ide/common/vectordrawable/Svg2Vector
```

and — worse — carries on to produce a truncated `ic_launcher_foreground.xml` that fails the Gradle
build much later with `Content is not allowed in prolog`.

Installing a GUI IDE in WSL to convert one SVG is not worth it. Java ignores classpath entries that
do not exist, so a directory holding only the jars that matter is enough:

```sh
FS=$HOME/android/fake-studio
mkdir -p $FS/lib $FS/plugins/android/lib
G=https://dl.google.com/dl/android/maven2/com/android/tools; V=31.7.0
curl -fsSLo $FS/plugins/android/lib/sdk-common.jar          $G/sdk-common/$V/sdk-common-$V.jar
curl -fsSLo $FS/plugins/android/lib/android-base-common.jar $G/common/$V/common-$V.jar
curl -fsSLo $FS/lib/util-8.jar                              $G/annotations/$V/annotations-$V.jar
curl -fsSLo $FS/lib/intellij.libraries.guava.jar \
  https://repo1.maven.org/maven2/com/google/guava/guava/33.3.1-jre/guava-33.3.1-jre.jar
```

Then pass `-a $HOME/android/fake-studio` to `pixiewood prepare`. About 5 MB instead of several GB.

## Building a demo APK

```sh
git clone --depth 1 https://gitlab.gnome.org/GNOME/gtk.git ~/src/gtk
cd ~/src/gtk
# GTK ships its own pixiewood manifest; add an architectures whitelist to it so
# only the emulator's x86_64 is built.
PW="perl ~/src/gtk-android-builder/pixiewood -C ~/src/gtk"
$PW prepare --sdk $HOME/android/sdk --toolchain $HOME/android/sdk/ndk/27.2.12479018 \
    --meson $HOME/.local/bin/meson -a $HOME/android/fake-studio \
    build-aux/android/org.gtk.Demo4.xml
$PW generate
$PW build
```

The APK lands in `.pixiewood/android/app/build/outputs/apk/debug/app-x86_64-debug.apk`. Install it
from the Windows side, where the emulator lives:

```pwsh
wsl -d Ubuntu-24.04 -- cp <apk> /mnt/c/Users/<user>/AppData/Local/Temp/demo.apk
C:\Android\Sdk\platform-tools\adb.exe install -r $env:TEMP\demo.apk
```

Timings on this machine, from a cold clone: `prepare` (meson configure of GTK plus 19 subprojects)
about two minutes; `build` **80 seconds for 3113 ninja targets**, then Gradle 14 seconds. Cross-
compiling the whole GTK stack is not the slow part of this port.

The debug APK is **136 MB** — unstripped, and carrying both demos GTK builds. Gradle says as much:
`Unable to strip the following libraries, packaging them as they are:` and then all 32 `.so`s.
A release build with stripping is the number that matters, and has not been measured.

### `prepare` downloads, and hangs quietly when a download fails

Not every subproject is a git wrap; some are tarballs fetched during `meson setup`. One of them,
`pixman`, comes from `cairographics.org`, which timed out repeatedly from WSL on the second build
of the day. meson does not fail on this — it retries "after a delay", forever, so `prepare` looks
like a slow configure rather than a stall. Ten minutes of nothing is the symptom;
`.pixiewood/bin-x86_64/meson-logs/meson-log.txt` is where it says so.

Two things follow. Watch that log rather than the process list when `prepare` seems slow. And
because every project gets its own `subprojects/packagecache`, a tarball already fetched for one
build can be copied into the next to skip the download entirely:

```sh
cp -n ~/src/gtk/subprojects/packagecache/*.tar.* ~/src/gtk/subprojects/packagecache/*.zip \
      ~/src/<project>/subprojects/packagecache/
```

When builds are eventually made repeatable, one shared cache directory is worth arranging.

### The host's GLib tools are too old for the GLib being built

Cross-compiling needs two GLibs: the target one, built from the `main` wrap, and the *build
machine's* code generators — `glib-mkenums`, `glib-genmarshal`, `glib-compile-resources` — which
meson takes from the host. On Ubuntu 24.04 those are GLib **2.80**, while the wrap builds 2.89.
That gap is not academic: it stopped a build dead.

libadwaita `main` failed to compile with

```text
../../src/adw-tab-view.c:2617:25: error: use of undeclared identifier 'ADW_TYPE_TAB_VIEW_SHORTCUTS'
```

which reads like an Android problem and is not one. `adw-tab-view.h` is the only libadwaita header
that writes `} G_GNUC_FLAG_ENUM AdwTabViewShortcuts;`, and GLib 2.80's `glib-mkenums` does not know
that macro, so it silently skipped the type: the generated `adw-enums.h` held 25 `ADW_TYPE_`
entries and not one of the `TAB_VIEW` ones. A missing symbol at the end of a long build, with the
real fault three steps upstream and no warning anywhere.

The obvious fixes do not work, and it is worth knowing why before reaching for them. `glib-mkenums`
is an architecture-independent Python script, and the newer GLib is already checked out as a
subproject, so two shims suggest themselves — and both fail:

* Generating 2.89's `glib-mkenums` into `~/bin` and putting it first on `PATH`.
* Copying the host `glib-2.0.pc` with `bindir` repointed at that directory, on `PKG_CONFIG_PATH`
  (`pkg-config --variable=glib_mkenums glib-2.0` then correctly answers `~/bin/glib-mkenums`).

Neither changes anything, because meson's `gnome.mkenums_simple()` resolves the tool once and bakes
an **absolute path** into `build.ninja`:

```text
build src/adw-enums.h: CUSTOM_COMMAND ... ../../src/adw-tab-view.h | /usr/bin/glib-mkenums
```

That is with a build directory deleted and reconfigured from scratch, with both shims in place. The
lookup meson uses for native GObject tooling does not consult `PATH`, and did not take the
`PKG_CONFIG_PATH` override either. A meson `--native-file` carrying a `[binaries]` entry would do
it, but pixiewood only ever passes `--cross-file`, and cross-file binaries describe the host
machine, not the build machine.

So the real conclusion is about the build host, not about a shim: **Ubuntu 24.04 is too old to
build GTK and libadwaita `main`.** The options are a newer distribution in WSL (25.10, Fedora,
Arch — GTK's own CI uses a rolling image), a container, or a GLib built and installed natively so
that `/usr/bin` genuinely carries the newer tools.

The sequencing this implies matters. GTK `main` itself builds fine here — nothing in it uses
`G_GNUC_FLAG_ENUM` — so **S1 is not blocked**: the Rust spike needs GTK and a C stub, not
libadwaita. But **Commune is a libadwaita application**, so this must be solved before S3, and the
shims above are not the way. Treat "move the build host to a newer distribution" as a prerequisite
of S3 rather than as a problem to be worked around.

Worth knowing before S3: Commune's own build runs `glib-compile-resources` and
`glib-compile-schemas` from the host too. If any of them turn out to be too old, this is the shape
of the fix — take the tool from the subproject, not from the distribution.

## What S0 measured

gtk4-demo from GTK `main`, on the `seed_api35` AVD (API 35, x86_64), 23 August 2026:

| Test | Result |
| --- | --- |
| Launch | ok, `org.gtk.demo4/org.gtk.android.ToplevelActivity`, no crash |
| Rendering | correct — GTK styling, fonts, spacing all as on the desktop |
| Soft keyboard → GTK text entry | **works**: typing `fish` into the demo's search filtered the list to Fishbowl |
| Popover (primary menu) | correct, arrow and placement right |
| Rotation to landscape | relaid out, no crash, no renderer complaints |
| HOME then relaunch | **blank window** with the stock manifest; fixed by `launchMode` (below) |
| Process survival | the process stayed alive across backgrounding |

Screenshots were taken for each and are not committed; regenerate with
`adb exec-out screencap -p`.

## Findings that change the plan

**1. Meson needs no fork.** `android_exe_type: 'application'` is in stock meson 1.12. An
`executable()` built that way produces `lib<target>.so`, which GTK's Java glue `dlopen`s and whose
`main` it calls. Commune's `meson.build` will need the same guard libadwaita already uses:

```meson
if meson.version().version_compare('>= 1.8.0') and host_machine.system() == 'android'
  demo_kwargs += { 'android_exe_type': 'application' }
endif
```

**2. libadwaita is already Android-aware upstream.** `demo/meson.build` sets `android_exe_type`
behind exactly that guard, and `src/meson.build:373` builds `adw-settings-impl-android.c` against a
`gtk4-android` dependency — so the system colour scheme has a real Android backend. This is the
strongest signal yet for Route A: the toolkit Commune's UI is written against is being maintained
for this platform, not merely tolerated.

**3. `launchMode="standard"` breaks the warm path.** pixiewood hardcodes it in
`generate/manifest.xsl:32`. Because GTK has one toplevel and Android stacks a new Activity on every
launch, returning to the app from the launcher shows an **empty white window** while the old,
working Activity sits behind it (`numActivities=2`, `LAUNCH_MULTIPLE` in logcat). Rebuilding with
`android:launchMode="singleTask"` fixes it: HOME then relaunch resumes the real UI.

This is the Android form of the single-instance question the Windows port had to answer for
`matrix:` links, and it has the same answer shape: one line, but it must be applied. pixiewood
regenerates `AndroidManifest.xml` on every `generate`, so the fix has to be a patch to
`manifest.xsl`, a post-`generate` step in our own build script, or an upstream change — not a hand
edit.

**4. The `INTERNET` permission comes from the appstream metainfo.** `manifest.xsl:59` emits
`INTERNET` and `ACCESS_NETWORK_STATE` only when the metainfo declares
`<requires><internet>always</internet></requires>`. Commune's
`data/io.github.steeb_k.Commune.metainfo.xml.in.in:60-62` already declares exactly that, and also
`<control>touch</control>` and `<display_length compare="ge">360</display_length>`. Nothing to do
— but it would have been a baffling failure to debug, since a Matrix client with no network
permission looks like a broken homeserver.

**5. Nothing in pixiewood knows about Rust.** The one wrap that looks like prior art, `rsvg`, is
**librsvg 2.40.22** — the last pre-Rust release, carrying a hand-written meson build. So pixiewood
sidesteps Rust rather than solving it, and S1 remains genuinely unexplored territory.

**6. The pkg-config half of the Rust problem looks solvable.** gtk4-rs issue #1997 says the
blocker is that Android wants one ninja pass over app and dependencies, which pkg-config-driven
`-sys` crates cannot join. But `meson setup` writes **48 `*-uninstalled.pc` files** into
`.pixiewood/bin-x86_64/meson-uninstalled/`, `gtk4-uninstalled.pc` (4.23.3) among them, with `-L`
and `-I` pointing into the build tree. Their timestamps put them at 19:39:04, with `build.ninja`
at 19:39:10 and the first real compile at 19:39:56 — that is, **the whole pkg-config view of the
dependency tree exists before ninja runs a single command.**

So a `custom_target` that runs `cargo build --target x86_64-linux-android` during the ninja phase
can be handed `PKG_CONFIG_PATH=<builddir>/meson-uninstalled` (with `PKG_CONFIG_LIBDIR` cleared so
the host's own `.pc` files cannot leak in, and `PKG_CONFIG_ALLOW_CROSS=1`) and see exactly the GTK
the APK will ship.

This is encouraging, not settled. Two things are still unproven: the target must be ordered after
the `libgtk-4.so` link steps, since the `.pc` files name libraries ninja has not built yet
(`depends:` in meson expresses that); and `system-deps`, which gtk4-sys uses, does its own
cross-compilation and version handling that has not been exercised here. S1 should start from this
rather than from the issue's pessimism.

**7. Every wrap tracks a moving target.** `glib`, `gtk`, `libadwaita`, `cairo`, `harfbuzz` and
`fontconfig` are all `revision = main` (or `master`) at `depth = 1`. Two builds a week apart are
not the same build. When anything is packaged for a hand-out, the revisions have to be pinned and
recorded here.

## S1 — Rust on Android works

**The make-or-break spike passed.** A GTK4 application written in Rust runs as an Android APK,
with Rust closures handling input, mutating Rust state and driving GTK widgets. gtk4-rs itself
needed no patches: `glib 0.22.8`, `gio`, `cairo-rs`, `graphene-rs`, `gdk-pixbuf`, `pango`,
`gdk4 0.11.4`, `gsk4` and `gtk4 0.11.4` all cross-compiled for `x86_64-linux-android` unmodified.

The spike lives outside this repository, at `~/src/gtk-android-rust-spike` in WSL, because nothing
should be designed into Commune before it is known to link. What follows is the part worth keeping.

### What was measured

| Test | Result |
| --- | --- |
| gtk4-rs cross-compiles for `x86_64-linux-android` | yes, unpatched, against the Android GTK 4.23.3 |
| Rust `staticlib` links into the APK's shared object | yes — `librust-spike.so`, 14.9 MB |
| App launches | yes; logcat shows `rust_spike_main entered`, then `window presented` |
| Rust closure runs on tap | yes; three taps logged `Rust answered 1/2/3 time(s)`, label updated to match |
| Rotation | no crash, same pid, and the counter went 3 → **4** rather than resetting |
| Debug APK size | 141 MB (unstripped, as with gtk4-demo) |

That the counter survived rotation matters more than it looks: the Activity is recreated, but the
GTK thread and the Rust state it owns live in the process, which outlives it.

### How the Rust is wired in

Issue [gtk4-rs#1997][] frames the problem as two things — Rust does not export `main`, and
pkg-config-driven `-sys` crates cannot join Android's single ninja pass. Both turned out to be
straightforwardly solvable.

**The entry point** is a three-line C stub, because GTK's glue `g_module_symbol`s for `main`:

```c
int rust_spike_main (void);

int
main (int argc, char **argv, char **envp)
{
  (void) argc; (void) argv; (void) envp;
  return rust_spike_main ();
}
```

**The library is a `staticlib`**, and that is the whole trick. rustc never links a staticlib, so
cargo does not need `libgtk-4.so` to exist — it needs only the pkg-config _description_ of it,
which meson writes at configure time. No dependency edge is required between the cargo target and
GTK's own link steps, and meson performs the one real link:

```meson
build_rust = find_program('build-rust.sh')

rust_static = custom_target('rustspike-static',
  output: 'librustspike.a',
  command: [build_rust, meson.project_source_root(), meson.project_build_root(), '@OUTPUT@'],
  build_by_default: true,
  console: true,
)

# `sources:` is what orders the cargo run before the executable link.
rust_dep = declare_dependency(
  link_args: [rust_static.full_path(), '-llog', '-ldl', '-lm'],
  sources: [rust_static],
)

exe_kwargs = {}
if meson.version().version_compare('>= 1.8.0') and host_machine.system() == 'android'
  exe_kwargs += {'android_exe_type': 'application'}
endif

executable('rust-spike', 'stub.c',
  dependencies: [gtk_dep, rust_dep],
  install: true,
  kwargs: exe_kwargs,
)
```

**The pkg-config view** is the other half. `PKG_CONFIG_LIBDIR` _replaces_ the search path rather
than extending it, so pointing it at meson's `meson-uninstalled` directory means an Android build
physically cannot pick up a `.pc` from `/usr`:

```sh
PKG_CONFIG_PATH="$BUILD/meson-uninstalled"
PKG_CONFIG_LIBDIR="$BUILD/meson-uninstalled"
export PKG_CONFIG_PATH PKG_CONFIG_LIBDIR
export PKG_CONFIG_ALLOW_CROSS=1

TOOL=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin
export CC_x86_64_linux_android="$TOOL/x86_64-linux-android31-clang"
export AR_x86_64_linux_android="$TOOL/llvm-ar"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$TOOL/x86_64-linux-android31-clang"

cargo build --release --target x86_64-linux-android --target-dir "$BUILD/cargo-target"
```

pkg-config resolves `gtk4` from `gtk4-uninstalled.pc` by its own uninstalled-package convention —
no renaming needed — and the whole `Requires` chain follows: glib 2.89.4, pango 1.58.2,
cairo 1.18.5, gdk-pixbuf 2.43.3, graphene 1.11.1.

One trap worth naming: meson passes `@OUTPUT@` **relative to the build directory**, which is
ninja's working directory. A script that `cd`s into the source tree before copying its artefact
will silently write it to the wrong place and the build will fail later with a missing file.
Resolve the output path before changing directory.

**Logging.** `println!` goes nowhere on Android. The spike declares
`__android_log_write` and links `-llog`, which puts Rust output in logcat under its own tag
(`adb logcat -s RustSpike`). GTK's own `g_log`/`g_print` are already redirected to logcat by
`gdkandroidruntime.c`, so Commune's `tracing` subscriber has two plausible sinks to choose from.

### The appstream namespace, which will bite Commune

`pixiewood generate` finds the application id with the XPath
`/pw:app/pw:metainfo/meta:component/meta:id`, where `meta` is bound to
`https://specifications.freedesktop.org/metainfo/1.0`. A metainfo file **without that namespace
declared on `<component>` does not match**, and the error names XInclude rather than the
namespace:

```text
Unable to find component id in manifest, if you are using XInclude ensure that the path is correct
```

GTK's own `org.gtk.Demo4.appdata.xml.in` declares
`xmlns="https://specifications.freedesktop.org/metainfo/1.0"`. **Commune's does not** —
`data/io.github.steeb_k.Commune.metainfo.xml.in.in` opens with a bare
`<component type="desktop-application">`, which is the ordinary way to write appstream metadata and
is accepted everywhere else. S3 will have to add the namespace, or teach the manifest to point at a
copy that has it. Adding it upstream-style is harmless to the Linux build and is the cheaper of the
two.

[gtk4-rs#1997]: https://github.com/gtk-rs/gtk4-rs/issues/1997

## Known gaps

* **The soft keyboard did not hide itself.** Once shown it stayed, through `ESC`, through the
  search bar being dismissed, and through rotation — where it covers most of a landscape screen.
  Only the system's own dismiss chevron or BACK put it away.

  This is _not_ a missing mechanism, which was the first guess and was wrong. The path is complete
  end to end: `gtk_im_context_android_focus_out` (`gtk/gtkimcontextandroid.c:469`) →
  `gtk_im_context_android_update_ime_keyboard` (`:357`) → the cached JNI method
  `setImeKeyboardState` → `ToplevelActivity.java:154`, which calls
  `WindowInsetsController.hide(WindowInsets.Type.ime())`. So either gtk4-demo never dropped
  keyboard focus, or the `hide` was overridden by the `requestFocus()` the show path performs on
  the same view.

  Which of those it is was not chased down: gtk4-demo's search bar is a poor proxy for a chat
  composer. **Retest in S3 with a real `AdwEntryRow`/`GtkTextView`** before drawing any conclusion.
  Record the answer here — for a chat app this is the single most load-bearing input behaviour.
* The IME comes up unbidden on launch.
* 136 MB debug APK, unmeasured stripped size.
* No Rust story (S1).
* GStreamer is not built at all: pixiewood's cross file sets `media-gstreamer = 'disabled'` for
  GTK, so voice messages, video and calls are all out of reach until S4.

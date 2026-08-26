# Commune on Android

This is the ledger for the Android port: what exists, what was measured, and what bit us on the
way. `doc/android-plan.md` is the route that was intended; this file is what actually happened.

The port is **exploratory**, and as of S3 **Commune runs on Android**. S0 established that the
toolchain works and that GTK is in better shape on Android than expected; S1 established that a
GTK application written in Rust can be an Android package at all, which was the open question the
whole route hung on.

## Contents

<!-- toc -->
* [State today](#state-today)
* [Where things are](#where-things-are)
* [The build host](#the-build-host)
* [Setting the environment up](#setting-the-environment-up)
* [Building a demo APK](#building-a-demo-apk)
* [What S0 measured](#what-s0-measured)
* [Findings that change the plan](#findings-that-change-the-plan)
* [S1 — Rust on Android works](#s1--rust-on-android-works)
* [S2 — Commune compiles for Android](#s2--commune-compiles-for-android)
* [S3 — Commune on Android](#s3--commune-on-android)
* [S5 — Notifications](#s5--notifications)
* [S5b — UnifiedPush: the endpoint reaches Rust](#s5b--unifiedpush-the-endpoint-reaches-rust)
* [S6 — The formats that would not draw](#s6--the-formats-that-would-not-draw)
* [S7 — aarch64, and an emulator that runs it](#s7--aarch64-and-an-emulator-that-runs-it)
* [S8 — Three input bugs that were one](#s8--three-input-bugs-that-were-one)
* [S9 — The space bar, and the reset that cancelled it](#s9--the-space-bar-and-the-reset-that-cancelled-it)
* [S10 — The keyboard that would not capitalise, and the gesture that left](#s10--the-keyboard-that-would-not-capitalise-and-the-gesture-that-left)
* [Before this ships](#before-this-ships)
* [Known gaps](#known-gaps)
<!-- /toc -->

## State today

| Spike | State |
| --- | --- |
| S0 — pixiewood baseline | **done**, gtk4-demo runs on the emulator |
| S1 — Rust hello-world APK | **done** — a Rust GTK app runs as an APK |
| S2 — Commune `cargo check` for Android | **done** — clean, with two small Android arms added |
| S3 — Commune login on the emulator | **done** — a password login against a homeserver completes and the session opens, which puts `matrix-sdk`, the bundled SQLite store, the crypto stack, the Keystore-sealed secrets and the device trust roots all on one exercised path |
| S4 — GStreamer | **steps 0, 1 and 2 done, 25 August 2026** — GStreamer 1.28.6 links statically out of the upstream Android binaries against pixiewood's GLib, 28 plugins are registered, and **audio and video both play on the emulator** through the same code every other platform runs. A voice message plays through OpenSL ES; an mp4 previews, sends and plays in the timeline, as does one that was already in the room. GTK's own `media-gstreamer` turned out not to be needed: `GstMediaStream`, written for macOS, covers Android too. Still missing: hardware decode via `androidmedia`, and calls. The plan and every measurement are in `doc/android-media-plan.md` |
| S5 — keystore, notifications, SSO, push | keystore **done**, brought forward into S3 because logging in should not come first; SSO **done** and confirmed against `matrix.org`; notifications **done** — a real message posts a real notification and tapping it opens the conversation; background delivery **done** via a foreground service, capped at six hours a day by Android 15; **real push is under way as S5b** — the plan is `doc/android-push-plan.md`, its step 0 (server chain, with `curl` alone) and step 1 (UnifiedPush registration; a real endpoint reaches Rust, and a push at a dead process starts the process) are **done, 26 August 2026**; next is step 2, the pusher |
| S6 — image formats | **done and confirmed on the emulator** — HEIC, HEIF and AVIF through gdk-pixbuf's Android loaders and SVG through GTK's own renderer, both of which were already in the APK. JXL is still unreadable |
| S7 — aarch64 | **builds and runs** — linked first time, and the emulator's ARM64 translation runs the arm64 APK, so a phone is needed once rather than every iteration. **Run on real hardware 24 August 2026** — a Pixel 9a on GrapheneOS, Android 17: installs, launches, renders with no GL errors, soft keyboard works |
| S8 — input handling | **the URL keyboard and plaintext passwords are fixed and confirmed on a Pixel 9a**, and were one bug: the Android IM context read a struct field nothing had assigned since `_init` |
| S9 — the space-bar cursor slide | **fixed, and confirmed on a Pixel 9a on 25 August 2026**. Three parts: the keyboard could not read the text, `GtkIMContext` cannot move a cursor so the move is spelled in arrow keys, and — the actual cause — every cursor movement was calling `InputMethodManager.restartInput` and cancelling the gesture. Also: the emulator **can** be used to test keyboards, which unblocks every input measurement in this ledger |
| S10 — auto-capitalisation and the back gesture | **fixed, 25 August 2026, on the emulator.** Two unrelated things that both made the app feel unlike an Android app: the composer never asked for sentence capitalisation, and every back gesture closed the application from wherever you were, media viewer included |

## Where things are

Everything the port needs on this machine, in one place, so that a session that has lost its
context can pick the work up from this file alone.

| What | Where |
| --- | --- |
| The branch | `android-port`, worktree `C:\Users\steeb-ai\commune-android` (based on `main`) |
| Build host | WSL `archlinux`, user `steeb`; the emulator and `adb` stay on the Windows side |
| Former build host | WSL `Ubuntu-24.04` — retired for S3, too old to build libadwaita; still holds the S1 and S2 work |
| Android SDK/NDK (Linux) | `~/android/sdk` on Arch, NDK `27.2.12479018` |
| Android SDK (Windows) | `C:\Android\Sdk` — for `adb` and the emulator only, **not** shareable with WSL |
| Emulator | AVD `seed_api35`, API 35, x86_64 |
| pixiewood | `~/src/gtk-android-builder` (revision `482e8ab`) |
| Android Studio jar shim | `~/android/fake-studio` (for `Svg2Avd`, see below) |
| GTK checkout | `~/src/gtk` — builds gtk4-demo |
| libadwaita checkout | `~/src/libadwaita` — **builds and runs** on Arch; also carries a scratch `gtksourceview.wrap` from testing that wrap, and a `dependency()` line appended to its `meson.build`  |
| S1 Rust spike | `~/src/gtk-android-rust-spike` — the working Rust-to-APK recipe (on the Ubuntu host) |
| S2 pkg-config view | `~/android/commune-pc`, generated by `build-aux/android/pkgconfig-stubs.sh` |
| S2 cargo target dir | `~/android/commune-target` (kept off the 9p mount, which is slow) |
| Commune Android build | `.pixiewood/` in the worktree; gitignored, `prepare` rebuilds it |

Commune's sources are read from `/mnt/c/Users/steeb-ai/commune-android`, but cargo's output goes to
a WSL-native `CARGO_TARGET_DIR`; building on the 9p mount is painfully slow.

### Re-running the S2 checks

Since S4 this needs GStreamer on the search path, and GStreamer is not stubbed for Android — it is
a real dependency there now, and `pkgconfig-stubs.sh` only stubs it under `WITH_LINUX_ONLY=1`. So
the view is taken from **Commune's own** build tree rather than libadwaita's, and the pruned
GStreamer prefix is appended to the search path exactly as `meson.build` does it. Note that
`$GSTREAMER_ANDROID_PREFIX` names the **per-architecture** directory,
`~/android/gst-android/x86_64`, not `~/android/gst-android`.

```sh
sh build-aux/android/pkgconfig-stubs.sh     $HOME/android/commune-pc $PWD/.pixiewood/bin-x86_64
GST=${GSTREAMER_ANDROID_PREFIX:-$HOME/android/gst-android/x86_64}
NDK=$HOME/android/sdk/ndk/27.2.12479018
TOOL=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin
export PKG_CONFIG_PATH=$HOME/android/commune-pc:$GST/lib/pkgconfig
export PKG_CONFIG_LIBDIR=$HOME/android/commune-pc:$GST/lib/pkgconfig
export PKG_CONFIG_ALLOW_CROSS=1
export CC_x86_64_linux_android=$TOOL/x86_64-linux-android31-clang
export CXX_x86_64_linux_android=$TOOL/x86_64-linux-android31-clang++
export AR_x86_64_linux_android=$TOOL/llvm-ar
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER=$TOOL/x86_64-linux-android31-clang
export CARGO_TARGET_DIR=$HOME/android/commune-target

cd /mnt/c/Users/steeb-ai/commune-android
cargo clippy --target x86_64-linux-android --all-targets
```

`src/config.rs` is normally generated by meson and is gitignored; a standalone `cargo check` needs
it to exist. Write it by hand from `src/config.rs.in` — the values only have to parse, since S2
compiles and does not run.

`pkgconfig-stubs.sh` reads its real `.pc` files from a meson build tree, and on the Arch host that
tree is **libadwaita's** (`~/src/libadwaita/.pixiewood/bin-x86_64`), which is the script's default.
That is a wider view than the S1 spike gave: libadwaita pulls in GTK, GLib, Pango and cairo, so
`libadwaita-1` is now a real module and one fewer thing is stubbed. Pass a different build
directory as the second argument to use another.

To check the **Linux** path from the same machine, regenerate the stubs with
`WITH_LINUX_ONLY=1` (which adds glycin) into a separate directory and build for
`x86_64-unknown-linux-gnu` with no cross variables set.

### A note on running things from PowerShell

`wsl -d archlinux -- bash -lc '…'` mangles `$VAR` and parenthesised paths before bash sees them,
which produces bewildering errors. Write the script to a file and run
`wsl -d archlinux -u steeb -- bash -l /mnt/c/…/script.sh` instead. The `.gitattributes` rule keeping
`build-aux/android/*.sh` at LF exists for the same reason: a POSIX shell will not run a CRLF script.

`-u steeb` is worth being deliberate about. The Arch WSL image starts as `root`, and building as
root leaves root-owned artefacts in the source tree and in `~/.cargo`, which then break the next
non-root build. Only the `pacman` steps want root.

## The build host

pixiewood, meson and the NDK are Linux tools, so the build host is WSL, while the emulator and
`adb` stay on the Windows side. The APK crosses over `/mnt/c`.

The host is **WSL `archlinux`**. It started as Ubuntu 24.04, which was enough for S0 through S2 but
**cannot build libadwaita** — the reason is the GLib section below, and it is a property of the
distribution's age rather than anything fixable in the build. Since Commune is a libadwaita
application, the host had to move before S3 could start.

Arch was chosen over Ubuntu 26.04, Fedora 44 and building GLib into `/usr` by hand, for one
reason: the requirement is not a fixed version but a moving one. Every pixiewood wrap is
`revision = main`, so the GLib being cross-compiled advances every week, and the host's code
generators have to keep up with it. A five-year LTS is the wrong shape for chasing a development
branch; GTK's own CI uses a rolling image for the same reason.

One honest qualification, since it is easy to state this too strongly: rolling does **not** put the
host ahead of the wrap. Arch ships GLib 2.88.3, current stable, while the wrap builds 2.89.4 from
`main`. `main` is a development series and will always be in front. What rolling buys is that the
host tracks stable closely instead of freezing years behind it — the gap stays weeks, not epochs.
The failure mode is not eliminated, only made much less likely, and it will announce itself the
same way if a wrap ever adopts a macro newer than 2.88.

What the move also bought, for free:

* **meson 1.12.0 from the repositories**, so the pipx install Ubuntu needed is gone.
  `android_exe_type` landed in meson 1.9 and Ubuntu 24.04 shipped 1.3.2.
* **No `--meson` argument.** pixiewood's README still offers `--meson <forked/meson/meson.py>`, but
  that predates the kwarg being upstream; stock meson is enough.

The Windows Android SDK at `C:\Android\Sdk` (the one seed-sync uses) **cannot be shared with WSL**.
Its NDK carries only `toolchains/llvm/prebuilt/windows-x86_64` and its build tools are `.exe`, so
WSL needs its own SDK. That is 2.4 GB at `~/android/sdk` and the duplication is unavoidable. Both
SDKs deliberately hold the same NDK version, `27.2.12479018` — the one GTK's own CI pins in
`.gitlab-ci/android-sdk.sh`.

### Windows `PATH` interop has to be turned off

WSL appends the Windows `PATH` to the Linux one by default. That is convenient until a Windows
executable shadows a Linux tool the build needs, and then it is baffling:

```text
/…/arch-stage2.sh: /mnt/c/Strawberry/perl/bin/cpanm: perl: bad interpreter: No such file or directory
```

Arch installs `cpanm` to `/usr/bin/vendor_perl`, which is not on the default `PATH`, so the lookup
fell through to **Strawberry Perl on the C: drive** and tried to run a Windows script under Linux.
`perl`, `java` and `git` are all equally shadowable, and this build runs all three.

`/etc/wsl.conf` closes the whole class off:

```ini
[interop]
enabled = true
appendWindowsPath = false
```

Nothing in the Android build needs a Windows binary — the APK crosses over `/mnt/c` and is
installed by the Windows `adb` from the Windows side. Applying this needs
`wsl --terminate archlinux`. While in that file, set the locale too (`locale-gen en_US.UTF-8`,
`/etc/locale.conf`), or every pacman hook warns about it.

## Setting the environment up

Run `sh build-aux/android/probe-env.sh` in WSL first; it prints what is missing. The full setup:

```sh
# Arch's own keyring first, on a fresh WSL image
sudo pacman-key --init && sudo pacman-key --populate archlinux
sudo pacman -Syu --noconfirm

sudo pacman -S --needed \
  base-devel git curl unzip which \
  meson ninja sassc gettext libxml2 glib2-devel gobject-introspection appstream shaderc \
  jdk17-openjdk \
  glib-perl perl-glib-object-introspection perl-ipc-run perl-json \
  perl-xml-libxml perl-xml-libxslt perl-app-cpanminus \
  rustup python \
  `# Commune's own build needs these, and only found out one at a time:` \
  blueprint-compiler desktop-file-utils dart-sass \
  `# not linked into the APK — blueprint-compiler reads their typelibs` \
  gtk4 libadwaita gtksourceview5 libshumate gst-plugins-bad

rustup default stable
rustup target add x86_64-linux-android

# Set::Scalar has no Arch package; it is pure Perl, so CPAN is fine
/usr/bin/vendor_perl/cpanm --notest Set::Scalar

# Linux SDK, separate from the Windows one
SDK=$HOME/android/sdk
curl -fsSL -o /tmp/clt.zip \
  https://dl.google.com/android/repository/commandlinetools-linux-13114758_latest.zip
unzip -q /tmp/clt.zip -d /tmp/clt && mkdir -p $SDK/cmdline-tools
mv /tmp/clt/cmdline-tools $SDK/cmdline-tools/latest
yes | $SDK/cmdline-tools/latest/bin/sdkmanager --sdk_root=$SDK --licenses
$SDK/cmdline-tools/latest/bin/sdkmanager --sdk_root=$SDK \
  "platforms;android-35" "build-tools;35.0.0" "ndk;27.2.12479018"

mkdir -p ~/src ~/.local/bin      # the Makefile symlinks into ~/.local/bin and will not create it
git clone https://github.com/sp1ritCS/gtk-android-builder.git ~/src/gtk-android-builder
make -C ~/src/gtk-android-builder prefix=$HOME/.local install
```

Two Arch-specific notes on the package list, both of which cost time to work out:

* **The Perl bindings are named inconsistently.** `glib-perl` and `cairo-perl` keep the old
  `<name>-perl` form, while everything newer is `perl-<name>`. `perl-glib` does not exist; asking
  for it fails while `perl-glib-object-introspection` — which depends on `glib-perl` — installs
  perfectly well.
* **`Set::Scalar` is in neither repository nor a sensible AUR detour.** It is pure Perl with no XS,
  so `cpanm` is the honest answer rather than pulling in an AUR helper for one module.

Add `~/.local/bin` and `/usr/bin/vendor_perl` to `PATH`, and set
`JAVA_HOME=/usr/lib/jvm/java-17-openjdk`, in `/etc/profile.d/`.

The last group is worth a note, because installing GTK on a machine that is
*cross*-compiling GTK looks wrong. It is not: `blueprint-compiler` reads
GObject-introspection **typelibs** to resolve `using Gtk 4.0` and the rest, and
those are host data. None of it reaches the APK, which gets the cross-built GTK
from `subprojects/`. Commune's Blueprints name five namespaces — `Gtk 4.0`,
`Adw 1`, `GtkSource 5`, `Shumate 1.0` and `GstPlay 1.0` — and the last two are
needed even though their Rust is gated out, because the `.blp` files are still
compiled. That is harmless: a compiled `.ui` for a widget nothing instantiates
is inert.

### `wsl.exe` needs `-d archlinux`, and the failure looks like something else

There are two distributions on this machine — `Ubuntu-24.04`, which is the **default**, and
`archlinux`, which is where all of this lives. `wsl.exe` without `-d` goes to Ubuntu, and the
result is not an error but a plausible lie.

Meson bakes the absolute path of every tool it found into `build.ninja`, and on Arch `/usr/sbin`
is a symlink to `bin`, so those paths read `/usr/sbin/meson`, `/usr/sbin/cargo`,
`/usr/sbin/pkg-config`. On Ubuntu none of them exist. Both machines have a `/home/steeb` and the
same `~/android/sdk/ndk/27.2.12479018`, so the build tree looks addressable from either — and an
hour went into "repairing" an environment that was never broken, by a session that had this written
down and did not read it.

Worse than wasted time, it **corrupted the build tree**, and the corruption surfaced far from its
cause. A `meson setup --reconfigure --clearcache` run against Ubuntu found Ubuntu's `nasm`, which
Arch does not have, and wrote `#define WITH_SIMD 1` into libjpeg-turbo's generated `jconfig.h`.
This wrap has no `jsimd_none.c` to fall back on — it links `simd`, which is `[]` when NASM is
missing — so the next build on Arch compiled `jccolor.c` with the SIMD calls live and no SIMD
library to resolve them:

```text
ld.lld: error: undefined symbol: jsimd_can_rgb_gray
>>> referenced by jccolor.c:634
```

Nothing about that error points at the distribution, or at a reconfigure two hours earlier.
The repair was to delete `.pixiewood/bin-x86_64/subprojects/libjpeg-turbo-3.1.1` — meson will not
rewrite a `configure_file` output it believes is current — and reconfigure from Arch.

One more of the same family, harmless but confusing: `hooks/checks-bin` is generated by
`meson setup` with the **source root baked in**, and the copy in this worktree was built by a setup
rooted at the `commune` worktree. It therefore lints `windows-port` rather than this branch, and
fails the pre-commit hook on files that are not here. Run `cargo +nightly fmt --all --check` in this
worktree by hand and commit with `--no-verify` until it is regenerated here.

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
# Studio's real lib/lib.jar carries the bundled Kotlin stdlib; see below
curl -fsSLo $FS/lib/lib.jar \
  https://repo1.maven.org/maven2/org/jetbrains/kotlin/kotlin-stdlib/2.1.0/kotlin-stdlib-2.1.0.jar
```

Then pass `-a $HOME/android/fake-studio` to `pixiewood prepare`. About 7 MB instead of several GB.

#### The fifth jar, which only gradients reveal

The four jars above are enough for GTK's `Demo4` icon and **not** enough for libadwaita's. The
Adwaita demo dies in `generate` with:

```text
java.lang.NoClassDefFoundError: kotlin/jvm/internal/Intrinsics
	at com.android.ide.common.vectordrawable.VdUtil.parseColorValue(VdUtil.kt)
	at com.android.ide.common.vectordrawable.SvgGradientNode.writeGradientStops(SvgGradientNode.java:392)
```

`VdUtil` is written in Kotlin, so anything reaching it needs the **Kotlin stdlib** — and the only
path that reaches it is `writeGradientStops`. GTK's icon has flat colours and never goes there;
libadwaita's `background.svg` has a gradient and does. **Commune's icon has gradients too**, so this
would have blocked S3 regardless.

Adding `kotlin-stdlib.jar` under its own name does nothing, and that is the part worth remembering:
pixiewood does not glob the shim directory, it maps over a **fixed list of six paths**
(`pixiewood:588`), and `lib/lib.jar` is one of them. In a real Studio installation that merged jar
is where the bundled Kotlin lives, so the stdlib has to be installed _as_ `lib/lib.jar`. No patch to
pixiewood is needed — only the right filename.

## Building a demo APK

Both GTK and libadwaita ship their own pixiewood manifest, so the demo build is the manifest plus
three commands. libadwaita's is the one that matters, since Commune is a libadwaita application:

```sh
git clone --depth 1 https://gitlab.gnome.org/GNOME/libadwaita.git ~/src/libadwaita
cd ~/src/libadwaita
M=build-aux/android/org.gnome.Adwaita1.Demo.xml

# Build only the emulator's architecture. The manifest XIncludes the metainfo out of
# the *build* tree, so the href has to name the same arch as the whitelist or the
# XInclude points at a directory that was never built.
sed -i 's|build://aarch64/|build://x86_64/|' $M
sed -i 's|</configure-options>|</configure-options>\n    <architectures mode="whitelist">\n      <arch>x86_64</arch>\n    </architectures>|' $M

PW="perl $HOME/src/gtk-android-builder/pixiewood -C $HOME/src/libadwaita"
$PW prepare --sdk $HOME/android/sdk --toolchain $HOME/android/sdk/ndk/27.2.12479018 \
    --meson /usr/bin/meson -a $HOME/android/fake-studio $M
$PW generate
$PW build
```

GTK's own demo is the same shape with `build-aux/android/org.gtk.Demo4.xml`; its manifest already
says `build://x86_64`, so only the whitelist has to be added.

The APK lands in `.pixiewood/android/app/build/outputs/apk/debug/app-x86_64-debug.apk`. Install it
from the Windows side, where the emulator lives:

```pwsh
wsl -d archlinux -u steeb -- cp <apk> /mnt/c/Users/<user>/AppData/Local/Temp/demo.apk
C:\Android\Sdk\platform-tools\adb.exe install -r $env:TEMP\demo.apk
```

**pixiewood lowercases the application id into the package name**: the manifest says
`org.gnome.Adwaita1.Demo` and the installed package is `org.gnome.adwaita1.demo`. `adb shell monkey
-p <the id as written>` answers "No activities found to run", which looks like a failed install and
is not one. `adb shell pm list packages` is the authority.

Timings on this machine, from a cold clone: `prepare` (meson configure of GTK plus 19 subprojects)
about two minutes; `build` **80 seconds for 3113 ninja targets**, then Gradle 14 seconds. Cross-
compiling the whole GTK stack is not the slow part of this port.

The debug APK is **136 MB** — unstripped, and carrying both demos GTK builds. Gradle says as much:
`Unable to strip the following libraries, packaging them as they are:` and then all 32 `.so`s.
A release build with stripping is the number that matters, and has not been measured.

The libadwaita demo on the Arch host, with the package cache pre-seeded: `prepare` configured
libadwaita plus 22 subprojects, `build` took **1m16s** wall clock (11m user across cores) of which
Gradle was 45s, and the debug APK is **125 MB**. Same conclusion as GTK's — cross-compiling the
whole stack is not the slow part of this port.

The package cache is worth carrying between hosts, not just between projects. Moving to Arch meant
every wrap tarball would have been fetched again, including the `pixman` download that stalls
(below); `tar`ing the Ubuntu host's `subprojects/packagecache` (26 files, 13 MB) into the new
checkouts avoided the lot.

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

### The host's GLib tools were too old for the GLib being built

**Resolved by moving the host to Arch.** Kept because the mechanism is worth knowing and the
failure will recur the next time the wrap outruns the distribution.

Cross-compiling needs two GLibs: the target one, built from the `main` wrap, and the *build
machine's* code generators — `glib-mkenums`, `glib-genmarshal`, `glib-compile-resources` — which
meson takes from the host. On Ubuntu 24.04 those are GLib **2.80**, while the wrap builds 2.89.
That gap is not academic: it stopped a build dead.

libadwaita `main` failed to compile with

```text
../../src/adw-tab-view.c:2617:25: error: use of undeclared identifier 'ADW_TYPE_TAB_VIEW_SHORTCUTS'
```

which reads like an Android problem and is not one. `adw-tab-view.h` is the only libadwaita header
that writes `} G_GNUC_FLAG_ENUM AdwTabViewShortcuts;`, and `G_GNUC_FLAG_ENUM` is **`Since: 2.88`**
(`glib/gmacros.h`) — so 2.80's `glib-mkenums` has never heard of it.

The exact mechanism is worth stating, because "it skipped the type" was the first reading and it is
not quite right. 2.80 does not skip anything: it **takes the macro to be the type's name**. Running
both hosts' `glib-mkenums` over the same construct shows it plainly:

```text
$ glib-mkenums --fhead "" --eprod "PARSED type=@type@ name=@EnumName@" test.h
2.80.0  →  PARSED type=flags name=G_GNUC_FLAG_ENUM      # Ubuntu
2.88.3  →  PARSED type=flags name=AdwTabViewShortcuts   # Arch
```

GLib's own documentation for the macro predicts this: "the attribute can also be placed after
`enum` and before the opening brace, but that may cause it to be misinterpreted as the name of the
enum if the macro is not defined." So 2.80 dutifully generated `ADW_TYPE_G_GNUC_FLAG_ENUM`, which
is why `adw-enums.h` held 25 `ADW_TYPE_` entries with none of the `TAB_VIEW` ones, and why the
compiler complained about a name nothing had ever written. A wrong symbol at the end of a long
build, with the real fault three steps upstream and no warning anywhere.

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

So the real conclusion was about the build host, not about a shim: **Ubuntu 24.04 is too old to
build libadwaita `main`.** The options were a newer distribution in WSL, a container, or a GLib
built and installed natively so that `/usr/bin` genuinely carries the newer tools. The first was
taken — see [The build host](#the-build-host) for why Arch specifically.

The sequencing mattered at the time. GTK `main` itself builds on Ubuntu — nothing in it uses
`G_GNUC_FLAG_ENUM` — so S0 and S1 were never blocked; the Rust spike needs GTK and a C stub, not
libadwaita. Only S3 was, and only because Commune is a libadwaita application.

Worth carrying forward: Commune's own build runs `glib-compile-resources` and
`glib-compile-schemas` from the host too. Both are 2.88.3 on Arch and neither has complained, but if
one ever does, this is the shape of the fix — take the tool from the subproject, not from the
distribution, and if that cannot be arranged, move the host.

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

## S2 — Commune compiles for Android

**`cargo check --target x86_64-linux-android` passed on the first attempt**, on an unmodified
tree, with three warnings — all of them from the `UnimplementedSecret` stub the `cfg_if!` was
falling through to. `cargo clippy --all-targets` for Android is clean too.

That is the answer to the question S2 was posed to ask. **Almost none of the 110k lines is platform
-dirty.** The macOS and Windows ports cut most of their seams at `not(target_os = "linux")` rather
than at a named platform, and Android inherits every one of them: the `image`-crate decoder, the
camera and location fallbacks, the `SystemSettings` fallback, `PRIMARY_MASK`, the unconditional
`FileLauncher`. The Cargo target tables do the same — `aperture`, `ashpd`, `glycin` and `oo7` are
all behind `cfg(target_os = "linux")` and simply do not appear in an Android build.

### The known risk did not materialise

`doc/android-plan.md` named **`aws-lc-sys`** as the thing most likely to stop S2, with a fallback to
the SDK's `ring` provider. It was not needed:

```text
target/x86_64-linux-android/debug/build/aws-lc-sys-*/out/libaws_lc_0_44_0_crypto.a
```

`aws-lc-sys v0.44.0` cross-compiled its C crypto library for Android with nothing but the NDK
`CC`/`AR` environment variables set. `matrix-sdk` keeps `rustls-aws-lc-rs`; no feature change, no
divergence from the desktop builds on the crypto path — which is exactly where divergence would
have been least welcome.

Everything else in the stack came along quietly: the whole of `matrix-sdk` including
`matrix-sdk-crypto`, `matrix-sdk-sqlite` and `matrix-sdk-search` (tantivy), `ruma`, `reqwest`,
`rustls-platform-verifier`, and the gtk-rs family up to `libadwaita`, `sourceview5` and
`libshumate`.

### How the check was run without the libraries existing

`cargo check` never links. A `-sys` crate's build script needs the pkg-config _module_ to exist
with an acceptable version; it never opens the library. So the check runs against the real `.pc`
files from the S1 GTK build plus **stubs** for everything not yet cross-built — libadwaita,
gtksourceview, libshumate, the GStreamer modules, sqlite3, libwebp:

```sh
prefix=/nonexistent-android-stub
Name: gstreamer-1.0
Version: 1.28.0
Libs: -L${libdir} -lgstreamer-1.0
```

`build-aux/android/pkgconfig-stubs.sh` regenerates the whole directory. This is honest for S2,
whose question is "does the Rust compile", and dishonest for anything else: it proves nothing about
linking, and nothing about those libraries existing on a device. S3 must either cross-build them or
gate them out for real.

The same trick, with `glycin-2` stubs added, verifies the **Linux** path still compiles after the
Android changes — `cargo check --target x86_64-unknown-linux-gnu` is clean, `aperture`, `ashpd`,
`oo7` and `glycin` included. That is how "Linux must stay untouched" is checked from a machine with
no GTK development packages installed.

### What was actually changed

Two arms, both small, because there was nothing else to fix:

* **`src/secret/android.rs`** — sessions as one JSON file each under
  `<data>/secrets.d/<id>.json`, written atomically, `0600` in a `0700` directory, carrying the same
  `version: 1` payload the macOS Keychain backend stores. `src/secret/mod.rs` gains the `cfg_if!`
  arm; `unimplemented` stays for other platforms.

  **This is a placeholder and the file says so at the top.** It leans on the application sandbox —
  `getFilesDir()` is owned by the application's UID — and nothing else. The passphrase that
  encrypts the local databases is in plain JSON. Android Keystore through JNI is the real answer
  and is the first task of S5; the `SecretExt` surface is the seam, so only that one file changes.
  Nothing may be handed to anyone before it does.
* **`src/main.rs`** — a `paranoid_android` logcat layer under `cfg(target_os = "android")`. An
  Android application has no stdout, so the existing subscriber would have written into nothing.
  `adb logcat -s Commune` reads it back.

### What compiling does not mean

It compiles; it has never run. `fn main()` is still a `fn main()` — S1's C stub and `staticlib`
wiring have not been applied to Commune, so there is no APK yet. Nothing has been done about the
gresource and locale paths, which are still Meson's compile-time absolutes, about GStreamer being
absent, or about the SSO redirect. Those are S3.

One warning is left in the Android clippy run, and it is not ours:
`src/utils/matrix/url_preview/tests.rs:106` trips `unnecessary hashes around raw string literal`.
It is pre-existing test code and this host's clippy (1.96) is newer than the one the project's
pre-commit hook runs, which is the whole of the explanation. It appears in the Linux check too and
is left alone.

## S3 — Commune on Android

S3 is the spike where Commune itself has to launch, and its first task was not Commune at all: the
host could not build libadwaita, and nothing else in S3 mattered until it could. That part is done.

### What the new host was measured against

The gate was deliberately the same libadwaita commit that failed on Ubuntu (`e5c9cd8`), so the
comparison is like for like and the only variable is the host.

| Test | Result |
| --- | --- |
| Host `glib-mkenums` parses `G_GNUC_FLAG_ENUM` | yes — `name=AdwTabViewShortcuts`, where 2.80 gave `name=G_GNUC_FLAG_ENUM` |
| `adw-enums.h` carries the type | yes — **26** `ADW_TYPE_` entries against Ubuntu's 25, `ADW_TYPE_TAB_VIEW_SHORTCUTS` at line 129 |
| libadwaita `main` cross-compiles for `x86_64-linux-android` | yes, unpatched |
| Adwaita demo APK builds | yes, 125 MB debug, 1m16s |
| Installs and launches on `seed_api35` | yes, `org.gnome.adwaita1.demo/org.gtk.android.ToplevelActivity` |
| Renders correctly | yes — boxed lists, symbolic icons, header bar, Adwaita styling all as on the desktop |
| `AdwNavigationView` push | yes, back arrow and transition correct |
| **`AdwTabView` at runtime** | yes — three tabs, tab-count button, editable page entry, nothing logged at critical level |

The last row is the one that closes the loop. `ADW_TYPE_TAB_VIEW_SHORTCUTS` was the symbol that did
not exist on Ubuntu; the Tab View demo instantiates the widget that registers it. It runs, and
`logcat` shows nothing from `GLib-GObject` or `Gtk` at critical level, so the type is not merely
present in a header but actually registered.

This is also the first evidence for Route A that is about _Commune's own toolkit_ rather than about
GTK. libadwaita is what the entire UI is written against, and it works on Android at the version we
build from.

### The runtime paths, which the plan had wrong

Reading `gdkandroidruntime.c:260-281` rather than assuming, the glue sets GLib's user dirs like
this — and the asymmetry matters:

| GLib dir | Points at | Holds |
| --- | --- | --- |
| `XDG_DATA_DIRS` | `getFilesDir()/share` — **internal** | the extracted `assets/`: gresources, locale, schemas |
| `XDG_CONFIG_DIRS` | `getFilesDir()/etc` — internal | |
| `XDG_DATA_HOME` | `getExternalFilesDir(null)/share` — **external** | user data |
| `XDG_CONFIG_HOME` | `getExternalFilesDir(null)/etc` — external | |

`doc/android-plan.md` said the `app_bundle.rs` Android arm should find the gresources under
`$XDG_DATA_HOME/commune/`. **That is wrong**: `XDG_DATA_HOME` is external storage, where the assets
are never extracted. The gresources are in `XDG_DATA_DIRS`, and the arm has to read that.

Two further consequences, both easy to get wrong:

* These are set with **`g_set_user_dirs()`**, a GLib-internal call the glue declares itself — not
  with `setenv`. So `std::env::var("XDG_DATA_DIRS")` sees **nothing**. Only GLib's own accessors
  (`glib::system_data_dirs()`, `glib::user_data_dir()`) return these values, and the Android arm of
  `app_bundle.rs` must go through them.
* Anything Commune writes via `glib::user_data_dir()` lands on **external** storage. That needs
  checking against the placeholder secret store before S3 logs in — see the gap below.

### The C libraries, one at a time

`doc/android-plan.md` decided to gate gtksourceview and libshumate out together and get an APK
without either. Looking at what each actually needs, that turns out to be the wrong shape: their
costs are nothing alike, and one of them is load-bearing.

| Library | What it needs | Already cross-built? | Decision |
| --- | --- | --- | --- |
| **gtksourceview-5** | glib, gobject, gio, gtk4, libxml2, fribidi, libpcre2-8; fontconfig and pangoft2 optional | **all of them**, as pixiewood subprojects | **wrap it** |
| **shumate-1.0** | + `libsoup-3.0`, `json-glib-1.0`, `libprotobuf-c`, `sqlite3` | none of those four | gate it |
| libwebp | — | n/a: `libwebp-sys` depends on `cc`, so it compiles libwebp itself | drop the meson assertion on Android |
| sqlite3 | a system library | no `.pc`, but the NDK sysroot ships `libsqlite3.so` | expect `-lsqlite3` to resolve; verify at link time |

The decisive argument is not build cost, it is what gating would cost. **The message composer's
text entry is a `sourceview::View` with a `sourceview::Buffer`**
(`message_toolbar/mod.rs:112`, `composer_state.rs:50,61`), using a markdown language spec and an
Adwaita style scheme. Gating GtkSourceView out means replacing the composer's widget and buffer,
which is precisely the thing S3 exists to exercise — "send a message" is the test. libshumate, by
contrast, is one file (`components/media/location_viewer.rs`) plus the location message row.

So GtkSourceView is cross-built, and it was measured rather than hoped for:

| Test | Result |
| --- | --- |
| Resolves as a pixiewood subproject from a `.wrap` | yes — `Dependency gtksourceview-5 ... found: YES 5.21.1 (overridden)` |
| Cross-compiles for `x86_64-linux-android` | yes, **unpatched**, 1115 build targets |
| Links | yes — `libgtksourceview-5.so`, 2362 ninja steps |
| Patches or source changes needed | none; five `-Dgtksourceview:*` options |

`subprojects/gtksourceview.wrap` carries it. On every platform other than Android the fallback is
switched off with `allow_fallback: host_machine.system() == 'android'`, so a Linux contributor
missing the development package still gets a plain "not found" instead of meson quietly cloning
GtkSourceView and building it from source for twenty minutes.

Three traps found on the way, none of them about GtkSourceView itself:

* **Its default branch is `master`, not `main`.** libshumate's _is_ `main`. A wrap naming the wrong
  one fails with `Remote branch main not found in upstream origin`, reported by meson only as
  `Git command failed`.
* **meson refuses a `PATH`-discovered pkg-config for a cross build**, saying `Pkg-config binary
  missing from cross or native file, or env var undefined` and then `Default target is not allowed
  for cross use`. It has to be named in a cross file's `[binaries]` or in `PKG_CONFIG`. pixiewood's
  cross files name no `pkgconfig` at all, which is consistent: everything it builds is a subproject,
  so meson never needs pkg-config. Only cargo does, which is why S1 pointed `PKG_CONFIG_LIBDIR` at
  `meson-uninstalled` for cargo and not for meson.
* **Building a library standalone against `meson-uninstalled` does not work**, and it is a dead end
  worth not repeating. Configure gets as far as `glib-2.0 found: YES 2.89.4` and then dies on
  `tool variable 'glib_genmarshal' contains erroneous value` — the uninstalled `.pc` advertises code
  generators at build-tree paths that were never generated there. Inside pixiewood the question does
  not arise, because a subproject receives GLib as a meson dependency object carrying real targets.
  Test wraps the way they will be used: as subprojects.

### GStreamer is a wall, not a switch

`doc/android-plan.md` filed GStreamer under features to gate, alongside the camera and
location stubs. That understated it. `gst` and its seven siblings were plain entries in
`[dependencies]` with no `cfg` on them, so without GStreamer the tree did not lose voice messages
and calls — **it did not compile.** 255 errors, in nine files:

| File | Errors | What it is |
| --- | --- | --- |
| `session/calls/pipeline.rs` | 102 | the whole WebRTC pipeline |
| `utils/media/video.rs` | 39 | video metadata and thumbnails |
| `components/media/video_player_renderer.rs` | 17 | |
| `components/media/video_player.rs` | 12 | |
| `session/calls/ringtone.rs` | 10 | |
| `utils/media/audio.rs` | 8 | duration and waveform |
| `components/media/location_viewer.rs` | 5 | libshumate, not GStreamer |
| `utils/media/mod.rs` | 3 | the `Discoverer` helper |
| `main.rs` | 1 | `gst::init()` |

Gating whole modules rather than individual expressions took that to zero. The gate is
`cfg(target_os = "android")` and not a Cargo feature: every platform seam in this codebase is
already shaped that way, and a target cannot be forgotten at the command line the way a feature
flag can.

Two things made this much less invasive than the error count suggests.

**The app already knows how to say "no".** Every fallback here is one the codebase had already: a
location message falls through to the same unsupported-event arm that any unknown `msgtype` hits,
the media viewer's location branch shows the fallback screen it shows for anything it cannot
display, a video message gets the error badge the row already uses for a video that will not load,
and the call buttons are hidden by exactly the code that hides them in a room that cannot be
called. Only one new translatable string was needed, for the video row, because reusing "Could not
retrieve media" would have been a lie. Nothing new was invented, and there is nothing to unpick in
S4 beyond deleting the `cfg`s.

**The compiler chains the gating for you, but only in Rust.** Gating a type forces the gating of
its Rust users, so nothing is missed. Blueprint is the exception, and it fails the other way round:
a `#[template_callback]` that stops existing does not fail to compile, it fails when the template
is instantiated. So `place_call` and `build_video` keep their signatures and stay registered, with
`place_call` a no-op that the hidden buttons can never reach. An orphaned `.blp` for a gated widget
is inert and harmless — it is compiled into the gresource and never referenced — but a live
template referencing a gated type would be a runtime crash. That asymmetry is worth remembering
before gating anything else.

The other libraries sorted themselves out by inspection rather than by trying:

* **libwebp** never needed cross-building. `libwebp-sys` depends on `cc`, so it compiles libwebp
  itself; the Meson `dependency()` was only ever an assertion, and it is gone on Android.
* **sqlite3 does** need a real library. `libsqlite3-sys` has no `cc` dependency, so it does _not_
  vendor a copy. Android ships `libsqlite3.so` in the NDK sysroot but no `.pc` file, so the Meson
  assertion is gone while the link still needs `-lsqlite3` to resolve. Untested: only linking will
  say, and nothing has linked yet.

Verified on both targets. `cargo clippy --all-targets` is clean for `x86_64-linux-android`, and the
Linux build carries exactly the warnings it carried before — the point of the exercise being that
Linux does not notice any of this.

### Commune runs

**The greeter draws, Log In navigates, and typing into the homeserver entry reaches Commune's own
validation** — the Next button changes from insensitive to sensitive as the text arrives. That is
Android key events through GTK's `ImContext`, into a `GtkEditable`, into a Rust property binding,
and back out as a widget state change. Screenshots were taken and are not committed; regenerate
with `adb exec-out screencap -p`.

The launch sequence, from logcat, because it is the thing that was uncertain for three spikes:

```text
nativeloader: Load ... libgtk-4.so ... : ok
GTK Runtime: Starting GTK Android runtime
GLib     : g_set_user_dirs: Setting XDG_DATA_DIRS to /data/user/0/…/files/share
GTK Runtime: Reached GTK Thread
GTK Runtime: Calling main()
Commune  : commune::application: Commune (io.github.steeb_k.Commune)
Commune  : commune::application: Datadir: /data/user/0/…/files/share/commune
```

`Calling main()` is the C stub being found and called; the two `Commune` lines are Rust running and
`app_bundle.rs` having resolved the data directory to exactly where the glue extracted the assets.

| Test | Result |
| --- | --- |
| APK builds | yes — 315 MB debug, x86_64 |
| Installs and launches | yes, `io.github.steeb_k.commune` |
| Greeter renders | yes — illustration, header bar, button, all Adwaita-correct |
| SVG assets load | yes (the greeter and homeserver illustrations are gresource SVGs) |
| Navigation | yes — Log In pushes the homeserver page |
| **Text entry** | **yes** — `adb shell input text` reaches the entry and the Next button becomes sensitive |
| Logging in against a homeserver | **not tried** |

### How it is built

Once per machine, before any of the below, because pixiewood has no wrap for
GStreamer and cannot build one — see `doc/android-media-plan.md`:

```sh
# gstreamer-1.0-android-universal-<version>.tar.xz, ~992 MB, 2.0 GB per
# architecture extracted. From https://gstreamer.freedesktop.org/data/pkg/android/
tar xf gstreamer-1.0-android-universal-1.28.6.tar.xz -C $HOME/android/gstreamer
sh build-aux/android/gstreamer-prefix.sh $HOME/android/gstreamer $HOME/android/gst-android
```

`meson.build` looks for the result at `$GSTREAMER_ANDROID_PREFIX`, defaulting to
`~/android/gst-android`, and refuses to configure without it. **Do not point it at the extracted
tarball directly** — that is what the prefix script exists to prevent, and the script's own header
explains why at length.

```sh
PW="perl $HOME/src/gtk-android-builder/pixiewood -C $PWD"
$PW prepare --sdk $HOME/android/sdk --toolchain $HOME/android/sdk/ndk/27.2.12479018 \
    --meson /usr/bin/meson -a $HOME/android/fake-studio \
    build-aux/android/io.github.steeb_k.Commune.xml
$PW generate
sh build-aux/android/patch-manifest.sh      # between generate and build, always
sh build-aux/android/patch-gtk-ime.sh       # likewise; see "The IME" below
sh build-aux/android/patch-gtk-intent.sh    # likewise; see the SSO section
sh build-aux/android/patch-gtk-service.sh   # likewise; see the foreground service
sh build-aux/android/patch-gtk-input-purpose.sh  # likewise; see the input purpose
sh build-aux/android/patch-gtk-ime-selection.sh  # likewise; see S9
sh build-aux/android/patch-gtk-ime-reset.sh      # likewise; see S9
sh build-aux/android/patch-gtk-ime-caps.sh       # likewise, and after the one above; see S10
sh build-aux/android/patch-gtk-caps-sentences.sh  # likewise; see S10
sh build-aux/android/patch-gtk-jni-attach.sh     # likewise; see the attachment crash below
sh build-aux/android/patch-notification-icon.sh  # likewise; see the small icon below
$PW build
```

Run these in the **archlinux** WSL distro, not Git Bash: `patch-manifest.sh` needs `XML::LibXML`,
which Git Bash's perl does not have, and the Java patches' `perl` and `awk` there mangle the
backslashes in their own substitutions and silently apply nothing.

**The Gradle copies of the glue are hard links, not copies.**
`.pixiewood/android/app/src/main/java/org/gtk/android/*.java` and
`subprojects/gtk/gdk/android/glue/java/org/gtk/android/*.java` are the same inodes, so patching
either patches both and there is no pristine copy left in the tree to diff against. `cp` between
them fails with _"are the same file"_, which is how this was found. To start over, restore from the
wrap's own git rather than from the other path:

```sh
git -C subprojects/gtk checkout -- gdk/android/glue/java/org/gtk/android/ImContext.java
```

All eleven patches run between every `generate` and `build`. `generate` rewrites the manifest from
its own XSL each time, and the Java ones write into `subprojects/gtk`, which a re-extracted wrap
loses. Each script is a no-op when its change is already in place, so running them all every time
is the cheap and correct habit.

About eleven minutes from cold on this machine, most of it Cargo. The APK lands in
`.pixiewood/android/app/build/outputs/apk/debug/app-x86_64-debug.apk`.

### Picking an attachment crashed the application, and sending one never worked

_Found and fixed 25 August 2026_ (`3600ca3c`, `f1765ed`), while trying to get a video into a room
to test S4 step 2. Two separate defects, neither about media, both hit by every attachment of every
type. They are here rather than in `doc/android-media-plan.md` for that reason.

#### The crash: a `GFile` vfunc on a thread with no JVM

```text
F libc  : Fatal signal 11 (SIGSEGV), fault addr 0x0
F DEBUG : #00 libgtk-4.so (gdk_android_content_file_query_info+497)
F DEBUG : #01 libgio-2.0.so (g_file_query_info+376)
F DEBUG : #02 libgio-2.0.so (query_info_async_thread+94)
F DEBUG : #03 libgio-2.0.so (g_task_thread_pool_thread+66)
```

`gdk_android_get_env` returns NULL for a thread that was never attached to the JVM, and its callers
dereference that unchecked — 29 of them in `gdkandroidcontentfile.c` alone, beginning with
`(*env)->PushLocalFrame`. **GIO guarantees those callers run on such a thread**:
`GdkAndroidContentFile` implements the synchronous `GFile` vfuncs and none of the `_async` ones, so
GIO supplies the async variants by running the synchronous vfunc in a `GTask` thread pool. This is
not a race — it is the documented design of both halves, meeting badly.

Patched downstream as `build-aux/android/patch-gtk-jni-attach.sh`: `gdk_android_get_env` attaches
the thread rather than returning NULL, with a `pthread_key` destructor to detach at thread exit,
since ART aborts a thread that exits while still attached. That fixes all 29 call sites at once,
where using the existing `gdk_android_get_thread_env` guard would mean editing each. The script's
header carries the argument in full, including why the classloader difference does not matter here.

#### The send: a `content://` file has no usable URI and no real path

With the crash gone, sending still failed, and the thumbnail with it:

```text
W Commune : commune::utils::media::video: Could not initialize pipeline for video thumbnail
E Commune : Could not send file: Invalid attachment data
```

One cause, two faces. A file the picker returns is a `content://` URI:

* **`GStreamer` cannot resolve it.** `GdkAndroidContentFile` is not a GVfs backend, so nothing
  handles the scheme. The preview, `GstDiscoverer` and the thumbnailer are all given `file.uri()`.
* **`g_file_get_path` lies about it.** GTK answers with `Uri.getPath()`, so
  `content://…/document/video%3A1000000034` becomes `/document/video:1000000034` — absolute-looking
  and pointing at nothing. `AttachmentSource::File` does `fs::read` on it and reports
  `InvalidAttachmentData`, which is that error exactly.

Fixed in Commune rather than in GTK, because a `GFile` that is genuinely not on the filesystem is
allowed to exist and the caller has to cope: `send_file_inner` checks the path rather than trusting
it, and copies the file once when it is not real. Everywhere else the path is real and nothing is
copied.

Two consequences of the copy, both found by testing rather than by reading:

* The upload takes the **bytes**, not the copy's path, because `matrix-sdk` names an attachment
  after the file it read and the copy has a generated name.
* The dialog is **told** the content type rather than sniffing it, because a temporary file has no
  extension for `g_content_type_guess`, and every attachment previewed as "File not Viewable".

The wider point: **this port has never been able to send an attachment**, of any type. It went
unnoticed because the crash arrived first and looked like the whole story.

Both defects share one cause, and it reaches further than the composer: a `content://` file has no
path and says otherwise. The routes that have been fixed, the ones that are merely untested and the
two that are known broken are inventoried in `doc/android-attachments-plan.md`.

### Every attachment open was black, and it was a directory that did not exist

_Found and fixed 25 August 2026_ (`eb951d5a`), reported as "opening pictures just shows a black
screen". Worth writing down because the symptom pointed at the renderer and the cause was nowhere
near it, and because the hazard was already documented in this codebase — reached by a route that
had not been changed with the rest.

`MediaContentViewer` never got a file:

```text
W Commune : commune::session_view::media_viewer::imp:
    Could not retrieve media file: No such file or directory (os error 2)
```

`save_data_to_tmp_file` writes into `TMP_DIR`, and `TMP_DIR` was
`glib::user_runtime_dir()`. The Android glue's `g_set_user_dirs`
(`gdk/android/gdkandroidruntime.c:277`) sets `XDG_CONFIG_DIRS`, `XDG_DATA_DIRS`, `XDG_CONFIG_HOME`
and `XDG_DATA_HOME` — and nothing else. So `g_get_user_runtime_dir` falls back to
`g_get_user_cache_dir`, which falls back to `$HOME/.cache`, and an Android process has no useful
`HOME`.

`DataType::base_dir_path` describes that fallback in its own doc comment and already routes around
it. `TMP_DIR` simply did not go through `DataType`. It does now, so temporary files land in
`<data>/cache/<profile>/tmp` — `getCacheDir()`, which is what Android provides for files it may
reclaim.

The second half is why a missing directory became a missing _file_: the code called
`fs::create_dir`, which creates only the last component and fails with `ENOENT` when the parent is
absent, rather than `fs::create_dir_all`.

**This was not about pictures.** Every attachment the viewer opened went through the same path.

### The APK is about twice the size it needs to be, for two reasons

_Measured 25 August 2026, x86_64 debug._ Neither of these has been acted on; both are recorded here
because every install over a slow link pays for them.

**Roughly 200 MiB of the APK is dead space.** Gradle packages incrementally, and repeated builds
leave orphaned data in the zip that no central-directory entry points at any more:

| | |
| --- | --- |
| APK on disk | 517.5 MiB |
| sum of its entries | 318.9 MiB |
| gaps larger than 1 MiB | 4, the largest 163.4 MiB |

`rm -rf .pixiewood/android/app/build/outputs/apk` before `pixiewood build` produced a 319.4 MiB
APK from the identical inputs — exactly the entry total. Whether it recurs on every incremental
build or only after a mid-build reconfigure is **not established**; the one measurement here
followed a reconfigure that happened while a build was running.

**The strip step fails, and Gradle carries on.** It says so, in a line that is easy to read past:

```text
Unable to strip the following libraries, packaging them as they are:
libcommune.so, libjpeg.so, libturbojpeg.so
```

The NDK's own `llvm-strip --strip-all` handles the same files without complaint, so this is AGP's
task failing rather than an unstrippable binary. What it costs, over all 36 native libraries:

| | |
| --- | --- |
| as built | 315.4 MiB |
| `--strip-all` | **158.5 MiB** |

`libcommune.so` alone is 172.4 MiB as packaged and 108.9 MiB stripped: a 50 MB `.strtab` and an
8 MB `.symtab` that were never meant to ship. And this is still a `buildtype=debug` build, which
compiles the C stack at `-O0` — `libharfbuzz-subset.so` is 35 MiB as built and 4.9 MiB stripped.

What is left after all that is our own binary, and it is the real story: 81 MiB of `.text` plus
about 15 MiB of `.eh_frame` and `.gcc_except_table`. That is Rust with `matrix-sdk`, `ruma` and the
crypto stack statically linked. The whole GTK stack — 35 libraries — is about 50 MiB stripped, and
GStreamer is 0.55 MiB of it. Untried levers, in the order they look worth trying: fixing the strip
task, a `release` build, then `lto`, `codegen-units = 1`, `panic = "abort"` and `strip` in the Cargo
profile.

When only the Rust has changed, `ninja src/libcommune.a` in `.pixiewood/bin-x86_64` is the fast
loop, and the loud one: meson's wrapper truncates compiler output in `pixiewood build`, so a Rust
error there arrives as a `FAILED: [code=101]` line with the diagnostic cut off, while ninja on its
own prints it in full.

**That whole block is only needed when the pixiewood manifest changes.** For a change to Rust,
Blueprint or resources — which is most changes — `pixiewood build` alone is the loop, and the
difference is minutes rather than seconds. `prepare` re-checks every wrap; `generate` rewrites the
Gradle project from its XSL; and the four patch scripts exist purely to repair what `generate`
rewrites, so skipping it skips them too. Running the full sequence every time is a habit worth not
acquiring: it was measured costing several minutes a round on changes whose actual compile work was
27 ninja steps.

Two other costs are worth knowing before blaming the compiler. Everything crosses `/mnt/c`, so
`meson install --tags runtime` copies the entire installed tree onto the Windows filesystem and
Gradle then packs a 334 MB APK and a 660 MB universal one; that I/O dominates a small change. And
`build` does every whitelisted architecture, so since aarch64 was added it is all paid twice —
which is the right default, but not when only the phone is about to be reinstalled.

`build-aux/android/io.github.steeb_k.Commune.xml` is the pixiewood manifest. Two things in it are
worth knowing. The `xi:include` reads the **built** metainfo, so the architecture it names has to
match the whitelist or it points into a directory that was never created. And pixiewood copies one
wrap per name in `<dependencies>` and follows nothing transitively, so an application — unlike GTK
and libadwaita, which have most of this in their own trees — has to name everything it wants a wrap
for.

### The entry point

#### The shape of the crate

GTK's glue `g_module_symbol`s for `main` in the application's shared object, and S1 established the
way to satisfy that from Rust: build the Rust as a **`staticlib`**, and let Meson link it into the
`android_exe_type: 'application'` executable next to a three-line C stub that exports `main`. The
`staticlib` is the load-bearing part — rustc never links one, so cargo needs only the pkg-config
_description_ of GTK and not `libgtk-4.so` itself, which is what avoids a circular dependency in
ninja. A `cdylib` would have to link GTK for real and would reintroduce it.

Commune cannot produce a `staticlib` as it stands, because it is a binary-only crate: `src/main.rs`
is the crate root and `src/meson.build` has a `cargo-build` target that runs `cargo build` and
copies the resulting _binary_. A package cannot expose one set of modules as both a bin root and a
lib root.

**So the crate was split**, and it turned out to be the smallest part of the day.
`src/lib.rs` took the module declarations, the two statics and the body of `main()` as
`pub fn run()`; `src/main.rs` is three lines. No code moved between modules, and Cargo needed no
`[lib]` section at all — it detects both roots and names both targets `commune`, so Meson still
finds the binary where it always did. `#[unsafe(no_mangle)] pub extern "C" fn commune_main()` in
the library is what `build-aux/android/stub.c` calls.

Two things were considered and are not worth revisiting:

* ~~**A second package in a workspace.**~~ The main crate would still have to become a library for
  anything to depend on it, so it is the same change plus a workspace migration.
* ~~**Keep the bin and export `main` from it.**~~ Not possible: `crate-type` is only a `[lib]` key,
  so there is no way to ask Cargo for a `staticlib` or a `cdylib` without a library target,
  whatever the symbols look like.

A `cdylib` — Cargo emitting the `.so` directly, with no C stub and no Meson link — is still not
ruled out, but it would need the same library target and it would have to link `libgtk-4.so` for
real, reintroducing the ninja ordering the `staticlib` sidesteps. There is no reason to try it now
that this works.

`crate-type` is set with `cargo rustc --lib --crate-type staticlib` rather than in `Cargo.toml`,
so no other platform builds a 391 MB archive it will never use.

#### The manifest patch, which is done

`build-aux/android/patch-manifest.sh` runs between `pixiewood generate` and `pixiewood build`. It
exists because pixiewood rewrites `AndroidManifest.xml` from `generate/manifest.xsl` every time,
so neither of these can be hand-edited once, and neither is something the pixiewood manifest can
express. Tested against the Adwaita demo's generated manifest: idempotent, still well-formed, and
the intent filter, `gtk.android.lib_name` metadata and `REORDER_TASKS` permission all survive.

* **`launchMode`, `standard` → `singleTask`.** S0 found this and it is unchanged: GTK has one
  toplevel, Android stacks a new Activity per launch, and returning from the launcher otherwise
  shows an empty white window in front of the working one.
* **`allowBackup`, `true` → `false`.** This one is new, and it matters more than it looks.
  pixiewood leaves Android's default in place, and that default lets `adb backup` and the system's
  cloud backup copy the application's private files off the device — including, right now, the
  plaintext file holding the passphrase that encrypts the local databases. The secret store's
  documentation had assumed backup was disabled; it was not. It stays disabled after the Keystore
  work too: a Keystore key cannot leave the device, so a backup carrying the databases without it
  would restore something unreadable.

### Four things that stopped a build or a launch

None of these were about Rust, Android or GTK. They are the kind of thing that only appears when
something is built for real, and each cost more to diagnose than to fix.

**`*.sh` must be LF, and the rule was too narrow.** `build-aux/compile-blueprints.sh` came out of
this Windows checkout with CRLF and killed the build **2654 targets in**, reported as:

```text
env: ‘bash\r’: No such file or directory
```

which names the interpreter rather than the script. `.gitattributes` had covered only
`build-aux/android/*.sh`; it now covers `*.sh`. This would have bitten the Windows port too —
`compile-blueprints.sh` is called by every platform's build.

**SQLite is not an NDK API.** The ledger previously said Android ships `libsqlite3.so` in the NDK
sysroot. **It does not** — there is no `libsqlite3` anywhere in it; the sysroot has 33 libraries and
sqlite is not among them. Android has SQLite, but only through Java, so every application wanting
it natively ships its own. `libsqlite3-sys` does not vendor a copy, so Android turns on
`rusqlite`'s `bundled` feature (`Cargo.toml`, Android section only), and SQLite is compiled into
the archive Cargo hands over.

**The GSettings schema was never installed**, and the app aborted on first launch:

```text
F GLib-GIO: Settings schema 'io.github.steeb_k.Commune' is not installed
F libc   : Fatal signal 6 (SIGABRT) in tid … (GTK Thread)
```

`configure_file` carries no install tag, and pixiewood packages with
`meson install --tags runtime`, which installs only tagged files — so the schema was skipped
silently while GTK's own, which are tagged, arrived. `data/meson.build` now sets
`install_tag: 'runtime'`. Worth remembering as a class: **anything installed by `configure_file`
is invisible to a tagged install.**

**`[profile.release] debug = true`** is deliberate for desktop and ruinous here. It produced a
3.4 GB static library and a 1.6 GB APK, with Gradle reporting it could not strip any of it. Turned
off for Android only (`--config profile.release.debug=false`), the archive is 391 MB and the APK
315 MB.

One measurement trap while chasing that size: **Gradle updates the APK in place**, so when the
native library shrank from 1.4 GB to 168 MB the APK stayed 1.65 GB — the old bytes orphaned inside
the zip, whose entries added up to only 333 MB. `rm -rf .pixiewood/android/app/build/{outputs,intermediates}`
and rebuilding gives the real number. Compare `unzip -l` against the file size before believing
either.

### The secret store

Done before trying to log in, deliberately: a login that wrote a plaintext passphrase to
semi-public storage would only be manufacturing something to clean up afterwards.

There were two problems here and they were easy to mistake for one.

**Where the file lives.** `DataType::Persistent` was `glib::user_data_dir()`, which the glue points
at `Context.getExternalFilesDir(null)` — external storage. It is now derived from `XDG_DATA_DIRS`,
which the glue sets to `Context.getFilesDir()`, and the cache with it, since GLib had no answer for
that at all. This moves the SDK's SQLite databases too, not only the secret store: sealing the
passphrase while leaving the message history it encrypts on semi-public storage would have been a
half-fix.

Deriving rather than asking Android is deliberate. Asking needs a `Context`, a `Context` needs a
realized toplevel, and sessions are restored before there is a window. If the derivation ever stops
matching, `crate::utils` panics rather than falling back — a fallback means silently writing
secrets to external storage again, which is the failure this exists to prevent.

**What is in the file.** AES-256-GCM under a key generated in the Android Keystore. The key is
never handed out — that is the whole point of the Keystore — so it cannot be copied off the device
and cannot be used except as this application's UID. Which is also why the encryption happens on
the Java side through JNI: there is no key material to give a Rust cipher. GCM authenticates as
well as encrypts, so a tampered file fails to decrypt rather than decrypting to something else, and
the Keystore chooses the IV, refusing a caller-supplied one.

`src/utils/android.rs` captures the `JavaVM` once, on the GTK thread, because a `JNIEnv` belongs to
its thread and the secret store runs on a tokio worker through `spawn_tokio!`. It is a separate
module from the secret store because S5's notifications will want the same thing. The way in is
`gdk_android_display_get_env()`, public API since GTK 4.18 and declared by hand, since the gtk-rs
bindings do not cover the Android backend.

Measured on the emulator with a round trip that was removed before committing:

| Test | Result |
| --- | --- |
| `DataType::Persistent` | `/data/user/0/io.github.steeb_k.commune/files/commune` — internal |
| `DataType::Cache` | `/data/user/0/io.github.steeb_k.commune/cache/commune` — internal |
| Keystore round trip | 27 bytes sealed to 57 and back, byte-identical |
| The system Keystore really did it | `keystore2` appears in logcat during the call |

57 is the arithmetic working out: 1 version byte, 1 IV length, the 12-byte GCM IV, 27 of plaintext
and the 16-byte GCM tag.

### The IME, and why no keyboard appeared

Tapping a text field produced no keyboard. `adb shell input text` had made this look like it
worked, and it does not: that injects key events and bypasses the IME entirely, so it proves the
widget accepts text and nothing about the path a person uses.

There were two unrelated causes, and separating them mattered because only one of them is ours.

#### GTK tells the IME the field is not a text field

`EditorInfo` arrived as `inputType=0, inputTypeString=NULL` — `InputType.TYPE_NULL`, which means
"this connection does not take composed text, send hard key events instead". Gboard honours that by
drawing nothing. `ImeTracker` still reports `onShown`, and `dumpsys input_method` still says
`mInputShown=true`, so everything claims to be working and the screen stays empty.

The cause is in GTK, not here — `ToplevelActivity.onCreateInputConnection`:

```java
//outAttrs.inputType = GlibContext.blockForMain(() -> activeImContext.getInputType());
outAttrs.inputType = InputType.TYPE_NULL;
```

The real implementation is commented out immediately above the hardcoded value. Everything behind
it is finished: `ImContext.java` declares `public native int getInputType()`,
`gtk/gtkimcontextandroid.c` implements it as `_gtk_im_context_android_get_input_type` — mapping
every `GtkInputPurpose` onto the matching Android constants, password and PIN included — and
registers it in `im_context_natives[]`. Only the call site is disabled.

`build-aux/android/patch-gtk-ime.sh` restores it, next to `patch-manifest.sh` and for the same
reason: pixiewood copies the glue's Java into the Gradle project on every `generate`. With the line
back, `EditorInfo` becomes `inputType=1, inputTypeString=Normal` with autocorrect and learning on,
and Gboard attaches. The script refuses to run rather than silently doing nothing if the text it
expects has changed, so a GTK update that fixes this upstream is noticed rather than papered over.

Confirmed GTK-wide, not ours: gtk4-demo produces exactly the same `inputType=0` on the same device.
That also retires S0's note that the demo's keyboard worked — it does not, on current GTK.

#### The emulator is in physical-keyboard mode

With the above fixed, Gboard attaches but draws a collapsed floating strip — backspace, enter,
emoji, and a handle — rather than a keyboard. Its menu gives it away: **"Show on-screen keyboard —
Alt K"**, which is Gboard's _physical keyboard_ toolbar. `dumpsys input` shows a device with
`KeyboardType: 2` (alphabetic), so Android believes a hardware keyboard is attached and Gboard
collapses accordingly. Tapping that entry gives the full QWERTY.

**This is not GTK's doing and not Commune's.** The stock Android Settings app, on the same
emulator, shows the identical floating strip. It is the emulator presenting an alphabetic keyboard
device, and it affects every application on the device.

`settings put secure show_ime_with_hard_keyboard 1` does not help; it was already `1`.

Two consequences worth knowing before testing input by hand:

* A **host keyboard does not type into this AVD** — `hw.keyboard=no` in `config.ini` means host
  keystrokes are never delivered to the guest, while a virtual alphabetic device still exists to
  confuse Gboard. Setting `hw.keyboard=yes` makes physical typing work; the floating toolbar stays,
  which is then the correct behaviour rather than a bug.
* Otherwise, reach the keyboard through the handle → **Show on-screen keyboard**.

Neither is evidence about a real phone, which has no keyboard device and gets a normal keyboard
from the `inputType` fix alone — confirmed on the Pixel 9a on 24 August 2026.

**This does not make the emulator useless for keyboard work, which is what it was read as for most
of this port.** Once the full QWERTY is up, it behaves like a keyboard: `input swipe` drives
Gboard's space-bar gesture, `input tap` on the key positions types through the `InputConnection`,
and the whole loop is scriptable. Every input question S8 and S9 answered was answered here, with
the phone switched off. See [S9](#s9--the-space-bar-and-the-reset-that-cancelled-it) for the rig,
and for the one rule that makes it valid: `adb shell input text` is **not** typing, because it
never goes through the `InputConnection`.

### The TLS that never returned

Discovery hung. Entering a homeserver and pressing **Next** left the button spinning with nothing
in logcat at all — no error, no warning, no request.

The panic hook installed for the Keystore work is what broke it open. `lib.rs` wraps the default
hook and sends the message through `tracing`, so what had been silence became one line:

```text
panicked at rustls-platform-verifier-0.7.0/src/android.rs:90:10:
Expect rustls-platform-verifier to be initialized
```

That crate panics rather than returning an error when it has not been given a JVM handle and a
`Context`, and it does so inside whichever tokio task was making the request. The task dies, its
`JoinHandle` never resolves, and the caller waits forever. A hang was the only symptom a panic
could have had here.

The plan had this one coming: it lists "the `rustls-platform-verifier`/`ndk-context` JVM-context
dance" among the things S3 would have to settle. What the plan did not anticipate was that
settling it properly is not currently possible.

**Why it cannot simply be initialized.** `reqwest`'s `rustls` feature enables the platform verifier
unconditionally — there is no root-store feature to select instead — and verification calls into a
Kotlin class, `org.rustls.platformverifier.CertificateVerifier`, which has to be built into the
APK. That component is [not on Maven][gh115], and it is not in the published crate either: 0.7.0
ships `src/`, `examples/` and licences and nothing else. Using it means vendoring the component out
of the crate's git repository and teaching pixiewood's generated Gradle project to find it.

[gh115]: https://github.com/rustls/rustls-platform-verifier/issues/115

**What was done instead.** `src/utils/tls.rs` configures the TLS itself. Android keeps its system
trust store as ordinary PEM files under `/system/etc/security/cacerts`, readable by any application
— 145 of them on the test device — so the roots are read from there and handed to `rustls`. The
verifier is still linked and is simply never asked anything. Two call sites route through it:
`utils::http::CLIENT` for the non-Matrix fetches, and `LoginHomeserverPage::client_builder`, which
now passes `.http_client(…)` so that `matrix-sdk` does not build its own.

This was a choice between imperfect options and it is worth being honest about which parts are
worse. The roots are the device's own, so they follow system updates rather than the build date —
that is the whole of what it buys. Against that: `rustls` does the verifying rather than Android,
so certificate transparency policy, operator-configured pinning and the per-app network security
config do not apply, though `rustls`'s own path building and hostname checking still do. And
user-installed CAs are invisible, because they live in `/data/misc/user/0/cacerts-added` where an
application cannot read them. That last one matches Android's default since Nougat rather than
departing from it, but it does mean **an intercepting proxy will not work**, which is worth knowing
before anyone tries a self-signed local homeserver. The real verifier remains the right end state.

One trap cost a build cycle and now carries a comment: `reqwest` downcasts what
`use_preconfigured_tls` is given to `Option<rustls::ClientConfig>` _exactly_, so handing it an
`Arc<ClientConfig>` is refused at runtime with "Unknown TLS backend passed to
`use_preconfigured_tls`". The static holds a bare config and callers clone it, which is cheap
because the expensive parts inside are already refcounted.

### Logging in works

With the TLS fixed, a password login against a homeserver completes and the session opens.

That is a larger result than it sounds, because it is the first thing to exercise the whole stack
at once rather than one piece of it. `matrix-sdk` ran, which means the bundled SQLite store
created its database and the crypto stack generated and persisted its keys; the Keystore-sealed
secret store wrote and read back a real session; the trust roots verified a real certificate chain;
and the text that started it was typed into an `AdwEntryRow` through the patched IME. Every S3
subsystem is on that path.

`matrix.org` itself does not get that far, and should not be expected to: it authenticates through
OAuth 2.0, so `discover_login_api` finds server metadata and hands off to the browser flow. That
needs the redirect the plan has always listed as unimplemented — see
[OAuth 2.0 / SSO login on Android](#oauth-20--sso-login-on-android). Discovery against it does
succeed, which is what proves the TLS fix; the flow then stops where the plan said it would.

### OAuth 2.0 / SSO login on Android

The redirect [Logging in works](#logging-in-works) stopped at is built now, and measured against
the real thing: `matrix.org`.

**Three separate breaks, not one.** The plan's own note — "it needs an intent-filter and a custom
scheme" — undersold it once the pieces were built and tried against a live server.

1. **The browser never opened.** `gtk::UriLauncher` has no Android backend at all —
   `gtkurilauncher.c` falls through to `gtk_show_uri_full`, which asks GIO's app-info registry for a
   handler, and that registry is empty on Android. `src/utils/android.rs::launch_uri()` builds the
   `ACTION_VIEW` `Intent` by hand over JNI and launches it through
   `gdk_android_toplevel_launch_activity`, the same entry point `gtk::FileLauncher` already used for
   its own Android intents (`gtkfilelauncher.c:511`).
2. **The app id's own redirect scheme doesn't parse.** A URI scheme is `ALPHA *( ALPHA / DIGIT / "+"
   / "-" / "." )` (RFC 3986 §3.1); `io.github.steeb_k.Commune`'s underscore is not in that set, and
   `url` confirms it by refusing to parse `io.github.steeb_k.commune:/…`. The redirect URI is
   `io.github.steeb-k.commune:/oauth2redirect` instead — the real domain the app id is derived from,
   `steeb-k.github.io`, which has no underscore to begin with.
3. **A redirect into a running Commune was dropped.** `patch-manifest.sh` sets
   `launchMode="singleTask"` (see [above](#the-manifest-patch-which-is-done)), so a second launch
   resumes the running Activity through `onNewIntent` rather than `onCreate` — and
   `ToplevelActivity` had no `onNewIntent` at all, so the redirect's `Intent` arrived and was never
   read. `build-aux/android/patch-gtk-intent.sh` adds it, doing what `onCreate` already does with an
   incoming `Intent`.

None of these three is reachable in isolation from `local_server.rs` alone — each one hides the
next, so each was only found by fixing the one before it and trying again against a real server.

**What matrix.org's server added on top.** With all three fixed, dynamic client registration
against `matrix.org` still failed: `invalid_redirect_uri`. matrix-authentication-service's
`client_registration.rego` policy requires that, for a native client's non-`https` redirect URI, the
registered `client_uri`'s host — read as reverse-DNS labels — be a _prefix_ of the redirect scheme's
own dot-separated labels. `github.com` reversed is `com.github`, not a prefix of
`io.github.steeb-k.commune`; `client_uri` is `https://steeb-k.github.io/` on Android for exactly
this reason — the same domain the scheme already reverses to.

**Confirmed against `matrix.org`, live, on the emulator, with a real account.** Commune launches
Chrome with the real authorization URL, matrix.org's client registration accepts it,
`account.matrix.org/login` renders in full, and — completed by hand, not by this automation, since
it needed a real account — the login itself completes: the redirect reaches `onNewIntent` →
`Application::process_uri` → `android::deliver_oauth_redirect`, and the session opens. Every part
of the chain this section describes is now measured, not predicted. SSO login on Android works.

### What has not been exercised

Running the app answers some of what S3 left open and not others.

**Answered.** The extracted assets land exactly where `app_bundle.rs` looks — logcat prints
`Datadir: /data/user/0/…/files/share/commune` — and `gio::Resource::load` did not fail, so
`XDG_DATA_DIRS` was the right choice. Text entry reaches Commune's own logic.

**Also answered, later.** A password login against a homeserver now completes — see
[Logging in works](#logging-in-works) — so `matrix-sdk`, the SQLite store and the crypto stack have
all run.

**Still open.** `glib::user_cache_dir()` is still untested and still looks wrong — the glue sets no
cache directory at all.

**Also answered, later still.** The soft keyboard's behaviour with a real `GtkTextView` composer,
S0's oldest open question, is no longer open — see
[The soft keyboard did not hide itself](#known-gaps) in Known gaps. `BACK` hides it correctly with
a real session and a real composer.

**Translations are missing.** `files/share` has no `locale` directory, so `bindtextdomain` points
at nothing and the app is English-only. Same cause as the schema — an untagged install — but in
`po/meson.build`, where `i18n.gettext()` does the installing and does not obviously take a tag.
Cosmetic for a spike, and it should be fixed before anyone sees it.

## S5 — Notifications

Notifications on Android were not broken. They were absent, and they said so to nobody.

`GNotification` is the whole of the notification code on every other platform:
`Notifications::send_notification()` fills in a `gio::Notification` and hands it to
`g_application_send_notification()`, which finds a backend for it. GIO ships three backends —
`gtk`, `freedesktop` and `cocoa` — and the first two are D-Bus, which no Android application can
reach. There is no fourth. So the call there is not a failure that could be logged, it is a
**silent no-op**: every notification Commune has ever raised on Android went nowhere, quietly, and
nothing in the code or the logs would ever have said so.

`src/utils/android_notifications.rs` is the replacement, and it is deliberately the same shape as
`src/utils/macos_notifications.rs` — an `init`/`send`/`withdraw` triple behind the `cfg_if!` in
`src/session/notifications/mod.rs` — because macOS is the other platform whose notification backend
GIO cannot serve, and the seam was already cut to that shape.

### Four things the platform demands

**A channel, or nothing is shown.** Since API 26 a notification whose channel does not exist is
dropped by the system without a word — the same failure mode as having no backend, arrived at
differently. The channel also owns the importance, the sound and whether the notification may
interrupt, and the user can retune all of it in system settings afterwards; an application that
created its channel with a low importance cannot talk its way back up later. So it is created once,
at `init()`, with `IMPORTANCE_HIGH`, which is what `im.received` and `NotificationPriority::High`
already asked for elsewhere. The channel ID is `im.received` too, for want of a reason to invent a
second name for the same thing.

**Permission, at runtime.** `POST_NOTIFICATIONS` arrived in API 33 and this build targets 36, so
the manifest entry `patch-manifest.sh` now adds earns only the right to ask. `init()` asks and does
not wait: the answer arrives at `Activity.onRequestPermissionsResult`, which would mean a fourth
patch to GTK's Java glue for something `checkSelfPermission()` can be asked at any later moment
anyway. The request needs the `Activity` rather than the application `Context` — the application
`Context` can be asked whether permission is held, but cannot ask for it — which is why `init()`
takes the window and is called from `Application::present_main_window()` rather than from
`lib.rs` next to `android::init()`. Until there is a window there is no `Activity`, and until
there is an `Activity` there is no `Context` at all.

**A tap has to survive the process.** What a notification carries is a `GAction` name and a
`GVariant` target, and by the time it is tapped the process that posted it may be long dead — on
Android that is the ordinary case, not the edge one. So none of it travels in memory. It is written
into the `Intent`'s URI under the same custom scheme SSO login already registered, as
`io.github.steeb-k.commune:/notification?action=…&target=…`, and read back in
`Application::process_uri()`.

**The icon is bytes, not a file.** macOS wants a file URL, so `macos_notifications` writes the
avatar into the cache and cleans up after itself at startup. `BitmapFactory.decodeByteArray` takes
the PNG directly out of `gdk::Texture::save_to_png_bytes()`, so nothing here touches the disk and
there is nothing to clean up.

### The delivery path was already built

The tap path cost almost nothing, and that is the point worth recording: the `intent-filter` in
`patch-manifest.sh` and the `onNewIntent` override in `patch-gtk-intent.sh` were both built for the
OAuth 2.0 redirect, and a notification tap needs exactly the same door. A `PendingIntent` naming
`ToplevelActivity` explicitly — an implicit `ACTION_VIEW` would be offered to every application on
the device — lands in `onNewIntent`, is handed to `GdkContext.open()`, and arrives at
`Application::process_uri()` as a URI. Which of the two arrivals it is, is a prefix test.

Two things about that path had to be checked rather than assumed.

`g_uri_parse_params()` is one of the few GLib functions gtk-rs does not bind: the generator gives up
on its `GHashTable` return and leaves the binding commented out (`glib-0.22.8/src/auto/uri.rs:291`).
This was found by the compiler, after being written against it. The query is taken apart by hand in
`android_notifications::tapped_action()` instead, next to the `tap_uri()` that builds it, so the two
halves of one format sit together.

The escaping has to survive a round trip through `GFile`, not just the wire. `GApplication::open()`
hands a URI over as a `GFile`, and for an unknown scheme GIO decodes it into a `GDummyFile` and
re-encodes it on the way out. Printed `GVariant` text is full of what a query cannot carry
literally — quotes, commas, spaces, colons, an ampersand in a room name — so both halves are
escaped with `g_uri_escape_string()` and unescaped again on the far side. Measured, not assumed:
see below.

### What was measured

On the emulator, 24 August 2026, against a real `matrix.org` account.

**The permission prompt is real.** Android showed "Allow Commune to send you notifications?" —
so the manifest entry took and the request reached the system with the right application name.
Granting it flips `dumpsys package` to `POST_NOTIFICATIONS: granted=true`.

**The channel exists**, exactly as asked for:

```text
NotificationChannel{mId='im.received', mName=Messages, mImportance=4, …}
```

**The tap path was proved before there was anything to tap**, with synthetic `Intent`s. A
deliberately broken target — `am start -a android.intent.action.VIEW -d
'io.github.steeb-k.commune:/notification?action=app.show-matrix-id&target=notavariant'` — produced
`Could not read the target of a tapped notification: 0-11:unknown keyword`, which proves the whole
chain from `onNewIntent` to the parser runs. A valid one, escaped as `tap_uri()` escapes it, came
back intact through `GFile` and activated the action with the payload it was given:

```text
Could not find session to process intent session="test-session"
  intent=ShowMatrixId(Room(MatrixRoomIdUri { id: "#test:example.org", via: [] }))
```

Quotes, comma, space, colons and slash all survived. That was the part of this least certain in
advance and it is now the part least in doubt.

**A real message posts a real notification.** A direct message from another account, with Commune
foregrounded on the room list:

```text
tag=mtbcuwGu//matrix:roomid/…/e/…   id=0
channel=im.received  importance=4  category=msg  flags=AUTO_CANCEL
icon=Icon(typ=RESOURCE pkg=io.github.steeb_k.commune id=0x7f020001)
android.title="steeb"  android.text="Hello."
android.largeIcon=Icon(typ=BITMAP size=96x96)
contentIntent=PendingIntent{… startActivity}
isNoisy=true
```

Every part of that is load-bearing. The small icon is a resource of ours, not the
`android.R.drawable.stat_notify_chat` fallback, so `getIdentifier()` found a drawable of ours —
which is the right shape for a status bar icon precisely because it is a silhouette, since Android
masks a small icon down to its alpha channel and tints it.

That lookup originally named `ic_launcher_monochrome`, and the shape was the only thing it got
right. **It is the wrong size, and S5 did not catch it** — `dumpsys` reports that a small icon
exists, not that it is legible. The drawable is the launcher's monochrome layer, so pixiewood bakes
in the adaptive-icon inset from `scale=".45"` in the manifest: correct for the launcher, where 45%
of a 108dp canvas keeps the glyph inside the 66dp safe zone, and wrong for a small icon, which is
drawn inside the shade's own badge and is expected to carry about 22dp of content in 24dp. The
glyph landed at roughly 40% of its slot, about half the diameter of every other notification's icon
in the same shade. `build-aux/android/patch-notification-icon.sh` strips the inset into an
`ic_notification` of its own and `SMALL_ICON_NAME` names that instead; the script's header carries
the detail. The large icon decoded to a 96×96 `Bitmap`, so the PNG bytes made
it across JNI. The tag is the ID `GApplication::send_notification()` would have been given, with
the `int` left at zero, which is what keeps replace-by-ID and makes `withdraw_notification()`
addressable. Nothing from our own code appears in logcat: no avatar that failed to decode, no
missing drawable, no JNI exception.

**Tapping it opened the conversation**, and `AUTO_CANCEL` removed the notification — the record is
gone from `dumpsys notification` afterwards.

### Syncing while Commune is not on screen

Everything above only matters while the app is being looked at, because Android freezes an
application's process as soon as none of it is visible. A message that arrives then is not late; it
is not received at all, since the sync loop is a tokio task in this process and a frozen process
runs nothing.

Of the three ways out, the foreground service is the one that can be measured here: it needs no
push gateway, no distributor application and no co-operation from the homeserver, so it works
against any server, including one that has never heard of us. UnifiedPush is better for battery and
worse for moving parts, and FCM needs Play services, a Firebase project and Gradle changes
pixiewood cannot express. Both remain possible on top of this — `android_notifications` takes no
position on where a notification came from — which is the point of doing this one first.

**The service does nothing**, and that is not laziness. The sync loop is already running in this
process; all a foreground service has to do to keep it running is exist. So `SyncService.java`
holds no state and starts no work, and every decision about when it should exist is in
`utils::android_sync_service`: started when a session appears, stopped when the last one goes. Both
callers are moments when Commune is on screen, which is required — since API 31 a background
process calling `startForegroundService()` gets `ForegroundServiceStartNotAllowedException`. Not a
real constraint, since a frozen Commune cannot call anything, but it does decide where the call
goes.

**Where the Java had to live is decided by pixiewood, not by us.** It symlinks exactly one
directory into the Gradle project — `org/gtk/android` out of `java-sources`, GTK's own glue
(`pixiewood:715-717`) — and offers an application no way to add a package of its own.
`java-sources` cannot be redirected either, because only that one subdirectory is linked, so
pointing it at a directory of ours would mean vendoring GTK's five glue classes and keeping them in
step by hand. So `SyncService.java` is copied in beside the glue by a fourth patch script,
`patch-gtk-service.sh`, and its package is a consequence of that layout rather than a claim about
whose code it is.

Its user-visible text travels as `Intent` extras rather than as Java resources, because the
translations are on the Rust side: `gettext` runs against Commune's own catalogues and Java has no
way to reach them.

**Measured.** The service starts with the session — `isForeground=true foregroundId=1
types=0x00000001`, which is `FOREGROUND_SERVICE_TYPE_DATA_SYNC` — on a `sync` channel of its own at
`IMPORTANCE_LOW`, so the ongoing notification is silent and can be muted without muting messages. A
direct message sent while the app was backgrounded, in a window bookended by `TO_BACK` and
`TO_FRONT` transitions fifty-two seconds apart, arrived and posted a notification. Before this it
would not have arrived at all.

**The six-hour cap is the reason push is still the answer.** Android 15 gives a `dataSync`
foreground service six hours in any twenty-four, then calls `onTimeout()`, and a service still
running when the grace period ends is killed with an ANR. `SyncService` stops itself there, and
Commune goes back to receiving only while on screen until it is next opened. So this buys most of a
day, not a permanent connection. UnifiedPush is not an optimisation of it; it is the thing this is
standing in for.

### The directory GTK empties, which cost a session twice

Testing the service turned up something older and worse than anything it changed.

Persistent data was under `Context.getFilesDir()`, which S3 chose over external storage and wrote
down as settled. That directory belongs to GTK's glue.
`SystemFilesystem.writeResources()` compares a fingerprint in the APK against the copy on disk and,
when they differ, calls `doWriteResources()` — which runs `cleanDirectory(getFilesDir())` before
extracting, and `cleanDirectory` recurses and deletes everything it finds.

The fingerprint differs on every build. So **every install of a new build wiped the secret store,
the SDK's databases and the message history, and logged the account out**, with nothing in the log
to say so. It presents as an app that has forgotten who you are, and it was read twice as a session
that failed to restore before the cause was found — once at the start of this work, once after the
notification build was installed.

`getNoBackupFilesDir()` — `<data>/no_backup`, a sibling of `files` and `cache` — is never looked at
by the glue, so the fix is to be out of its way rather than to race it. It is also the right
directory on its own merits: the databases are sealed with a Keystore key that cannot leave the
device, `patch-manifest.sh` already forces `allowBackup="false"` for that reason, and this is what
Android provides for data that must not be backed up. The cache stays at `getCacheDir()`, which the
glue does not touch. There is nothing to migrate, because anything at the old path was destroyed by
the build that would have carried the migration.

**Measured, rather than argued from where the directories sit.** A rebuild produced an APK whose
`assets/afpr` differs from the copy on the device, which is what makes the glue take the destructive
path rather than skip it — so this exercised the wipe, not a build that happened not to trigger it.
After installing and launching: everything under `files` carries the launch time, `afpr` included,
so `cleanDirectory` ran; `no_backup/commune` and the `secrets.d/<id>.sealed` inside it still carry
their original timestamps; and the app came back to its room list with the foreground service
running, which only happens once a session has been restored — so the Keystore key still decrypted
the sealed file across a reinstall. Repeated once more, with the same result.

The general lesson is worth more than the fix: **`getFilesDir()` is GTK's on this port, not
Commune's.** Anything of ours that is put there is on borrowed time.

### The icon theme, which is a wrap and two patches

Android is the only platform Commune runs on with no icon theme of its own, and nothing said so:
an icon simply resolved or drew as the missing-icon placeholder. See the Known gaps entry for how
that presented and which eleven names were affected.

`adwaita-icon-theme` is an unusually easy thing to cross-build, because there is nothing to build.
It is data — no compiler, no library, nothing to link — and every one of its `install_*` calls
already carries `install_tag : 'runtime'`, which is exactly what pixiewood's
`meson install --tags runtime` needs to not drop it the way it dropped the translations. It
installs to `datadir/icons/Adwaita`, and the glue points `XDG_DATA_DIRS` at `<files>/share`, so it
lands where GTK already looks with nothing to wire up.

The one structural wrinkle is that **nothing depends on it**, so nothing pulls it in. A
`dependency()` would have nothing to ask for. `meson.build` calls `subproject()` on it directly,
under `is_android`.

Two patches, in `subprojects/packagefiles/adwaita-icon-theme-android.patch`, both for things that
only appear in a cross build:

* **Cursors.** The project installs them unconditionally on anything that is not Windows: 16 MB of
  the 19 MB it installs, plus fifty-odd `install_symlink`s that would then have to survive APK
  asset packing. Android draws no pointer and has no X11 cursor names, so none of it has a reader.
  The patch adds an `android` branch that installs none.
* **The icon cache.** `find_program('gtk4-update-icon-cache')` finds the **cross-built** one in
  `subprojects/gtk` — an Android binary — and meson refuses to run it on the build host:
  `An exe_wrapper is needed for … gtk4-update-icon-cache`. That was the first failure the build
  hit. Skipped on Android, at no real cost: GTK scans the theme directory when there is no cache,
  and the install script was already `skip_if_destdir`.

**2.8 MB installed, 91 KB in the APK**, because SVGs compress. Measured on the emulator:
`files/share/icons/Adwaita` is extracted on the device with `index.theme`, `symbolic`, `scalable`
and `16x16` and no `cursors`, and `view-more-horizontal-symbolic` renders in the quick reaction
popover — an icon in neither Commune's set nor GTK's, so nothing else could be drawing it.

One correction to the Known gaps entry while it was being checked: the warning triangle beside
"Could not decrypt this message" is `warning-symbolic`, which Commune ships and which was always
drawing correctly. Only the "Back to Latest" button was the placeholder.

### What this does not do

**Notification buttons are written but unexercised.** `Notification.Action` is built for the
`buttons` argument, and the only caller that passes any is the call notification, which is
`cfg`-gated out on Android along with the rest of GStreamer. So that arm compiles and has never
run.

**The channel name is untranslated**, like everything else here — see
[Translations are missing](#what-has-not-been-exercised).

## S5b — UnifiedPush: the endpoint reaches Rust

Real push, planned in `doc/android-push-plan.md` after the six-hour cap made the foreground
service a stand-in. Step 0 — the whole server chain proven with `curl` and a throwaway Synapse,
the spec pinned at `AND_3.1.0`, and the discovery that the glue runs `main` on _every_ process
start — is written up in the plan; this section is what happened on the device.

**Step 1 is done, 26 August 2026, on the emulator.** The pieces: `PushReceiver.java` beside the
glue (`patch-gtk-receiver.sh`, the eighth patch script), a `<receiver>` for the five connector
actions and a `<queries>` entry for the `unifiedpush://link` activity in `patch-manifest.sh`,
`src/utils/android_push.rs` for discovery, registration and the receiving end, and
`android::seed_from_jni()` so a JNI entry can capture the VM and `Context` in a process where GTK
never opened a display. Registration state — token, distributor, endpoint — is a keyfile under
`DataType::Persistent`, which is `no_backup`: the token is the registration's identity and has to
outlive the process, and the broadcasts it validates arrive in processes started long after the
one that registered.

Two mechanics are worth recording:

* **The JVM does not see `g_module_open`.** Native methods resolve only through libraries loaded
  with `System.loadLibrary`, so the receiver loads `libcommune.so` once more by name — dlopen
  reference-counts, so that is bookkeeping, not a second copy — and `stub.c` takes the address of
  `Java_org_gtk_android_PushReceiver_nativeReceive`, because Meson links the staticlib as a plain
  archive and an object nothing references is dropped.
* **A never-opened distributor does not exist, as far as broadcasts go.** ntfy installed fresh
  from F-Droid received nothing: a package that has never been launched is in the stopped state,
  and broadcasts are silently not delivered to it. The identical registration after opening ntfy
  once was answered in 178 ms. The fix is `FLAG_INCLUDE_STOPPED_PACKAGES` on the `REGISTER`
  intent — being installed is what makes it the user's distributor; having been opened should not
  be part of the contract. **Measured**: with ntfy force-stopped (which re-enters the stopped
  state) and no process, the flagged `REGISTER` started ntfy's process and was answered.

That last verification measured two more things at once, because it ran after a reinstall of
Commune: ntfy logged _"Package name retrieved with shared identity"_ — the API 34
`FLAG_SHARE_IDENTITY` path works, the legacy `application` extra was not needed — and, the token
having survived under `no_backup`, ntfy matched its existing subscription and re-announced **the
same endpoint** instead of minting a second registration. That is the plan's
"survives a reinstall via `NEW_ENDPOINT`" measurement, made exactly as hoped.

### Step 2 — the endpoint becomes a pusher

**Done, 26 August 2026, on the emulator.** When a session reaches `Ready` — and for every session
when the endpoint changes — `ensure_pusher()` reconciles the account with the state file: list
the pushers, remove the one for an endpoint this device has moved off, and if the current
endpoint is not among them, probe its server for the Matrix gateway
(`/_matrix/push/v1/notify` answering `{"unifiedpush":{"gateway":"matrix"}}`, through
`utils::http` — which matters on Android, where reqwest's stock TLS panics) and register it with
`format: event_id_only`. On logout the pusher is removed **before** `client.logout()`, because a
pusher belongs to the account rather than the device, and after logout there is no token left to
remove it with.

Three judgement calls worth recording:

* **Only pushkeys this device has held are ever deleted.** The `app_id`
  (`io.github.steeb_k.commune.android`) is the same for every Commune on Android, so "everything
  under our `app_id`" on the homeserver includes the user's other phones. The state file carries
  `previous_endpoint` for exactly as long as its pusher may still need removing.
* **No gateway, no pusher.** An endpoint whose server does not answer the probe is not registered
  at all — the alternative is routing notification metadata through a third-party gateway the
  user never chose. The setup screen (step 5) is where that refusal becomes visible.
* **The two callers race, so they take turns.** The endpoint arriving and the session becoming
  ready happen within milliseconds on a fresh registration, and both saw "no pusher" — measured:
  _"Registered the push endpoint as a pusher"_, twice, 23 ms apart. The homeserver treats the
  second as an upsert, so nothing broke; a mutex now makes the loser re-read and find the work
  done.

**Measured:** against the emulator session's real homeserver, the probe passed and the pusher
registered on the first launch that had an endpoint, and a relaunch — including the `NEW_ENDPOINT`
fan-out re-running the reconciliation — registers nothing again and warns about nothing. What
step 2 deliberately does not claim: that a real message now pushes a frozen Commune end-to-end —
that needs a second account sending a real message, and it is step 3's headline measurement, the
S5 methodology repeated with the process dead.

One scheduling fact from the same session, not a bug of ours but worth knowing when a measurement
looks like a failure: right after a device boot, the broadcast queue deferred the `REGISTER` by a
full hundred seconds between enqueue and dispatch (`dumpsys activity broadcasts history` shows
both timestamps). A `NEW_ENDPOINT` that seems to never come may merely not have been sent yet.

**Measured, all against ntfy 1.25.2 from F-Droid on the emulator:** discovery finds
`["io.heckel.ntfy"]` through the `unifiedpush://link` query; the endpoint —
`https://ntfy.sh/upeOjmM0pSDm0r?up=1`, the `up` + 12 naming the plan documents — arrives at
`nativeReceive` and is logged from Rust; and a synthetic Matrix notify POSTed at ntfy's gateway
reaches the running application as a `MESSAGE` broadcast about five seconds later, most of it
ntfy's own delivery latency.

### The push at a dead process, measured a step early

`am stop-app` (which kills without the stopped state that `am force-stop` would set), then the
same gateway POST. Everything below is one logcat:

* ntfy held the message and sent the broadcast; Android logged
  `Start proc … for broadcast {…PushReceiver}` — the process exists again because of us.
* `RuntimeApplication.onCreate` ran `main`, exactly as step 0 read in the source: GStreamer
  registered its plugins, the application constructed itself, and **no `Activity` appeared and
  nothing crashed** — `activate` never fired, so nothing tried to present a window.
* `nativeReceive` logged the message **771 ms after process start**.
* **Session restore runs without a window.** `Restoring previous session … @fakeguy` — the
  Keystore unseals from a background process, and the ordinary sync loop started.
* That restore reached `android_sync_service::update()`, and the API 31 exception the module had
  written off as unreachable was thrown for real — `Background started FGS: Disallowed` — and
  absorbed by the error arm built for other failures. The module comment now tells the truth.
* **The freezer closed the window ten seconds in.** `ActivityManager: freezing` came 10.6 s after
  process start, while the sync loop's 30-second long-poll was still in flight. That is the
  budget: the accidental full-app wake fits a session restore but not a classic sync, which is
  the measured version of why step 3 plans on `NotificationClient` fetching one event rather
  than syncing.
* One emulator artifact worth not chasing later: the woken process's syncs failed with DNS errors
  against a homeserver the same emulator resolves when foregrounded. Noted, unexplained, and to be
  retested on hardware before it is believed.

## S6 — The formats that would not draw

Two lines in logcat, from an ordinary scroll through a real account:

```text
commune::utils::media::image: Could not decode image: unsupported image format
commune::session_view::room_history::message_row::url_preview::imp: Could not load the image of a
  URL preview: Image format not supported
```

`src/utils/media/image/decoder/` is the seam this port shares with macOS and Windows. Linux decodes
with glycin, which is a set of C libraries plus D-Bus and Flatpak machinery that exists nowhere
else; everywhere else decoding is the pure-Rust `image` crate, which reads BMP, GIF, ICO, JPEG,
PNG, TIFF and WebP and has never read SVG, HEIC, AVIF or JXL.

### What was already in the APK, and what the ledger got wrong

The entry in [Known gaps](#known-gaps) used to predict that porting the Windows fix would not be
enough on its own — that HEIC and AVIF would need `libheif` and `libavif` cross-built and wrapped,
since pixiewood's dependency list only names `rsvg`. That was wrong twice over, and both
corrections are the same shape: **the thing was already linked, and nobody had looked.**

**The codecs.** gdk-pixbuf builds a family of `android` loaders whenever it finds `jnigraphics`,
and this build finds it. `subprojects/gdk-pixbuf/gdk-pixbuf/meson.build:200` lists them:

```meson
android_loader_formats = [ 'jpeg', 'png', 'gif', 'webp', 'bmp', 'ico', 'wbmp', 'heif' ]
```

Every one is a shim over the NDK's `AImageDecoder` (`io-android-utils.c` includes
`<android/imagedecoder.h>`), and they are static libraries linked into the shared object rather
than modules to be found at runtime — `libstaticpixbufloader-android.a` is built beside
`libgdk_pixbuf-2.0.so`, and `strings` on the shipped library finds `image/heic` and `image/webp`.
So the platform's own codecs have been in the APK since the first build, reachable through a
library GTK already links.

`AImageDecoder` also means **AVIF comes free** on API 31 and up. The loader never inspects the
format; it hands the bytes to Android, which works it out. A file the "heic" loader accepts is
whatever Android can decode, not whatever the loader is named after.

**The renderer.** `nm -D` on this build's `libgtk-4.so` finds 38 exported `gtk_svg_*` symbols. GTK
grew a public SVG renderer of its own in 4.22 — `GtkSvg`, a `GdkPaintable` and
`GtkSymbolicPaintable` implementing much of SVG 2, animations included — and it is what draws the
symbolic icons this port already fixed. librsvg is not needed for SVG here and never was; see
[SVG, which needed no new library](#svg-which-needed-no-new-library).

### What was measured

Three images built for the purpose, each labelled with its own format so that which decoder ran is
readable off the screen rather than inferred, pushed to `/sdcard/Download` and sent into a room on
the emulator:

| File | Path taken | Result |
| --- | --- | --- |
| `test-heic.heic` | `image` crate declines → pixbuf sniff declines → `image/heic` by name → `AImageDecoder` | draws |
| `test-avif.avif` | same, and the loader that answers is still the HEIF one | draws |
| `test-svg.svg` | sniffed as SVG before the blocking-pool hop → `GtkSvg` → cairo → `RawFrame` | draws |

All three had been "Image format not supported" before. The AVIF is the one worth noticing: nothing
in the APK registers `image/avif`, and it decodes anyway, because the loader that took it never
looks at the format.

### The loader that cannot be sniffed for

The Windows fallback (`0454830b`) hands anything the `image` crate does not recognise to GdkPixbuf
via `gdk_pixbuf_new_from_stream`, which picks a loader by **sniffing the bytes**. Cherry-picked
here as `f745e2bb` — the decoder file was byte-identical on both branches, so it applied exactly
and the two ports carry one seam rather than two spellings of it.

On its own it changed nothing on Android, and the reason is worth writing down because it is
invisible from the Rust side. `io-android-heif.c` declares:

```c
static const GdkPixbufModulePattern signature[] = {
  { NULL, NULL, 0 }
};
```

An empty signature array. `format_check` (`gdk-pixbuf-io.c:115`) walks it with
`for (pattern = module->info->signature; pattern->prefix; pattern++)` — zero iterations, no score,
never chosen, no matter what the bytes say. It is not an oversight: the loader defers the whole
question of format to `AImageDecoder`, so it has nothing of its own to match on. Every _other_
android loader declares a signature, and every one of those covers a format the `image` crate
already read. The one loader that fills a gap is the one sniffing cannot reach.

A loader like that can only be selected by name, so `b825cb2d` gives the fallback a second try:
sniff first, and for a container we recognise ourselves, ask again by MIME type. One container
needs it — the ISO base media file that HEIC, HEIF and AVIF all sit in, which is a `ftyp` box at
offset four.

All three MIME types are offered to every such file rather than read off the brand, because the
loaders behind them do not divide up the way the names suggest. Android registers `image/heic` and
`image/heif` only, and decodes AVIF behind both. A desktop libheif module answers to all three and
likewise decodes any of them. Reading the brand would mean asking Android for `image/avif` and
being told no, for a file it can read.

Sniffing is still asked first, and that ordering is load-bearing rather than tidy: it is right for
every loader that declares a signature, and insisting past it would only mean handing a corrupt
file to a decoder that already refused it. There is a test for exactly that.

### SVG, which needed no new library

SVG is the one format where the fallback chain runs out on Android. The `image` crate has never
read it. GdkPixbuf reads it through librsvg's loader **module**, and there is no librsvg in the
APK — `rsvg` is named in pixiewood's dependency list but nothing in Commune's Meson resolves the
wrap, so it is never built, and gdk-pixbuf here has no module directory to load one from anyway.
The obvious next move was to cross-build librsvg and wrap it, the way `adwaita-icon-theme` was
wrapped for the icons.

That was not necessary. `nm -D` on this build's `libgtk-4.so` finds 38 exported `gtk_svg_*`
symbols: GTK 4.22 grew a public SVG renderer of its own, `GtkSvg`, a `GdkPaintable` and
`GtkSymbolicPaintable` covering much of SVG 2 with animation, and it has been linked into every
APK this port has produced. It is the same renderer that draws the symbolic icons — which is
why [the icon investigation](#known-gaps) could rule out "the SVG renderer is broken" so early, and
the significance of that was missed at the time.

`src/utils/media/image/decoder/svg_android.rs` is the whole of it, and two things about it are
not like the rest of the backend.

**It runs on the main thread.** `GtkSvg` is a GTK object, and gtk-rs asserts as much on
construction; it cannot go to the Tokio blocking pool where everything else in this backend
decodes. So the sniff and the render happen in `Loader::load` before the hop. That is safe here
and it is worth saying why rather than trusting it: every image request is spawned with
`glib::MainContext::default().spawn()` in `utils::media::image::queue`, and the future is a
`LocalBoxFuture`, so `load` is always polled on the main context. The rendered result is a
`RawFrame` of plain bytes, so `ImageInner` stays `Send` and the rest of the pipeline is untouched.

**It draws through cairo, not through a renderer.** `gtk::Snapshot` gives a `GskRenderNode`, and
`gsk_render_node_draw` puts one on a cairo surface with no GSK renderer to create, realize or keep
alive, and no GL context anywhere near it. The cost is a format conversion — cairo hands back
premultiplied native-endian `0xAARRGGBB` at a stride of its own choosing, and everything
downstream wants tightly packed straight-alpha RGBA — which is about twenty lines and is tested.

Three things this deliberately does not do:

* **No animation.** `GtkSvg` will animate given a frame clock; this asks for one picture. An
  animated SVG draws as its first frame instead of as an error, which is the trade the whole
  fallback path already makes.
* **No vector rescaling.** The SVG is rasterised once, at its declared size or at 512 px if it
  declares only a `viewBox`, and scaled down from there like any other still. Re-rendering per
  requested size is what a vector is for and would be the better answer if these ever look soft.
  2048 px caps either axis, because a declared size is a number in a file we did not write.
* **Nothing outside Android.** The path is `cfg`-gated, and `gtk::Svg` is behind gtk-rs's `v4_22`,
  enabled in the Android-only dependency table. Cargo unions features per target, so enabling it
  beside the main `gtk` entry would let the Linux build call symbols the GNOME 49 runtime's GTK
  4.20 does not have. macOS and Windows do not need it: their GTK stacks ship librsvg's pixbuf
  loader, so the ordinary fallback already reaches SVG there.

One caveat about its tests, since a test that cannot run is worse than none if nobody says so.
`svg_android.rs` carries two — the sniff recognises an SVG and refuses PNG, JPEG, HTML and nothing;
and the premultiplication is undone with rounding rather than truncation, which is exactly the sort
of error that reads as a bad SVG renderer instead of as a bad copy. **Neither runs anywhere today.**
The module is `cfg(target_os = "android")`, and nothing on this port runs a test binary on an
Android target — the build is `cargo rustc --lib --crate-type staticlib`, which does not compile
`cfg(test)` at all. They are there for whoever wires that up, and until then they are read, not run.
The ISO base media tests in `image_rs.rs` are in a module no `cfg` guards, so those do run on a
macOS or Windows checkout.

## S7 — aarch64, and an emulator that runs it

Every measurement in this file up to here was taken on the x86_64 emulator, which is the one
architecture no phone uses. `build-aux/android/io.github.steeb_k.Commune.xml` whitelisted it alone.

Both are whitelisted now rather than aarch64 instead. The emulator is where the test loop lives,
and the manifest's `xi:include` reads the built metainfo out of the **x86_64** build directory by
name, so dropping that architecture would point the include at a directory that was never created —
the trap already written up under [Building a demo APK](#building-a-demo-apk).

### What it took, which was almost nothing

`rustup target add aarch64-linux-android` on the build host, and one `<arch>` element. pixiewood
already shipped `prepare/arch/aarch64.cross`, and the NDK already had
`aarch64-linux-android31-clang`. Nothing else.

It linked first time: 3185 C objects for GTK, GLib, cairo, pango, harfbuzz and the rest, and a
177 MB `libcommune.so` with `ring`, `rustls`, the bundled SQLite, `matrix-sdk` and the JNI glue all
resolved against the aarch64 sysroot. For a first build on a new architecture that is a better
result than it had any right to be, and the reason is that nothing here was ever x86-specific: the
port's Android arms are all JNI and `cfg(target_os)`, neither of which knows what a register is.

The x86_64 SIMD failure does not recur, and it is worth saying why rather than being relieved about
it. That one was NASM-specific. libjpeg-turbo's aarch64 path is NEON **intrinsics** compiled by
clang, with `have_simd` set unconditionally for the architecture (`simd/meson.build:177`), so the
generated `jconfig.h` and the built library agree — which on x86_64, after the wrong-distro
reconfigure, they did not.

| APK | Size |
| --- | --- |
| `app-arm64-v8a-debug.apk` | 334 MB |
| `app-x86_64-debug.apk` | 330 MB |
| `app-universal-debug.apk` | 660 MB |

Debug, unstripped, and Gradle says so: `Unable to strip the following libraries, packaging them as
they are`, for all 36. A stripped release build is still unmeasured.

### The emulator already runs ARM, so the phone can wait

There was no need for an arm64 AVD, which on an x86_64 host means whole-system emulation and is
miserable. `seed_api35` already reports:

```text
ro.product.cpu.abilist:        x86_64,arm64-v8a
ro.dalvik.vm.native.bridge:    libndk_translation.so
```

Android's x86_64 system images have carried ARM64 translation since API 30. So the arm64 APK
installs and runs on the emulator that was already here, and `dumpsys package` confirms Android
chose it rather than falling back:

```text
primaryCpuAbi=arm64-v8a
secondaryCpuAbi=null
```

`nativeloader` then loads out of `base.apk!/lib/arm64-v8a/`, the session restores, and the
homeserver is reachable — which is a larger claim than it sounds, because it means the Keystore
JNI, the filesystem trust roots in `src/utils/tls.rs`, the bundled SQLite and the crypto stack all
work as ARM code. The UI draws, icons included, so the icon-theme wrap and GTK's SVG parsing are
fine here too.

**What this does not establish.** Translation is not a phone. Cold start from `nativeloader` to
"Homeserver is reachable" was 8.7 s against 3.0 s for the native x86_64 build — one measurement
each, both first-launch — and that number says something about `libndk_translation.so`, not about
hardware. Nothing about GL performance transfers either, since the translation layer sits between
the app's native code and the host's graphics stack. What it does buy is that every iteration from
here can be checked on the emulator, and the phone is needed once, at the end.

## S8 — Three input bugs that were one

Reported from the Pixel, in the order they were hit, and they read as a pile of unrelated
papercuts: a URL field with a prose keyboard; **passwords rendered in plain text**; and Next
buttons that stayed dead until focus left the field. The fair reaction to that list is the one it
got — that input handling here is fourth-class and perhaps the whole approach is wrong.

They were one bug.

### The dead field

`GtkIMContextAndroid` has `input_purpose` and `input_hints` struct members.
`gtk_im_context_android_init` sets them to `GTK_INPUT_PURPOSE_FREE_FORM` and
`GTK_INPUT_HINT_NONE`, and **nothing assigns them again.** The file contains no `set_property`, no
`get_property` and no `g_object_class_override_property` at all, where `gtkimcontextwayland.c` and
`gtkimcontextime.c` both have them.

The values were never lost, only elsewhere. `GtkIMContext` installs `input-purpose` and
`input-hints` itself and stores them in its own private struct, so a widget setting the purpose
works exactly as documented — and the Android backend then consults two fields nobody has written
since they were initialised. Every text field in every GTK application on Android was announced to
the IME as free-form prose.

The switch statement that reads them is complete and correct. It maps every `GtkInputPurpose` onto
the matching Android `InputType`, password and PIN included. It was simply reading a variable
nobody populated.

That single fact explains all three reports:

| Symptom | Why |
| --- | --- |
| Passwords in plain text | No `TYPE_TEXT_VARIATION_PASSWORD`, so the IME leaves suggestions and composing on, and composing text draws unmasked. Anything forcing a commit masks it, which made it look intermittent |
| URL field with a prose keyboard | No `TYPE_TEXT_VARIATION_URI`. Setting `input-purpose: url` on the widget changed nothing, because the property arrived and was then ignored |
| Next button dead until focus left | A composing IME holds text in preedit rather than committing per keystroke, so `changed` never fires. **Inference, not measurement** — the correct types suppress composing, so it should follow |

### The fix, and why it is one line

`patch-gtk-input-purpose.sh` fills the two dead fields from the live properties immediately before
the switch reads them. Nothing else needed changing, because nothing else was wrong.

Focus moving between two entries swaps the active `GtkIMContext`, and
`ToplevelActivity.setActiveImContext` calls `imm.restartInput`, so the type is re-queried per
field rather than fixed at whichever field was focused first. That was checked before building
rather than after, because a fix that worked only for the first field would have looked like a
working fix.

**Confirmed on the Pixel 9a:** passwords mask as typed, and the homeserver field gets a URL
keyboard.

### What this says about the backend

This is the **second** complete-but-unconnected implementation found in this one subsystem.
`patch-gtk-ime.sh` exists because `ToplevelActivity.onCreateInputConnection` hardcoded
`outAttrs.inputType = InputType.TYPE_NULL` **with the working line commented out directly above
it**, which meant no keyboard appeared at all.

Two of those is a pattern worth naming, because it changes what "immature" means here. Nothing so
far has been an architecture that cannot express what Android needs; it has been code that was
written and never wired up. Both fixes were one line each, and both were carried locally as patch
scripts without waiting on upstream.

The space-bar cursor gesture is the honest test of whether that pattern holds, because it is the
first one that is genuinely _missing_ rather than disconnected — see
[Before this ships](#before-this-ships).

## S9 — The space bar, and the reset that cancelled it

Sliding a thumb along Gboard's space bar should drag the cursor with it. In Commune it moved a
character or two and stopped, which is where S8 left it: _"Spacebar sliding is already a huge
deal."_ The cause turned out not to be in the Java glue where the search started, and finding it
needed a way to test a keyboard without a keyboard.

Both defects here are GTK's rather than Commune's, and are written up for filing in
[doc/upstream-gtk-android-ime.md](upstream-gtk-android-ime.md).

### The emulator can be typed on after all

Every keyboard measurement in this ledger before tonight carried an asterisk, because the emulator
shows Gboard's collapsed physical-keyboard strip rather than a keyboard — see
[The emulator is in physical-keyboard mode](#the-emulator-is-in-physical-keyboard-mode). That made
the phone the only place input could be judged, which is why S8 was deferred until there was one.

The strip's own menu ends that. **Hamburger → "Show on-screen keyboard"** draws the full QWERTY,
`setting secure show_ime_with_hard_keyboard` already being `1`, and from there the gesture can be
driven synthetically:

```sh
adb -s emulator-5554 shell input swipe 790 1444 500 1444 1200   # a slow drag along the space bar
```

Two things had to be got right for that to measure anything.

**`adb shell input text` is not typing.** It injects key events at the view and never touches the
`InputConnection`, so the keyboard does not learn the characters exist and its cursor model stays
at 0 no matter what the application answers. Measured directly: with text put in that way Gboard
asked for `setSelection(0, 0)` — the whole document, as far as it knew, being empty. Typing the
same string by tapping Gboard's own keys produced `setSelection(10, 10)` instead. Every keyboard
measurement has to tap keys.

**A control is needed.** The identical swipe was run first against the stock Android Settings
search box, which moved the cursor seven characters. That fixes the swipe as a working instrument
before pointing it at Commune, so that "nothing happened" can be read as a fault in the app rather
than in the test.

The marker technique: after the gesture, `adb shell input text "X"` inserts at GTK's real cursor
regardless of where the keyboard believes it to be, so a screenshot shows the answer as a letter in
a word rather than as a caret to squint at.

### The baseline

| | stock Settings box | Commune, before |
| --- | --- | --- |
| identical 290 px space-bar swipe | cursor moves 7 characters | cursor does not move |
| Gboard's suggestion strip | `world`, `worlds` | `what`, `I`, `I'm` |

The suggestion strip is the tell. Sentence-openers mean Gboard has no idea what is in the field.

### What the keyboard actually asks for

The ledger's previous explanation was inference, and it was wrong. It said the gesture "falls back
to synthesised arrow keys and loses track of its own position". Logging every `InputConnection`
call shows Gboard never sends a key event for this. It sends **`setSelection`**, once, and then
gives up — and the cursor not having moved at all in the baseline is what rules the arrow-key story
out, since arrow keys would have worked: `AKEYCODE_DPAD_LEFT` maps to `GDK_KEY_Left`
(`gdkandroidkeysyms-private.h:61`), and five of them injected by hand move a `GtkText` cursor five
characters. That was measured before anything was written, because the whole fix depends on it.

### Three things were wrong, and only the third was the cause

**The keyboard could not read the text.** `ImeConnection` overrode no text query, so all of them
fell through to `BaseInputConnection`, which answers out of the `Editable` from `getEditable()` —
the composing scratch buffer that `commitText` and `finishComposingText` call `clear()` on after
every commit. Truthfully, as far as that class knew, the document was empty. The real text was one
unused call away: `ImContext` already declares `public native SurroundingRetVal getSurrounding()`
and `gtkimcontextandroid.c` already implements it. Nothing called it.

**`GtkIMContext` cannot move a cursor.** The protocol an input method gets is `commit`,
`delete-surrounding` and `retrieve-surrounding`. There is no `set-cursor`, so `setSelection` is
spelled out in arrow keys — the same trick `deleteSurroundingText` already uses to spell deletion
out in backspaces.

**Every cursor movement restarted the input method.** This is the actual cause.
`gtk_text_move_cursor` ends, unconditionally, with

```c
  priv->need_im_reset = TRUE;
  gtk_text_reset_im_context (self);
```

so every arrow key runs `gtk_im_context_reset`. On Wayland that discards preedit state and returns.
On Android `gtk_im_context_android_reset` calls the Java `ImContext.reset`, which is
`imm.restartInput(view)` — which destroys the `InputConnection` and builds a new one. Whatever
gesture the keyboard had in flight dies with it.

The proof is in the `EditorInfo` of the connection that replaced it. After a slide that moved the
cursor one character from 11 to 10, `dumpsys input_method` reports `initialSelStart=10` — a
_freshly created_ connection, seeded at the position the cursor had just reached. The keyboard was
not confused; it was hung up on mid-sentence and had to dial again.

That also means the cost is not limited to this gesture: a full IME teardown was being paid per
arrow key, per tap into a field, and per selection change.

### What was measured

All on the emulator, against Commune's room-list search entry — a `GtkText` that filters locally,
so nothing here goes near the account.

| build | text typed by | what the keyboard asked for | where the cursor ended up |
| --- | --- | --- | --- |
| before any change | `input text` | nothing | did not move |
| text queries answered from `getSurrounding` | `input text` | `setSelection(0, 0)` | jumped to the start |
| the same build | Gboard's keys | `setSelection(10, 10)`, once | moved one character, stopped |
| `getExtractedText` added | Gboard's keys | `setSelection(10, 10)`, once | moved one character, stopped |
| `reset` no longer restarts input | Gboard's keys | `setSelection` 10, 9, 8, 7, 6 | **slid five characters with the swipe** |

Rows two and three are the same build and differ only in how the text got there, which is the
cleanest statement of why `input text` cannot be used to test a keyboard. `setSelection(0, 0)` was
not a bug in the fix; it was the fix faithfully carrying out the instruction of a keyboard whose
model of the document was empty, because nothing had ever gone through its `InputConnection`.

**`getExtractedText` did not change the outcome.** Rows three and four are identical. It is kept
because `BaseInputConnection` returns `null` there and a `null` reads to the keyboard as "no text",
which is worth not saying when it is untrue — but the honest record is that the decisive change was
the last row, and nothing before it moved the needle past one character.

The final slide is 230 px for five characters against the Settings box's 290 px for seven: the same
sensitivity, within the threshold that starts the gesture.

Regression, same rig: typing `testing` on Gboard's keys and deleting three characters with Gboard's
backspace gives `test`, the room list filters to `testchat`, the suggestion strip offers
`test`/`rest`/`testing`, and `inputType` is still `0x1`. The S8 fixes still hold.

**Confirmed on the Pixel 9a, 25 August 2026** — _"This works beautifully."_ Including **in the
message composer**, which is the `GtkTextView` case and the one worth having checked: there
`retrieve-surrounding` returns a sliding window rather than the whole buffer, so the offsets handed
to the keyboard are relative to a frame that moves as the cursor does. Computing every move as a
delta from the position read in the same call is evidently enough to absorb that.

The emulator rig predicted hardware correctly, which is the other thing worth knowing about it:
every conclusion in this section was reached with the phone switched off, and none of them had to
be revised once it was switched on.

### One bug of mine, not a demonstrated cause

The first attempt answered `getTextAfterCursor` with

```java
s.text.substring(s.end, Math.min(s.text.length(), s.end + length))
```

and `length` comes from the keyboard, which is entitled to pass `Integer.MAX_VALUE`. `s.end +
length` then overflows to a negative, `Math.min` picks it, `substring` throws — and an exception
thrown across a binder reaches the caller as a plain `null`, which the keyboard reads as an empty
document with nothing anywhere to explain it. The clamp is now by the room available rather than by
adding and hoping.

Recorded because it is a real defect and a nasty failure mode, **not** because it was observed:
nothing in the measurements above is known to have been caused by it.

### What this says about the backend

S8 counted two complete-but-unconnected implementations and called it a pattern. This is the third:
`getSurrounding()` was declared, implemented in C, registered in `im_context_natives[]`, and called
by nothing.

But the pattern does not hold all the way, and the difference matters. The
restart-on-every-cursor-move is not unwired code. It is a **wrong interaction between two pieces
that are each locally reasonable** — `gtk_text_move_cursor` resetting the IM context, which is
correct on every other backend, and the Android backend mapping `reset` onto `restartInput`, which
is the only honest mapping available to it. It misbehaves only where those two meet, which is why
nothing upstream has caught it, and why no property Commune sets could ever have worked around it.

That makes it a better-shaped upstream contribution than the previous two: a one-line guard with a
reproducible measurement behind it.

## S10 — The keyboard that would not capitalise, and the gesture that left

Two reports from using the port as a phone application rather than testing it as one, on 25 August
2026. They share nothing technically, and they share everything else: each is a place where the app
does what a GTK application does and not what an Android application does, and neither is visible
from a desktop.

> _"the compose textbox doesn't capitalize at the beginning of the line or after a period — it
> doesn't act like a normal input box on Android."_
>
> _"opening a picture — it goes full screen, and on Android you swipe from the edge to go back on a
> view like that, but this app acts like you are swiping back from the main view, sending you back
> to the desktop."_

### The composer never asked to be capitalised

[S8](#s8--three-input-bugs-that-were-one) fixed the field that carries this: `input-purpose` and
`input-hints` reach `GtkIMContextAndroid`, and its switch turns them into an Android `InputType`
with all the flags Android has for a text field, `TYPE_TEXT_FLAG_CAP_SENTENCES` included.

Nothing in Commune sets a hint. The default is `GTK_INPUT_HINT_NONE`, which is right for a
desktop — a hardware keyboard has a shift key and the user is holding it — and wrong for the one
widget in the application that is prose. So the composer gets
`input-hints: spellcheck | uppercase-sentences`, which becomes `TYPE_TEXT_FLAG_AUTO_CORRECT |
TYPE_TEXT_FLAG_CAP_SENTENCES`, and nothing else in the app gets either: a room name is a name, a
homeserver is a URL, a search box is a query, and a keyboard that shifts the first letter of any of
them is a keyboard fighting the user.

`word-completion` was deliberately left off. It maps to `TYPE_TEXT_FLAG_AUTO_COMPLETE`, which tells
the keyboard the _application_ is completing what is typed — true here, the composer completes
mentions and emoji — and the cost is that some keyboards then stop offering their own suggestions.
The composer's completion is for `@` and `:`; the keyboard's is for every other word. Both are
wanted.

### Which is when the keyboard capitalised everything

Setting the flag alone is worse than not setting it, and this was found by reading the glue rather
than by shipping it.

Android does not decide by itself which letters are sentence-initial. It asks the field, through
`InputConnection.getCursorCapsMode()`, and shifts whatever the field calls the start of a sentence.
`ImeConnection` does not override it, so the answer comes from `BaseInputConnection`, which reads
the `Editable` that `getEditable()` returns — the composing scratch buffer that `commitText` and
`finishComposingText` `clear()` after every commit. The question is therefore always asked of an
empty document with the cursor at 0. That is the start of a sentence, truthfully, for that buffer.
The keyboard would have shifted the first letter of every word.

This is the same root cause as [S9](#s9--the-space-bar-and-the-reset-that-cancelled-it) — the
scratch buffer answering questions about a document it does not hold — and the same fix shape.
`patch-gtk-ime-caps.sh` overrides `getCursorCapsMode` and hands `TextUtils.getCapsMode` the text
around the cursor from the `snapshot()` that `patch-gtk-ime-selection.sh` already built. It has to
run after that patch, and it says so.

What it does not fix is the same limitation S9 records: `gtk_text_view_retrieve_surrounding_handler`
returns the cursor's line widened to three word boundaries, not the buffer. Caps mode only looks
backwards from the cursor, so the window is enough whenever the sentence started inside it — which
in a composer is a line, and so nearly always. A sentence carried across a hard line break gets a
capital it did not earn.

### And then capitalised every word anyway, for a second reason

Measuring the fix rather than trusting it found a third bug, one line long, in the JNI field cache
`gtk_im_context_android_init_java_cache` fills:

```c
FILL_INPUT_TYPE (text_flag_cap_words, "TEXT_FLAG_CAP_WORDS")
FILL_INPUT_TYPE (text_flag_cap_sentences, "TEXT_FLAG_CAP_WORDS")
```

The second line reads Android's `TEXT_FLAG_CAP_WORDS` into the field called
`text_flag_cap_sentences`. `_gtk_im_context_android_get_input_type` is correct; the constant it
reaches for is not. `GTK_INPUT_HINT_UPPERCASE_SENTENCES` therefore sends `CAP_WORDS`, and the
keyboard shifts the first letter of every word — the same visible symptom the `getCursorCapsMode`
omission would have produced, arriving by a completely different route.

_Measured_ with `dumpsys input_method` on the composer, which is the only reason it was caught:

| | `inputType` | means |
| --- | --- | --- |
| Before | `0xa001` | `CLASS_TEXT \| CAP_WORDS \| AUTO_CORRECT` |
| After | `0xc001` | `CLASS_TEXT \| CAP_SENTENCES \| AUTO_CORRECT` |

Carried as `patch-gtk-caps-sentences.sh`. It is the third defect in this file found by asking what
a value actually is rather than what the code says it is, after the dead struct fields of
[S8](#s8--three-input-bugs-that-were-one) and the cleared scratch buffer of
[S9](#s9--the-space-bar-and-the-reset-that-cancelled-it).

### Back is not close

GDK's Android backend registers an `OnBackInvokedCallback` and turns it into exactly one thing:

```c
GdkEvent *event = gdk_delete_event_new (surface);
gdk_surface_handle_event (event);
```

`gdkandroidtoplevel.c:133`. A delete event is the only thing GDK has to say here, and GTK turns it
into `GtkWindow::close-request`. So the system back gesture, the back button and closing the window
are one signal, and `Window::close_request` answered all three the way a desktop answers the third:
save the window size, save the session, quit.

That is right in exactly one place — the room list, where there is nothing left to go back through —
and wrong everywhere else. It is most obviously wrong in the media viewer, which is full-screen, has
a back button of its own in its header bar, and is the view where the edge swipe is most natural.

The fix is on the Commune side, not upstream: GDK is not wrong to send a delete event, and there is
nothing else it could send. `Window::close_request` unwinds what is on screen one step at a time and
only lets the close through when there is nothing left to unwind. The order is what is on top:

1. **A popover**, which holds the focus while it is up — the only handle on one from here, since
   GTK exposes no list of open popovers.
2. **A dialog**, through `adw_application_window_get_visible_dialog()`. `AdwDialog::close()` returns
   whether it closed, so a dialog that refuses (`can-close: false`, a confirmation in flight) does
   not silently eat the gesture.
3. **The visible page**, which decides for itself. The login flow pops its `AdwNavigationView`; the
   session view closes the media viewer if it is up, then closes an open search bar, and otherwise —
   only when the split view is collapsed, which is to say only on a phone-shaped window — deselects
   the room, which is the same thing `Escape` and the header bar's back button do.

The search bar is found by walking the mapped part of the widget tree for a `GtkSearchBar` in
search mode rather than by listing the five this application has. Only the page on screen is
mapped, so the walk finds the one the user is looking at and cannot reach into a page they are not.
It was added after the first round of measurement below found back leaving the room while the
search bar was still open.

All of it is `#[cfg(target_os = "android")]`. On every other platform closing a window closes the
window, and a stack of things to unwind first would be a bug rather than a feature.

### Room Details was a window, which on Android is an Activity that never arrives

Testing the gesture everywhere found the one place it could not be tested: opening _Room Details_
wedged the application. The diagnosis is under [Known gaps](#known-gaps); the short version is that
it was an `AdwPreferencesWindow`, GDK gives every GTK toplevel its own Activity, and `singleTask`
in the manifest means Android hands that Activity's intent to the one that already exists.

The manifest keeps `singleTask` — it is what makes the launcher relaunch and the SSO redirect work.
`RoomDetails` stops being a window instead:

* `AdwPreferencesWindow` → `AdwPreferencesDialog`, which is where libadwaita has gone and which the
  build had been warning about on every run. The FIXME at the top of the file said this could not be
  done "because we need to be able to open the media viewer"; the media viewer in question is the
  history viewer's own, an overlay inside the widget, and it works in a dialog because an
  `AdwDialog` on a phone-shaped window is very nearly the whole screen anyway.
* `modal` and `destroy-with-parent` go — a dialog is both — and `default-height` becomes
  `content-height`.
* The `win.toggle-fullscreen` action that `RoomDetails` installed for that media viewer goes too. A
  dialog cannot be fullscreened, and now that the dialog's root is the main window the action falls
  through to the one `Window` already installs, which is the window the viewer wanted fullscreened
  in the first place. The viewer's `fullscreened` binding follows the same path.
* Two call sites had reached for the window: `general_page` took `transient_for()` to find the main
  window, which is now `root()`, and `invite_subpage` closed itself by finding an
  `AdwPreferencesWindow` above it, which is now the `AdwPreferencesDialog` ancestor.
* `add_toast` loses its `AdwPreferencesWindow` branch, which nothing can reach any more.

### Where a dialog's back actually has to live

The first attempt at "back pops a subpage before closing the dialog" was written into
`Window::close_request` and never ran once. `close-request` is `G_SIGNAL_RUN_LAST` with a boolean
accumulator (`gtkwindow.c:1271`), so connected handlers run **before** the class closure, and
`AdwDialogHost` connects one (`adw-dialog-host.c:276`) that closes the visible dialog and returns
`GDK_EVENT_STOP`. The emission ends there. Every dialog closing correctly on back — which it did
from the first build — was libadwaita doing it, not this port.

That is the behaviour to want, so the window keeps out of it and the two dialogs with subpages say
what back means for them instead. On Android they are constructed with `can-close` unset, which
turns `AdwDialogHost`'s `adw_dialog_close()` into a `close-attempt` emission, and their
`close_attempt` pops a subpage or `force_close()`s if there is none. `RoomDetails` and
`AccountSettings` both do this; nothing else in the application has a subpage stack.

The cost of `can-close` is that a programmatic `close()` becomes a back step too, so the four places
that meant "close this now" — following a Matrix URI out of the general page, the invite subpage
finishing, `account-settings.close`, and the session logging out — say `force_close()`.

`RoomDetails` has one step above the subpages: the media history viewer carries a `MediaViewer` of
its own, several subpages down from anything that can name it, and back was skipping it — popping
the whole Media subpage and taking the viewer with it. `close_attempt` looks for it first, by the
same mapped-subtree walk the search bar uses.

### The deadlock underneath, which had been hiding behind an unreachable window

Back inside room details then stopped working entirely — and so did every touch. The first two
theories were both wrong and both cost a build each: it is not a stale `GtkPopover` grab (the walk
found no mapped popover, and a tap does not clear it), and it is not `GtkWindow`'s delete-event grab
guard (`gtkmain.c:1723`, which drops delete events when a grab lives outside the window).

`debuggerd -b <pid>` settled it in one shot. The GTK thread and GStreamer's own thread are deadlocked
against each other:

```text
"GTK Thread"                              "GstPlay"
ScaleRevealer::transition_done            gst_play_stop_internal
  set_visible(false)                        gst_element_set_state
    gtk_widget_unmap                          gst_play_sink_change_state
      MediaContentViewer::clear                 g_rec_mutex_lock  ← waits
        gtk_video_set_media_stream(NULL)
          gtk_picture_clear_paintable
            g_object_unref(GstMediaStream)
              gst_play_dispose
                g_thread_join  ← waits
```

Detaching the stream tears down the paintable the video sink owns, and the GTK thread holds the
sink's lock while it does it. Dropping the last reference to the stream in the middle of that
disposes `GstPlay`, and `gst_play_dispose` joins its own thread — which is sitting in
`gst_play_sink_change_state` waiting for the lock the GTK thread is holding. Neither moves again.
The application is not slow or confused at that point, it is stopped; Android keeps compositing the
last frame, which is why it looks alive.

The fix is one line of ordering in `clear_video`, which is the macOS and Android arm of
`MediaContentViewer`: keep the stream alive across the detach and let go of the last reference on an
idle, once the unmap has returned and the lock is free.

This is not a room-details bug and was not introduced by making it a dialog — it is in the media
stack, and room details is simply the first place on Android that could reach it, because the window
it used to be never opened. The timeline's media viewer does not reproduce it, which is a timing
difference rather than a structural one and is not a reason to think it cannot.

Which then wedged back inside room details entirely, and the reason is worth keeping. "Is the media
viewer open" had been asked as `is_visible()`, because that is what `reveal()` sets. It is not what
`close()` clears: closing starts a transition, and the widget is hidden in
`ScaleRevealer::transition_done`, which on Android does not reliably arrive. So the viewer stayed
`visible` after it had gone, the walk found it on every back, and every back closed something that
was already closed. `MediaViewer::is_open()` asks the revealer's `reveal-child` instead, which
`close()` sets synchronously. The session view's own check had the same bug latent in it and now
uses the same method.

### What was measured

Every surface reachable on the emulator, with both the BACK key and a real edge swipe
(`input swipe 3 1300 750 1320 250`):

| Where | What back does |
| --- | --- |
| Room, narrow window | Returns to the room list |
| Media viewer | Closes it, returns to the room |
| Media viewer, fullscreened | Closes it — **with the key**; see below |
| Explore | Returns to the room list |
| Search bar open in a room | Closes the search bar (after the fix above; before it, left the room) |
| Dialog (About) | Closes the dialog |
| Popover (the main menu) | Closes the popover |
| Room list | Leaves the application, which is correct |

Two things the matrix does not say plainly.

**In the fullscreened media viewer the edge swipe does not reach the application.** Android's
immersive mode takes the gesture insets for itself, so an edge swipe there shows the system bars
instead of invoking back; the BACK key, the header bar's own back button and the swipe-down
dismissal all still work. This is how every immersive-mode Android application behaves and is not
something the port can change from its side.

**A popover is dismissed by the touch, not by the gesture.** Touching anywhere outside a popover
closes it, which happens on the swipe's first touch, so the gesture that follows is a back from the
page underneath. The BACK key, which involves no touch, closes the popover and stops there. Normal
behaviour rather than a defect — nobody swipes from the edge to dismiss a menu.

One thing this deliberately does not touch: dismissing the soft keyboard. Android hands BACK to the
IME before the activity while the keyboard is up, so the app never sees that press, and the
behaviour recorded under [Known gaps](#known-gaps) — BACK hides the keyboard and leaves the room
open — is unchanged.

## Before this ships

A running list, in the user's words where they said it. Nothing here blocks further development;
all of it blocks calling the port finished.

* ~~**Sliding the space bar to move the cursor.** _"I need to touch on it before we ship
  anything."_ Upstream, in `gdk/android/glue/java/org/gtk/android/ImContext.java` — four missing
  `InputConnection` overrides.~~ **Fixed on 24 August 2026** by `patch-gtk-ime-selection.sh` and
  `patch-gtk-ime-reset.sh`, and **confirmed on a Pixel 9a on 25 August 2026** — see
  [S9](#s9--the-space-bar-and-the-reset-that-cancelled-it). The missing overrides were real but were
  not the cause; the cause was `reset` restarting the input method on every cursor movement.
* ~~**The homeserver field still gets a plain keyboard, not a URL one.** Setting `input-purpose`
  did not change it; the remaining break is upstream and sits in the same file as the space-bar
  bug, so one keyboard pass covers both.~~ **Fixed** by `patch-gtk-input-purpose.sh` and confirmed
  on the Pixel 9a on 24 August 2026 — it was the same dead field as the plaintext passwords. See
  [S8](#s8--three-input-bugs-that-were-one).
* **Translations are missing entirely.** `po/meson.build`'s `i18n.gettext()` install carries no
  `install_tag`, so pixiewood's `meson install --tags runtime` drops it silently and the
  application is English-only on Android. Recorded much earlier as something that _"should be fixed
  before anyone sees it"_, and still true.
* **File the GTK IME defects upstream.** Notes are written and ready to paste:
  [doc/upstream-gtk-android-ime.md](upstream-gtk-android-ime.md). Six of the nine patch scripts
  this port carries are GTK bugs rather than Commune glue, and every one of them is a local patch
  that has to be re-applied after each `pixiewood generate` and re-checked against each GTK update:

  | script | defect |
  | --- | --- |
  | `patch-gtk-ime.sh` | `outAttrs.inputType` hardcoded to `TYPE_NULL`, working line commented out above it — no keyboard at all |
  | `patch-gtk-input-purpose.sh` | `input_purpose`/`input_hints` read from struct fields nothing assigns after `_init` — every field announced as free-form prose, passwords included |
  | `patch-gtk-ime-reset.sh` | `reset` calls `restartInput` on every cursor movement, cancelling any IME interaction in flight |
  | `patch-gtk-ime-selection.sh` | `ImeConnection` answers no text query and has no `setSelection` |
  | `patch-gtk-ime-caps.sh` | `ImeConnection` answers `getCursorCapsMode` from the cleared scratch buffer, so every word looks sentence-initial |
  | `patch-gtk-caps-sentences.sh` | the JNI field cache reads `TEXT_FLAG_CAP_WORDS` into `text_flag_cap_sentences`, so `UPPERCASE_SENTENCES` capitalises every word |

  The first three and the sixth are defects with one-line fixes, and the fifth is a six-line
  override that only became worth writing once the fourth had put the real text within reach. The
  fourth is a missing feature, and the honest part of it is that `GtkIMContext` cannot express "put
  the cursor here" at all — so what is carried here is a workaround (synthesised arrow keys) rather
  than something to propose as a patch without asking the maintainers first.

  This does not block Commune shipping. It is on the list because carrying six downstream patches
  against a moving `main` branch is a standing cost, and because the fixes are worth more to other
  GTK-on-Android applications than they are here.
* **Two loose ends from the push round (S5b), to be revisited once the push implementation is
  complete** — parked deliberately, because both were seen exactly once and chasing them
  mid-implementation would be debugging a moving target:
  * **The woken process's syncs failed with DNS errors on the emulator** — `failed to lookup
    address information` against a homeserver the same emulator resolves when foregrounded, in the
    process a push had started in the background. Possibly emulator DNS flakiness, possibly
    something real about resolver behavior in a background-started process (the same logcat showed
    `netlink_route_socket` SELinux denials). If it is real, the step 3 wake path inherits it.
    Retest on the Pixel 9a with the finished wake path: kill Commune, push, watch whether the
    single-event fetch resolves and completes. **Second sighting, 26 August 2026, and it widens
    the suspect pool**: a fresh _foreground_ launch hung at "Fetching Account Data…" with every
    in-app request stalled while `adb shell ping matrix.kzenjak.com` resolved fine, after the
    emulator had been up all day — and an emulator reboot cured it completely. So it is not
    specific to background-started processes; "long-running emulator's networking degrades" is
    now the leading theory, and the hardware retest is what settles it.
  * **One launch died silently at the splash screen**, right after a reinstall, on 26 August 2026
    — no crash-buffer entry, no `GTK Runtime` line, `binderDied` about 24 seconds after process
    start — and the identical launch a minute later worked. Seen once, unexplained. If it recurs,
    treat it as the panic-hook lesson says: a hang or death with an empty logcat is usually a
    panic in a tokio task. Check it has not become reproducible before shipping — a first launch
    after install is exactly the launch a new user sees.

## Known gaps

* ~~**Room Details wedges the application, and it is a second toplevel that does it.**~~ **Fixed on
  25 August 2026**, the same day it was found, by making it stop being a toplevel. Kept here because
  the diagnosis is the useful part and the trap is still set for the next widget that wants a window.

  Opening _Room Details_ used to leave the room on screen with every touch and every back press
  dead; only force-stopping recovered. `RoomDetails` was an `AdwPreferencesWindow` — a second GTK
  toplevel — and GDK's Android backend gives every toplevel its own Activity, so presenting it
  started a second `ToplevelActivity`:

  ```text
  START u0 {cmp=io.github.steeb_k.commune/org.gtk.android.ToplevelActivity (has extras)}
      with LAUNCH_SINGLE_TASK ... result code=3
  ```

  `result code=3` is `START_DELIVERED_TO_TOP`. The intent went to the Activity that already existed
  instead of creating another, because
  [`patch-manifest.sh`](../build-aux/android/patch-manifest.sh) sets
  `android:launchMode="singleTask"` — on the reasoning, written into that script, that _"GTK has a
  single toplevel"_. That was true when it was written and was not true here. The new GTK toplevel
  got no Android window while still holding GTK's focus and grab, and every event went to a window
  nobody could see.

  **The manifest was left alone.** `singleTask` is load-bearing for two other things — the launcher
  relaunch it was added for, and the SSO redirect, which reaches a running Commune through
  `onNewIntent` and therefore only because of it. What changed instead is the widget:
  `AdwPreferencesWindow` → `AdwPreferencesDialog`, which is where libadwaita has gone anyway and
  which the build had been warning about. See
  [S10](#s10--the-keyboard-that-would-not-capitalise-and-the-gesture-that-left).

  **`CallView` is the only other window in the application, and Android never sees it.** It is an
  `AdwWindow`, and the whole of it — `mod call_view`, the import, the `call_view` field,
  `watch_calls`, `present_call_view` and `handle_call_action` — is already
  `#[cfg(not(target_os = "android"))]`, because the WebRTC-over-GStreamer stack behind it is not on
  Android yet. So it cannot spring this trap today. It will, unchanged, on the day calls arrive:
  whoever does that work has to decide between a dialog (modal, so no reading the room during a
  call), a page in the main window, and reopening the `launchMode` question.

  Nothing else in the application is a toplevel. `Adw.ShortcutsDialog`, `Adw.AboutDialog`,
  `AccountSettings` and every `ToastableDialog` are dialogs already, and nothing constructs a
  `GtkWindow`, `GtkAlertDialog` or `GtkAboutDialog` at runtime.
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

  **Superseded.** Retesting in S3 found something else entirely underneath: GTK told the IME the
  field was not a text field at all, so on current GTK there was no keyboard to hide. See
  [The IME, and why no keyboard appeared](#the-ime-and-why-no-keyboard-appeared).

  **Retested again, 24 August 2026, with a real session and a real `sourceview::View` composer.**
  Typed into the composer in a real room; `dumpsys input_method` showed `mInputShown=true`. The
  system **BACK** button hides it correctly: `mInputShown` goes to `false`, the composer keeps
  focus, the typed text is untouched, and the room stays open. That is the behaviour that matters —
  the ordinary gesture of dismissing the keyboard to see the timeline while composing works.

  Two things this does not settle. It was measured against the emulator's own collapsed toolbar,
  not a full soft keyboard — see
  [The emulator is in physical-keyboard mode](#the-emulator-is-in-physical-keyboard-mode) — since
  Gboard still treats this AVD as having a physical one; a device or an AVD without that quirk
  would be the stronger test. And **`ESC`** does not behave like `BACK`: it triggers GTK's own
  back-navigation and leaves the room entirely rather than only dismissing the IME. Not itself a
  bug, just a reason not to read `ESC` and `BACK` as equivalent here.

  **Tested at last on real hardware, 24 August 2026 — a Pixel 9a running GrapheneOS, Android 17,
  with a real Gboard.** The keyboard itself works: it appears for the composer, types, and hides.
  That closes the question this bullet has carried since S0, and it took a physical device to do
  it, because the emulator has never stopped claiming a hardware keyboard.

  Two things it immediately found that no emulator run could have.

  ~~**The homeserver field does not ask for a URL keyboard, and setting the purpose did not fix
  it.**~~ **Fixed** (`7011944e`), together with plaintext passwords, because they were the same
  bug. The first guess above was right about where the break was and wrong about it being
  unreachable: `GtkIMContextAndroid` reads its own `input_purpose` field, which
  `gtk_im_context_android_init` sets to `FREE_FORM` and nothing assigns again, while the value the
  widget set lives in `GtkIMContext`'s private struct. Every field in every GTK application on
  Android was therefore announced as free-form prose. One line, carried as
  `patch-gtk-input-purpose.sh`. See
  [Three input bugs that were one](#s8--three-input-bugs-that-were-one).

  **Confirmed on the Pixel 9a:** the password field masks as you type, and the homeserver field
  gets a URL keyboard.

  ~~**Sliding along the space bar to move the cursor is broken, and it is upstream.** It moves a
  character or two and stops. `ImContext.java` is 107 lines and overrides exactly four methods;
  there is no `setSelection`, no `getTextBeforeCursor`, no `getTextAfterCursor` and no
  `getSelectedText`. Gboard's spacebar gesture has to both move the cursor and read back where it
  landed; with none of that implemented it falls back to synthesised arrow keys and loses track of
  its own position almost at once.~~ **Fixed on 24 August 2026**, and the explanation above was
  wrong on two counts, both corrected by measurement in
  [S9](#s9--the-space-bar-and-the-reset-that-cancelled-it):

  * Gboard does **not** fall back to synthesised arrow keys. It calls `setSelection` and nothing
    else. What ruled the arrow-key story out is that the cursor did not move at all — arrow keys
    would have worked, and were separately measured doing so.
  * The missing overrides were real, and adding them was not enough. The cause was
    `gtk_text_move_cursor` resetting the IM context on every cursor movement, which on Android
    means `InputMethodManager.restartInput` — tearing the `InputConnection` down underneath the
    gesture. One guard in `gtk_im_context_android_reset`, carried as `patch-gtk-ime-reset.sh`.

  Confirmed on the emulator and then on the Pixel 9a, 25 August 2026.
* The IME comes up unbidden on launch. Still true of the Adwaita demo on Arch, so it is the glue's
  behaviour and not something either demo does.
* ~~The Android data directory is external storage~~ and ~~`glib::user_cache_dir()` looks
  wrong~~. **Both fixed**, see [The secret store](#the-secret-store). Measured on the emulator as
  `/data/user/0/…/files/commune` and `/data/user/0/…/cache/commune`.
* **The Keystore key is not bound to user presence.** No `setUserAuthenticationRequired`, so
  anything running as this UID can decrypt without a lock-screen prompt. Deliberate for now —
  sessions are restored at startup, before there is a window to prompt over — and the obvious next
  tightening. Hardware backing is not required either: the Keystore uses secure hardware where the
  device has it and falls back to software silently, and nothing here refuses that.
* **`cargo doc` fails on two pre-existing links, and not only for Android.**
  `cargo doc --no-deps --target x86_64-linux-android` errors with `unresolved link to gdk::Texture`
  (`src/utils/media/image/decoder/mod.rs:15`) and warns `redundant explicit link target` (`:22`).
  Neither is an Android problem: `gdk` is not a dependency of the crate at all — only `gtk`
  (package `gtk4`) is — so the link cannot resolve on any target, and `src/meson.build` builds docs
  with `-Dwarnings`. The fixes are `[`gtk::gdk::Texture`]` and dropping the explicit target. Only
  the Android target was measured; this is left alone here because that file is the decoder seam
  shared with the macOS and Windows ports.
* **330 MB debug APK** for Commune on x86_64, and 334 MB on aarch64 (125–136 MB for the
  demos). The debug symbols are
  already off; what is left is GTK, libadwaita, GtkSourceView, harfbuzz and 168 MB of Rust. A
  release build with stripping has not been measured, and neither has an `aarch64` one.
* GStreamer is not built at all: pixiewood's cross file sets `media-gstreamer = 'disabled'` for
  GTK, so voice messages, video and calls are all out of reach until S4.
* ~~OAuth 2.0 / SSO login cannot complete.~~ **Fixed and confirmed against a real `matrix.org`
  account**, see [OAuth 2.0 / SSO login on Android](#oauth-20--sso-login-on-android).
* ~~**Unsupported image filetypes will surface here too, and the fix has not been ported.**~~
  **Ported, and the prediction under it was wrong.** The entry used to say that HEIC and AVIF would
  need `libheif`/`libavif` cross-built and wrapped before a GdkPixbuf fallback could reach them,
  because pixiewood's dependency list only wraps `rsvg`. That is not so: the codecs have been in
  the APK all along. gdk-pixbuf builds an `android` loader family whenever it finds `jnigraphics`,
  and this build has it — jpeg, png, gif, webp, bmp, ico, wbmp and **heif**, each one a shim over
  the NDK's `AImageDecoder`. `strings libgdk_pixbuf-2.0.so` shows `image/heic` and `image/webp` in
  the shipped library.

  Two commits: `f745e2bb` cherry-picks the Windows fallback (`0454830b`) whole — the decoder file
  was byte-identical on both branches, so the ports now share one seam — and `b825cb2d` makes that
  fallback reach a loader Android will not let it sniff for. See
  [The loader that cannot be sniffed for](#the-loader-that-cannot-be-sniffed-for).

  SVG needed a third path, and no new library either: see
  [SVG, which needed no new library](#svg-which-needed-no-new-library).
* **Eleven icons render as the missing-icon placeholder, because no icon theme is in the APK.**
  Reported from the emulator as "several of the symbols appear broken", the "Back to Latest"
  button's among them. It is not a font and it is not the SVG renderer: GTK 4.23 parses symbolic
  SVGs itself (`gtk/svg/`), and everything Commune ships in its own gresource draws correctly.

  There is no `share/icons` in the installed application at all —
  `run-as io.github.steeb_k.commune ls files/share` gives `commune fontconfig glib-2.0 gtk-4.0
  xml` and nothing else — because `adwaita-icon-theme` is not among the wraps in
  `build-aux/android/io.github.steeb_k.Commune.xml`, and nothing else supplies one. On a desktop it
  is simply a package that is always installed, which is why this has never come up before.

  So an icon resolves only if Commune ships it (57 in `data/resources/icons/`) or GTK ships it
  builtin (127 in `gtk/icons/`). Of the 17 names the UI asks for that Commune does not ship, six
  are in GTK's set and render — `list-add`, `object-select`, `pan-down`, `user-trash`,
  `view-fullscreen`, `view-more` — and eleven resolve to nothing:

  | Icon | Where it shows |
  | --- | --- |
  | `go-last-symbolic` | the "Back to Latest" button, the reported one |
  | `document-edit-symbolic` | edited-message marker |
  | `view-more-horizontal-symbolic`, `view-restore-symbolic` | menus and window controls |
  | `image-missing-symbolic` | the placeholder for a picture that failed to load, itself missing |
  | `call-start`, `call-stop`, `camera-web`, `camera-disabled`, `microphone-disabled`, `audio-input-microphone` | calls, already gated out with GStreamer |

  Six of the eleven are the call and camera icons, which no reachable code draws while S4 is
  undone, so the visible damage was five.

  **Fixed** by wrapping `adwaita-icon-theme` rather than by copying the handful of SVGs into
  `data/resources/icons/`. Copying would have been smaller and would have made Commune ship icons
  that shadow the system theme on Linux for no Android-only reason; the wrap keeps working as the
  UI grows and costs 91 KB in the APK. See
  [The icon theme, which is a wrap and two patches](#the-icon-theme-which-is-a-wrap-and-two-patches).
* **The TLS trust roots are read from the filesystem rather than verified by Android.** Deliberate,
  measured, and narrower than the platform verifier in ways written down in `src/utils/tls.rs` and
  in [The TLS that never returned](#the-tls-that-never-returned). Replacing it needs the Kotlin
  component vendored into the APK.

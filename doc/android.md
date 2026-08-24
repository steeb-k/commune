# Commune on Android

This is the ledger for the Android port: what exists, what was measured, and what bit us on the
way. `doc/android-plan.md` is the route that was intended; this file is what actually happened.

The port is **exploratory**. Nothing of Commune runs on Android yet. What S0 established is that
the toolchain works and that GTK itself is in better shape on Android than expected.

## Contents

<!-- toc -->
* [State today](#state-today)
* [The build host](#the-build-host)
* [Setting the environment up](#setting-the-environment-up)
* [Building a demo APK](#building-a-demo-apk)
* [What S0 measured](#what-s0-measured)
* [Findings that change the plan](#findings-that-change-the-plan)
* [Known gaps](#known-gaps)
<!-- /toc -->

## State today

| Spike | State |
| --- | --- |
| S0 — pixiewood baseline | **done**, gtk4-demo runs on the emulator |
| S1 — Rust hello-world APK | not started; the make-or-break spike |
| S2 — Commune `cargo check` for Android | not started |
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
  libglib2.0-dev-bin libxml2-utils ninja-build sassc python3-pip pipx git unzip curl
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

**6. Every wrap tracks a moving target.** `glib`, `gtk`, `libadwaita`, `cairo`, `harfbuzz` and
`fontconfig` are all `revision = main` (or `master`) at `depth = 1`. Two builds a week apart are
not the same build. When anything is packaged for a hand-out, the revisions have to be pinned and
recorded here.

## Known gaps

* **The soft keyboard does not hide.** Once shown it stays, through `ESC`, through the search
  being dismissed, and through rotation — where it covers most of a landscape screen. Only the
  system's own dismiss chevron or BACK puts it away. For a chat app, whose composer takes focus
  constantly, this needs a proper look before Route A is committed to.
* The IME comes up unbidden on launch.
* 136 MB debug APK, unmeasured stripped size.
* No Rust story (S1).
* GStreamer is not built at all: pixiewood's cross file sets `media-gstreamer = 'disabled'` for
  GTK, so voice messages, video and calls are all out of reach until S4.

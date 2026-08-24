# Plan: porting Commune to Android

This is an **exploration**, not yet a plan of record. It records what was established on 23 August
2026 before any spike was built, and the questions that decide whether the GTK codebase goes to
Android as-is or whether a Kotlin client is written against the shared Rust core instead. When
spikes start, `doc/android.md` becomes the ledger of what actually exists.

All of this work lives on the `android-port` branch (worktree `../commune-android`), based on
`main`. It is expected to move much more slowly than the macOS and Windows ports, and to be
rebased onto `windows-port` once that lands — it needs the same "gate at Linux, not at not-macOS"
build-system changes.

## Contents

<!-- toc -->
* [What exists upstream](#what-exists-upstream)
* [What exists on this machine](#what-exists-on-this-machine)
* [How a GTK app is entered on Android](#how-a-gtk-app-is-entered-on-android)
* [What this changes for Commune](#what-this-changes-for-commune)
* [The two routes](#the-two-routes)
* [Decisions](#decisions)
* [Spike ladder](#spike-ladder)
* [Open questions](#open-questions)
<!-- /toc -->

## What exists upstream

* **GTK has an in-tree Android backend** (`gdk/android/`, by Florian "sp1rit"), present since
  the 4.18 line and actively maintained: SDK 36 target (4.21.x), IME and popup fixes (4.21.3),
  Adreno fixes (4.19.3), vsync-aligned frame timing (4.23.1), `content://` URIs accepted by
  `g_file_new_for_uri` (4.23.3). Java glue ships in-tree:
  `ToplevelActivity`, `RuntimeApplication`, `ImContext`, `ClipboardProvider`,
  `SystemFilesystem`, `GlibContext`. Upstream calls it experimental.
* **pixiewood** (`sp1ritCS/gtk-android-builder`) builds an existing **Meson** app into an APK:
  `prepare` drops wraps for glib/cairo/harfbuzz/fontconfig/gdk-pixbuf/rsvg/gtk/libadwaita into
  `subprojects/` and writes NDK cross files (API 31, NDK `27.2.12479018` in GTK's own CI — the
  same NDK this machine has); `generate` emits a Gradle project from XSLT templates; `build`
  runs one ninja per arch, `meson install --destdir root --tags runtime`, then Gradle.
  `root/lib` is symlinked to `jniLibs/`; everything else under `root/` (gresources, locale,
  icons, gtksourceview data…) is copied to APK `assets/`; schemas are compiled into
  `assets/share/glib-2.0/schemas`. The app must be `executable(..., android_exe_type:
  'application')` (Meson ≥ 1.9) exposing `main`. Linux host, Perl, Java 17.
* **Rust is the gap.** gtk-rs issue #1997 (sp1rit, March 2025, still open, no comments): the
  Android build wants a single-pass ninja over app + all subprojects, which pkg-config-driven
  `-sys` crates cannot join, and Rust does not export `main`. Suggested route: build the Rust
  crate as a **staticlib**, let Meson link it into the `android_exe_type` shared object with a
  C stub, with the subprojects' link flags supplied by Meson. Nobody has published a working
  Rust GTK APK. Every app on gtk4android.geopjr.dev is C or Vala (Tuba, Elastic, Adwaita demo,
  gtk4-demo).
* **matrix-rust-sdk on Android is proven** (Element X is built on it, via UniFFI + cargo-ndk).
  The `rustls-aws-lc-rs` provider is the known irritant on Android cross builds (aws-lc-rs
  #918); the `rustls-platform-verifier`/`ndk-context` JVM-context dance seed-sync already went
  through applies here too.

## What exists on this machine

* SDK `C:\Android\Sdk`: platforms 34/35, build-tools 34/35, NDK `27.2.12479018`, emulator with
  AVD `seed_api35` (x86_64), platform-tools. JDK 17 (Adoptium). `wsl` Ubuntu-24.04 with ninja,
  perl, gcc, 900 GB free — but **no meson, no java, no NDK** inside WSL yet.
* rustup (MSVC host) already has `aarch64-linux-android`, `armv7-linux-androideabi`,
  `x86_64-linux-android`; `cargo-ndk` and the Gradle + UniFFI pattern live in
  `~/seed-sync/{android,crates/seed-mobile,scripts/run-android.ps1,docs/android-packaging.md}`.
  Its self-signed release keystore convention (`android/keystore.properties`, gitignored) is the
  house pattern for signing.
* pixiewood needs a Linux host. WSL is the candidate; the emulator stays on the Windows side
  and `adb` reaches it over the WSL network (or the APK is copied out and installed with the
  Windows `adb`).

## How a GTK app is entered on Android

`RuntimeApplication` (Java) does `System.loadLibrary("gtk-4")`, reads the manifest metadata
`gtk.android.lib_name`, extracts `assets/` to the app's files dir (`SystemFilesystem.
writeResources`), and calls native `startRuntime`. `gdkandroidruntime.c` then
`g_module_open`s the app library, looks up the exported symbol **`main`**, sets
`XDG_DATA_DIRS`/`XDG_DATA_HOME`/`XDG_CONFIG_*` via `g_set_user_dirs()` to
`getFilesDir()/share` etc., routes `g_print`/`g_log` to logcat, and runs `main` on a **separate
"GTK thread"** with `argv = {arg0}`. There is no process `main`; there is no console; there is
no `argv` to speak of.

Consequences for us: `main.rs` becomes a library entry point; `tracing` must go to logcat
(`paranoid-android` or `tracing-android`), not stdout; everything path-shaped is relative to
`getFilesDir()`; the process may be killed at any time by the OS, and the Activity is
recreated on rotation — `ToplevelActivity` handles that below GTK, but session state must
survive it.

## What this changes for Commune

Codebase today: 110k lines of Rust, 160 Blueprint files, ~2k lines of `main`/`application`.
Platform seams already exist and are all `cfg_if!`-shaped: `src/secret/mod.rs`,
`src/components/camera/mod.rs`, `src/utils/location/mod.rs`, `src/system_settings/mod.rs`,
`src/utils/media/image/decoder/` (glycin on Linux, `image` crate elsewhere),
`src/session/notifications/mod.rs`, `src/utils/app_bundle.rs`, `src/utils/mod.rs` (data dirs),
and the media backend seam in `src/components/media/`. Android is `target_os = "android"`,
which is **not** `linux`, so every `not(target_os = "linux")` fallback already applies to it —
the same free ride Windows got from macOS. What is _not_ free:

| Area | Linux today | Android |
| --- | --- | --- |
| Entry point | `fn main()` binary | `#[no_mangle] extern "C" fn main(argc, argv)` in a `cdylib`/`staticlib`, no `windows_subsystem` analogue |
| Logging | stdout | logcat sink |
| gresources, locale | absolute `PKGDATADIR` | `app_bundle.rs` android arm: `$XDG_DATA_HOME/commune/*.gresource` (assets extracted by the glue) |
| Data/cache dirs | XDG | `getFilesDir()`/`getCacheDir()` — GLib's XDG answers are already redirected by the glue; verify `user_cache_dir()` |
| Secrets | oo7 / Secret Service | `UnimplementedSecret` panics. Needs Android Keystore via JNI, or (spike) a file under `getFilesDir()` with the SDK's store encryption — SecretFile already exists for tokens |
| Notifications | GNotification (D-Bus) | none in GLib; `NotificationManager` via JNI, plus FCM/UnifiedPush for background — the largest Android-only chunk |
| Media | GTK GStreamer backend | pixiewood's GTK is built `-Dmedia-gstreamer=disabled`; GStreamer itself must be cross-built (Cerbero) — voice messages, video, and **calls** all hang on it |
| Camera | aperture/PipeWire | stub; later `ahcsrc` (GStreamer's Android camera source) |
| Location | ashpd portal | stub (already) |
| Images | glycin | `image` crate — fine; SVG/HEIC/AVIF/JXL unsupported as on macOS/Windows |
| Message search | `experimental-search` (SQLite) | should work; `sqlite` bundled by the SDK |
| SSO | localhost redirect server | must become an intent-filter redirect (`commune://` or `matrix:` scheme) — `src/login/local_server.rs` cannot receive a browser redirect on Android |
| gtksourceview, libshumate | packaged | **no pixiewood wraps** — must be added (both are Meson projects, so feasible) or the features gated out for the spike |
| Background sync | process lives | foreground service or push-only; the app is frozen when backgrounded |
| Text input | GTK IM | GTK's Android `ImContext` — exercise the composer hard (emoji, autocorrect, Blueprint `TextView`s) |
| Adaptive UI | libadwaita breakpoints, already good on phones | same widgets; the win of this route |

## The two routes

**Route A — GTK on Android (pixiewood).** One codebase, all the UI carried over, Linux-mobile
adaptivity reused. Cost: we are the first Rust app through this path; GStreamer and two
Meson libraries must be cross-built; notifications, keystore, SSO redirect and background
sync are JNI work; the result is a 60–100 MB APK with a non-native feel (no predictive back,
no Material, GTK IME), and "experimental" upstream.

**Route B — Kotlin/Compose over a shared Rust core (the seed-sync shape).** The Matrix logic
is `matrix-sdk` + our `src/session/**` model layer; the UI is `src/session_view/**`,
`components/**`, Blueprints. A UniFFI facade over the session model gives Compose a typed API;
Element X shows the SDK side works. Cost: the whole UI rewritten, a second UI to keep in
lockstep, and our model layer is GObject-shaped (`glib::Object` subclasses, `ListModel`s,
signals) — it is not headless like `seed-core`. Extracting a GObject-free core is itself a
large refactor and would touch every feature ledger under `doc/`.

The honest middle: **Route A's spikes cost days and answer the question empirically; Route B
costs months and is where we land if A fails.** Nothing in Route A is wasted if it fails —
the Android `cfg` arms (data dirs, logging, secrets, SSO redirect) are needed by Route B too,
because the Rust core still runs on Android either way.

## Decisions

Made on 23 August 2026, before any spike:

* **Host: WSL Ubuntu-24.04**, already on the machine, with the emulator (`seed_api35`) on the
  Windows side. pixiewood runs in WSL; the APK is installed with the Windows `adb`.
* **Architecture: `x86_64` first**, because that is what the emulator runs and nothing about
  the route is arch-specific. `aarch64` is added the day a device is plugged in.
* **Gate features to reach a baseline.** Unless a spike finds a structural impediment, the
  first Commune APK builds without gtksourceview, libshumate, GStreamer, calls, camera and
  location, behind one Cargo feature (working name `android-spike`) and the matching Meson
  option. Every gate is temporary and is listed in `doc/android.md` until it is removed.
* **Secrets: plain file first, Keystore the moment the route is chosen.** During spikes the
  session secret lives in a file under `getFilesDir()` (Android sandboxes it per app, and the
  `SecretFile` machinery already exists). Android Keystore via JNI is the first task of S5
  and is not optional: if it turns out easy it is done in S3, if not it is planned before any
  build is handed to anyone.
* **Background sync and push are in the same bucket as secrets:** a push-less "syncs while
  open" client is acceptable for the spikes and for a first hand-out, and the foreground
  service / UnifiedPush work is planned the moment the route is chosen.
* **Distribution: sideload**, signed with a self-signed keystore following the seed-sync
  pattern (`android/keystore.properties`, gitignored; key backed up offline). The Play Store
  is not a goal; its constraints (16 KB page alignment, target-SDK currency) are noted, not
  designed for.

## Spike ladder

Each spike is a yes/no gate; stop at the first no and record it in `doc/android.md`.

* **S0 — pixiewood baseline in WSL.** **Done, 23 August 2026** — results in `doc/android.md`.
  Nothing of Commune's; establishes the toolchain and the emulator round-trip. Half a day.
  1. In WSL: `apt install meson ninja-build openjdk-17-jdk-headless sassc libxml2-utils
     libglib2.0-dev-bin gettext` plus the Perl modules pixiewood's README lists for Debian;
     check `meson --version` ≥ 1.9 (use `pip install meson` if Ubuntu's is older).
  2. NDK and SDK: bind the Windows ones (`/mnt/c/Android/Sdk`) rather than downloading twice;
     if NTFS-over-9p makes ninja crawl, copy `ndk/27.2.12479018` and `platforms;android-35`
     and `build-tools;35.0.0` into `~/android` and record which.
  3. `git clone sp1ritCS/gtk-android-builder`, `make prefix=$HOME/.local install`.
  4. Build gtk4-demo from GTK's own manifest (`build-aux/android/org.gtk.Demo4.xml`,
     `-Dandroid-backend=true`), x86_64 only; then the Adwaita demo (pixiewood's libadwaita wrap).
  5. Install on `seed_api35` with the Windows `adb` (the APK is reachable at
     `\wsl$\Ubuntu-24.04\...`), screenshot both, exercise: text entry with the soft keyboard,
     rotation, a popover, a file dialog, backgrounding and returning.
  6. New `build-aux/android/probe-env.sh` in the shape of the macOS and Windows probes, and
     the start of `doc/android.md` with the versions, the manifest used, the wall-clock of the
     GTK build, and what the demos got wrong on the emulator.
* **S1 — Rust hello-world APK.** **Read `doc/android.md` first.** S0 changed this spike's
  starting assumptions: stock meson already has `android_exe_type`, libadwaita already
  carries the guard that uses it, and `meson setup` writes a complete set of
  `*-uninstalled.pc` files — `gtk4-uninstalled.pc` among them — before ninja runs a single
  command, which is most of what step 2 below expected to have to discover.
  The make-or-break spike, in a throwaway repo outside
  Commune (`~/gtk-android-rust-spike`), so nothing is designed before it is known to link.
  One to three days.
  1. A Meson project with `subprojects/` prepared by pixiewood (glib, cairo, harfbuzz,
     fontconfig, gdk-pixbuf, gtk, libadwaita) and one `executable('spike', 'stub.c',
     android_exe_type: 'application', link_with: rust_lib, dependencies: [gtk, adw])`.
     `stub.c` is `int main(int argc, char **argv) { return spike_main(argc, argv); }`.
  2. `rust_lib` is a `custom_target` running `cargo build --target x86_64-linux-android` on a
     `staticlib` crate whose `spike_main` opens an `adw::ApplicationWindow` with a button. The
     order problem from gtk4-rs #1997 is attacked head-on: Meson configures the subprojects
     first, so their `meson-uninstalled/*.pc` files exist under the build dir; the
     `custom_target` sets `PKG_CONFIG_PATH`/`PKG_CONFIG_SYSROOT_DIR` at those, plus the NDK
     `CC`/`AR`/linker vars `cargo-ndk` would set. The `-sys` crates then see the exact GTK the
     APK ships. If the `.pc` files are not there at that point in the ninja graph, the fallback
     is `PKG_CONFIG_ALLOW_CROSS=1` with hand-written `.pc` stubs — link flags are all Meson's
     anyway, since the C stub is what links.
  3. Known traps to hit deliberately: `-sys` crates' `system-deps` version checks against
     the subproject's GTK; the two-cargo-runs problem (a `custom_target` is not incremental —
     acceptable for a spike); `aws-lc-sys` does not exist yet here, `ring` is not needed yet;
     `libgcc`/`libunwind` symbols on x86_64-android; `main` visibility (`G_MODULE_EXPORT`,
     `-rdynamic` or a version script).
  4. Pass: the button toggles a label on the emulator; `adb logcat` shows a `tracing` line
     through an `android_log` sink; rotation does not crash. Fail after three days: the
     finding goes to `doc/android.md`, a comment goes on gtk4-rs #1997, and Route B is costed.
* **S2 — Commune compiles for `x86_64-linux-android`.** `cargo check` only, no APK: add the
  android cfg arms (entry, logging, data dirs, `UnimplementedSecret` → a temporary file
  secret), gate gtksourceview/shumate behind a feature, take the `aws-lc-rs` → `ring` fallback
  if needed. Measures how much of the 110k lines is actually platform-dirty.
* **S3 — Commune login on the emulator.** Cross-build gtksourceview5 and libshumate as wraps
  (or keep them gated), no GStreamer, password login against `testing/local-homeserver.sh`,
  send a message, see the timeline. This is the "is the UI usable on a phone with GTK's
  IME" test. Screenshots into the ledger.
* **S4 — GStreamer.** Cerbero Android build, or gst-plugins-rs' Android CI artifacts; voice
  message playback first, calls last.
* **S5 — the Android-only features**: keystore secrets, `NotificationManager`, SSO via
  intent-filter, foreground service / push. Only after S3 says the route is worth it.

## Open questions

The questions that shaped the decisions above are answered; what remains is answered by the
spikes themselves and recorded in `doc/android.md`:

* Whether a Rust `staticlib` can be linked into pixiewood's single ninja pass at all (S1).
* Whether GTK's Android `ImContext` is good enough for a chat composer (S3).
* Whether the session model, being GObject-shaped, would give a Kotlin client anything to
  reuse beyond the SDK pins — only relevant if Route A fails.

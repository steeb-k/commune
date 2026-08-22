# Plan: porting Commune to macOS

This is the working plan for the macOS port, written before any of it was built. It produces an
unsigned, relocatable `Commune.app` and `.dmg` from a developer's Mac. When work starts,
`doc/macos.md` becomes the ledger of what actually exists (build steps, environment, what is
stubbed); this file records the intended route and is updated as decisions change.

## Contents

<!-- toc -->
* [Context](#context)
* [M0 — `cargo check` and `clippy` pass on macOS](#m0--cargo-check-and-clippy-pass-on-macos)
* [M1 — runs from the Meson dev build](#m1--runs-from-the-meson-dev-build)
* [M2 — unsigned `Commune.app` and `.dmg`](#m2--unsigned-communeapp-and-dmg)
* [M3 — polish](#m3--polish)
* [M4 — stretch: camera QR scanning](#m4--stretch-camera-qr-scanning)
* [Dependencies and from-source recipes](#dependencies-and-from-source-recipes)
* [Docs and ledger updates](#docs-and-ledger-updates)
* [Risks and open questions](#risks-and-open-questions)
* [Critical files](#critical-files)
<!-- /toc -->

## Context

Commune is a permanent fork of Fractal 14.1 (`doc/fork.md`): Rust + GTK4/libadwaita, Meson driving
Cargo, shipped today only as Linux/Flatpak. The code already has platform seams — `cfg_if!` aliases
in `src/secret/mod.rs`, `src/components/camera/mod.rs`, `src/utils/location/mod.rs`,
`src/system_settings/mod.rs` — but the non-Linux branches are stubs that panic (secrets), don't
compile (`Location::new()`), or don't exist (glycin image decoding is unconditional). Build-time
absolute paths (`PKGDATADIR`, `LOCALEDIR`) and Linux-only install artifacts (desktop/D-Bus files)
also block a relocatable bundle.

Decisions already made:

* Local Mac for development; **no Homebrew for GTK**. An existing GTK dev environment of _unknown
  type_ (jhbuild/MacPorts/Nix/custom) is on the machine. Everything is prefix-agnostic:
  `GTK_PREFIX=$(pkg-config --variable=prefix gtk4)`, driven by `PKG_CONFIG_PATH`. Homebrew or
  cargo only for non-GTK tooling, optionally.
* Secrets go to the macOS Keychain via `security-framework` (already in `Cargo.lock`, 3.7.0).
* Deliverable is an unsigned, relocatable `Commune.app` + `.dmg` (no notarization, no CI for v1).
* Dropped or stubbed in v1: location sharing, system 12/24h clock (both already degrade
  gracefully).
* Kept on the list, sequenced last: native notifications, camera QR scanning.

Project rules that constrain every step: the gresource prefix stays `/org/gnome/Fractal/`;
anything carrying the app name or ID is recorded in `doc/rebrand.md`; docs are ledgers in `doc/`;
the pre-commit hook (`hooks/checks/src/main.rs`) enforces nightly rustfmt, clippy pedantic, typos,
cargo-machete, cargo-deny (`deny.toml` targets are Linux-only today), `cargo-sort --grouped`, a
sorted `src/ui-blueprint-resources.in`, and rumdl.

## M0 — `cargo check` and `clippy` pass on macOS

Linux must stay untouched in behaviour.

1. **Inventory the Mac environment.** New `build-aux/macos/probe-env.sh` (POSIX sh, read-only,
   prints `name | found | version | required | status`, ends with "needs building from source:").
   It reports: `GTK_PREFIX`, `PKG_CONFIG_PATH`, `GI_TYPELIB_PATH`, `XDG_DATA_DIRS`;
   `pkg-config --modversion` / `--atleast-version` for every dependency in `meson.build:25-42`
   (glib/gio ≥ 2.82, gtk4 ≥ 4.20.2, libadwaita-1 ≥ 1.8, gstreamer app/base/pbutils/play/video
   ≥ 1.20, gtksourceview-5, libwebp, shumate-1.0 ≥ 1.1, sqlite3) plus libsoup-3.0,
   gdk-pixbuf-2.0, librsvg-2.0, `intl`/`libintl.h`, `msgfmt`; `gst-inspect-1.0` for
   `gtk4paintablesink`, `avfvideosrc`, `avfdeviceprovider`, `zbar`, `uridecodebin(3)`, `level`,
   `playbin3`, `videoconvert`, `audioconvert`, `osxaudiosink`, `vtdec_hw`/`avdec_h264`,
   `webpdec`; `pkg-config --variable=pluginsdir/pluginscannerdir gstreamer-1.0`; the libshumate
   vector renderer (`grep ShumateVectorRenderer …/Shumate-1.0.gir` or
   `nm -gU libshumate-1.0.dylib | grep shumate_vector_renderer_new`); `gdk-pixbuf-query-loaders`
   (svg/png/jpeg) and its cache/moduledir variables; `giomoduledir` and the presence of a TLS
   module (glib-networking); the layout of `share/glib-2.0/schemas/gschemas.compiled`,
   `share/icons/{Adwaita,hicolor}`, `share/gtksourceview-5`, `share/mime/mime.cache`,
   `etc/fonts`; tools: meson ≥ 1.4, ninja, rustup cargo ≥ 1.95 + nightly rustfmt, cargo-nextest,
   glib-compile-resources/schemas, blueprint-compiler, grass or sass, dylibbundler, create-dmg
   (optional), rsvg-convert, iconutil, hdiutil, codesign. Paste the output into `doc/macos.md`.
2. **Gate the build system.** `meson.build:37-38`: wrap `dependency('glycin-2')` and
   `dependency('glycin-gtk4-2')` in `if host_machine.system() != 'darwin'`. `data/meson.build`:
   wrap the desktop-file block (and its validate test) and the D-Bus service block the same way;
   keep metainfo and gschema (the schema is required at runtime everywhere,
   `src/application.rs:47` aborts without it). `src/config.rs.in`: `#[cfg(target_os = "linux")]`
   on `DISABLE_GLYCIN_SANDBOX` (dead-code lint otherwise). Keep `PKGDATADIR`, `LOCALEDIR` and
   `RESOURCES_FILE` as the non-bundle fallback.
3. **Location stub.** `src/utils/location/mod.rs:36-37`: add
   `impl UnimplementedLocation { pub(crate) fn new() -> Self { Self } }`. The call sites at
   `src/session_view/room_history/message_toolbar/mod.rs:202,629,866` stay unchanged.
4. **gettext.** `Cargo.toml` unchanged (`gettext-rs` with `gettext-system`). Verify what
   `gettext-sys` 0.27 does on macOS: expected, without env vars it builds a vendored static
   libintl (needs Xcode CLT `cc`/`make`); with `GETTEXT_SYSTEM=1 GETTEXT_DIR=$GTK_PREFIX` it links
   the prefix's libintl. Have `meson.build` export the latter into `cargo_env` on darwin when
   `dependency('intl')` is found; `probe-env.sh` prints the export line for bare cargo use.
5. **Image decoder abstraction** (the hard blocker). New
   `src/utils/media/image/decoder/{mod.rs,glycin.rs,image_rs.rs}`; `mod.rs` is a `cfg_if!`
   re-export (Linux → glycin, else image_rs). The shape mirrors exactly what
   `src/utils/media/image/mod.rs` and `src/components/media/animated_image_paintable.rs` use:
   * `Loader::for_bytes(glib::Bytes)`, `Loader::for_file(&gio::File)`,
     `async fn load(self) -> Result<Image, Error>` (Linux: applies
     `SandboxSelector::NotSandboxed` when `DISABLE_GLYCIN_SANDBOX`, then `load_future()`).
   * `Image: Clone` with `width()`, `height()`, `async next_frame() -> Result<Frame, Error>`,
     `async specific_frame(&FrameRequest)`.
   * `Frame: Clone + Debug` with `width()`, `height()`, `delay() -> Option<Duration>`,
     `texture() -> gdk::Texture` (Linux keeps the "0 µs means still" rule from
     `GlycinFrameExt`, `mod.rs:1050-1063`; `texture()` is `glycin_gtk4::frame_get_texture`).
   * `FrameRequest::new().with_scale(w, h)`; `Error: Debug + Display` with
     `is_unknown_format()` (Linux: `glycin::LoaderError::UnknownImageFormat`, today at
     `mod.rs:1029`).
   * `image_rs.rs` uses the pure-Rust `image` crate. Chosen over gdk-pixbuf: no loader modules
     or `loaders.cache` rewriting, has animated WebP/APNG, its frame-pull API matches, and it is
     memory-safe — the closest analogue to glycin's sandbox. Decode on `RUNTIME.spawn_blocking`
     (pattern at `image/mod.rs:639`): `ImageReader::with_guessed_format()` → `into_decoder()`,
     read `orientation()` and `dimensions()`, `DynamicImage::apply_orientation`, texture via
     `gdk::MemoryTexture::new(w, h, R8g8b8a8, Bytes, w * 4)`. Animated (gif/webp/png): keep
     `Frames` from `AnimationDecoder::into_frames()` over an owned `Cursor<Arc<[u8]>>`; on
     exhaustion rebuild and loop (glycin loops; `animated_image_paintable.rs:200-214` relies on
     it); delay from `image::Delay::numer_denom_ms`, clamp 0 → 100 ms. `specific_frame` is the
     first frame plus `DynamicImage::thumbnail(w, h)` when smaller than natural size.
     **Unsupported on macOS v1: SVG, HEIC, AVIF, JXL** → `ImageError::UnsupportedFormat`.
   * Edit `src/utils/media/image/mod.rs`: every `glycin::` → `decoder::`; `into_loader`
     (`:98-111`) drops the sandbox branch; `Image { decoder, first_frame }` (`:162-172`);
     `Frame::Glycin` (`:339`) → `Frame::Decoded`; `to_image_loader_request` (`:456`) returns
     `decoder::FrameRequest`; `From<glib::Error> for ImageError` (`:1026`) →
     `From<decoder::Error>`; delete `GlycinFrameExt` (`:1037-1067`), use `delay().is_some()`,
     `delay()`, `texture()`. Re-word the "glycin" comments (`:66-68`, `:179-183`, `:206-209`,
     `cache.rs:3`, `queue.rs:420,472`).
   * Edit `animated_image_paintable.rs`: import `decoder::{Frame, Image}`; `:51` and `:60` use
     `Frame::height`/`Frame::width`; `update_animation` uses `Frame::delay()`.
   * `Cargo.toml`: move `glycin` and `glycin-gtk4` into
     `[target.'cfg(target_os = "linux")'.dependencies]`; add
     `image = { version = "0.25", default-features = false, features = [...] }` with
     `bmp, gif, ico, jpeg, png, tiff, webp` under
     `[target.'cfg(not(target_os = "linux"))'.dependencies]` (confirm the resolved 0.25.x has
     `ImageDecoder::orientation`, else bump).
6. **Secrets.** New `src/secret/macos.rs`; `src/secret/mod.rs` `cfg_if!` gains a `macos` branch
   (`Secret = macos::MacosSecret`); `unimplemented` stays for other OSes. The structure mirrors
   `linux.rs` (`restore_sessions_inner`, `store_session_inner`, `delete`, all via `spawn_tokio!`):
   * Generic-password items (`security_framework::item::{ItemSearchOptions, ItemAddOptions,
     ItemClass, Limit, SearchResult}`, `passwords::delete_generic_password`);
     `service = APP_ID` (already profile-distinct), `account = session.id`, label
     `"Commune: Matrix credentials for {user_id}"` (same string as `linux.rs:180-184`).
   * The secret payload is `serde_json` of `KeychainSecret { version: 1, homeserver, user_id,
     device_id, id, client_id, passphrase: Zeroizing<String> }` — the Keychain has no free-form
     attribute search, so attributes ride inside the secret.
   * `restore_sessions`: search `service = APP_ID`, `load_data` + `load_attributes`,
     `Limit::All`; `errSecItemNotFound` → `Ok(vec![])`; skip items with bad JSON or a newer
     version (log).
   * Errors → `SecretError::Service(error.to_string())`. Tokens stay in `SecretFile`
     (unchanged).
   * `Cargo.toml`: `security-framework = "3"` under
     `[target.'cfg(target_os = "macos")'.dependencies]`.
7. **Data dirs** (now, before any macOS user data exists). `src/utils/mod.rs:62-73`
   `DataType::dir_path()`: `#[cfg(target_os = "macos")]` → `glib::home_dir()` +
   `Library/Application Support` or `Library/Caches`, then `PROFILE.dir_name()` (unchanged).
8. **Modifier constant.** `src/utils/key_bindings.rs`: add `pub(crate) const PRIMARY_MASK`,
   `META_MASK` when `cfg!(target_os = "macos")`, `CONTROL_MASK` otherwise. The sweep is M3.
9. `deny.toml` `[graph] targets` gains `aarch64-apple-darwin` and `x86_64-apple-darwin`.
10. `Cargo.toml` target tables in cargo-sort order: `cfg(not(target_os = "linux"))` (image),
    `cfg(target_os = "linux")` (aperture, ashpd, glycin, glycin-gtk4, oo7),
    `cfg(target_os = "macos")` (security-framework); run the cargo-sort check.

Verify on the Mac: `sh build-aux/macos/probe-env.sh`; `meson setup _build -Dprofile=development`;
`CARGO_TARGET_DIR=_build/cargo-target cargo check && cargo clippy --all-targets -- -D warnings`;
`cargo +nightly fmt --check --all`; `cargo deny check`; `cargo machete --with-metadata`; the
cargo-sort check; `typos`; `rumdl check .`. Verify on Linux/Flatpak: clippy still clean, behaviour
unchanged.

## M1 — runs from the Meson dev build

Goal: log in, timeline images work.

1. **Runtime paths.** New `src/utils/app_bundle.rs`: `RuntimePaths { resources_file,
   ui_resources_file, localedir }`; non-macOS returns the compile-time consts. On macOS, if
   `current_exe()` is under `*.app/Contents/MacOS/`, the root is `Contents/Resources` →
   `share/commune/*.gresource`, `share/locale`; and, **first thing in `main()`** (edition-2024
   `unsafe { env::set_var }` with a `// SAFETY:` comment — no threads yet, the `RUNTIME`
   `LazyLock` untouched) set `GSETTINGS_SCHEMA_DIR`, `GDK_PIXBUF_MODULE_FILE`,
   `GST_PLUGIN_SYSTEM_PATH_1_0`, `GST_PLUGIN_SCANNER_1_0`, `GIO_MODULE_DIR`, `XDG_DATA_DIRS`
   (prepend `Resources/share`), `GTK_DATA_PREFIX`, `GTK_EXE_PREFIX`, and `FONTCONFIG_FILE` if
   bundled. `main.rs:62` → `bindtextdomain(GETTEXT_PACKAGE, paths.localedir)`; `main.rs:73-76` →
   `Resource::load(&paths.*)`. Not a launcher script: LaunchServices delivers `matrix:` Apple
   Events and the Keychain ACL identity to `CFBundleExecutable`; a wrapper breaks both.
2. `meson install -C _build` (prefix `~/.local` or `$GTK_PREFIX`); schemas get compiled by
   `gnome.post_install` (`meson.build:177`); the dev run needs
   `XDG_DATA_DIRS=$prefix/share:$GTK_PREFIX/share` for the app schema and Adwaita icons.
3. **Fix what the first run reveals**, each its own commit: `gtk4paintablesink` missing → recipe
   below; shumate vector renderer → recipe; `file_row.rs:113`
   `gio::AppInfo::launch_default_for_uri` → switch to `gtk::FileLauncher` unconditionally (modern
   API, fine on Linux too).

Verify: password login; SSO login (`src/login/local_server.rs` binds localhost — expect a firewall
prompt); send and receive text; image thumbnail, animated GIF, sticker pack; video plays; Keychain
Access shows the item under service `io.github.steeb_k.Commune.Devel`; quit and relaunch restores
the session; `meson test -C _build` passes.

## M2 — unsigned `Commune.app` and `.dmg`

Files: `build-aux/macos/{Info.plist.in, make-icns.sh, bundle.sh, make-dmg.sh, README.md}`;
`meson.build` darwin-only `run_target`s `macos-bundle` and `macos-dmg` (passing build root, app
ID, version, profile). `.gitignore` gains `*.app/`, `*.dmg`, `*.iconset/`.

1. `make-icns.sh <svg> <out.icns>`: `rsvg-convert` at 16…1024 (plus @2x) → iconset →
   `iconutil -c icns`. Sources `assets/appicon.svg` and `assets/appicon-devel.svg` (derived
   artifact, not committed).
2. `Info.plist.in` (meson-configured `@APP_ID@`, `@VERSION@`, `@APP_NAME@`). `CFBundleName` is
   what fixes the lowercase `commune` that macOS shows in the menu bar and the Dock today: with
   no bundle it has nothing to go on but the name of the executable, and
   `glib::set_application_name()` only reaches the items GTK builds itself, like "About Commune".
   `CFBundleIdentifier=@APP_ID@`, `CFBundleExecutable=commune`, `CFBundleName`/`DisplayName`,
   `CFBundleIconFile=commune.icns`, `CFBundleShortVersionString`/`Version`,
   `CFBundlePackageType=APPL`, `LSMinimumSystemVersion` (match the prefix; default 12.0),
   `NSHighResolutionCapable`, `CFBundleURLTypes` for `matrix`, `NSCameraUsageDescription` (for
   M4), `LSApplicationCategoryType=public.app-category.social-networking`,
   `NSPrincipalClass=NSApplication`.
3. `bundle.sh <build-dir> <out-dir>`: `DESTDIR=tmp meson install`, then assemble:

   ```text
   Commune.app/Contents/
     Info.plist, PkgInfo (APPL????)
     MacOS/commune                      # rpath @executable_path/../Frameworks
     Frameworks/*.dylib                 # all non-system dylibs
     Resources/commune.icns
     Resources/share/commune/{resources,ui-resources}.gresource
     Resources/share/locale/*/LC_MESSAGES/commune.mo
     Resources/share/glib-2.0/schemas/gschemas.compiled   # app + org.gtk.gtk4.* (+ others found)
     Resources/share/icons/{hicolor (app svg), Adwaita (symbolic + index.theme)}
     Resources/share/gtksourceview-5/, share/mime/
     Resources/lib/gdk-pixbuf-2.0/2.10.0/{loaders/*.so, loaders.cache (paths rewritten)}
     Resources/lib/gstreamer-1.0/*.dylib (needed subset) + gst-plugin-scanner
     Resources/lib/gio/modules/*.so (glib-networking)
     Resources/etc/fonts/ (only if fontconfig in play)
   ```

   Prefix-agnostic dylib walk: `dylibbundler -od -b -x MacOS/commune -x <each plugin>
   -d Contents/Frameworks -p @executable_path/../Frameworks -s $GTK_PREFIX/lib` (extra `-s` for
   Nix/MacPorts paths), then an `otool -L` audit loop over everything in `Contents/` asserting
   only `/usr/lib`, `/System`, `@executable_path`, `@rpath`, `@loader_path` remain
   (`install_name_tool` for stragglers). If the environment turns out to be jhbuild,
   `gtk-mac-bundler` is the alternative; `bundle.sh` stays primary (no extra tool). Ad-hoc sign:
   `codesign --force --deep -s ${CODESIGN_IDENTITY:--} Commune.app` (arm64 needs a signature to
   run; Keychain ACLs bind to it). The ad-hoc identity changes per build, so the Keychain asks
   to allow on each rebuild; the mitigation, documented in `doc/macos.md`, is a stable
   self-signed "Code Signing" certificate from Keychain Access and `CODESIGN_IDENTITY="Commune
   Dev"`.
4. `make-dmg.sh`: stage the app plus an `Applications` symlink →
   `hdiutil create -volname Commune -srcfolder stage -ov -format UDZO out.dmg` (`create-dmg`
   optional, for a background image).

Verify: `open ~/Desktop/Commune.app` in a clean shell (no `PKG_CONFIG_PATH` or `DYLD_*`); the
`otool -L` audit is clean; `DYLD_PRINT_LIBRARIES=1 …/commune | grep -v Commune.app` shows only
`/usr/lib` and `/System`; the M1 smoke list passes from the bundle; mount the dmg, copy to
`/Applications`, launch.

## M3 — polish

Shortcuts, the menu bar, notifications, URL scheme, file open.

0. **The menu bar**, macOS only. Everything in the hamburger menu is reachable only from a button
   in a sidebar that is already cramped, while the menu bar every other Mac application uses sits
   empty. GTK can fill it: `gtkapplication-quartz.c` turns the `GtkApplication` `menubar` into
   `[NSApp setMainMenu:]`, and builds the application menu itself — About, Preferences, Services,
   Hide, Quit — from `app.about`, `app.preferences` and `app.quit`.

   Three things this depends on:

   * **`app.preferences` does not exist.** GTK's application menu names it unconditionally, so
     the item is dead until `src/application.rs` has one. It should open the account settings of
     the current session.
   * **The hamburger's own items cannot be used as they are.** `session.create-direct-chat`,
     `session.create-room`, `session.join-room` and `session.open-image-packs` are installed with
     `klass.install_action` on the `SessionView` **widget**, and the global menu resolves through
     the application muxer plus the active window's `win.` group only — a widget's actions are
     not in it. Each one needs a `win.` action on `Window` forwarding to the visible session view.
   * **Setting a menubar replaces GTK's default one**, which is where the Edit and Window menus
     come from (`gtk/ui/gtkapplication-quartz.ui`). Ours has to carry both; Window is only
     `gtk-macos-special: window-submenu`.

   So: a menu model with File, Edit, View, Window and Help, set with `set_menubar()` in
   `Application::startup`; the `win.` forwarders; `app.preferences`; and the hamburger button
   hidden on macOS, since everything in it is then in the menu bar.

1. **Shortcuts sweep** to `<Primary>`: `src/application.rs:233-237` (`<Primary>q`, `<Primary>w`),
   `src/session_view/mod.blp:31-50` (four triggers), `src/shortcuts-dialog.blp` (`<ctrl>` →
   `<primary>`), Rust `CONTROL_MASK` at `src/window.rs:113` and
   `src/session_view/mod.rs:89-91,148-150,161-163,170-172` → `key_bindings::PRIMARY_MASK`.
   Afterwards `grep -rn "<Control>\|<ctrl>\|CONTROL_MASK" src` is empty (or a commented,
   deliberate binding).
2. **Notifications.** GLib's Cocoa backend auto-selects inside a bundle with
   `CFBundleIdentifier`; `src/session/notifications/mod.rs:114-148` is unchanged. Verify that a
   click fires `app.show-matrix-id` with its target; if the Cocoa backend drops the target, fall
   back to plain activation and note it.
3. **`matrix:` URL scheme.** `CFBundleURLTypes` (M2) plus `HANDLES_OPEN`/`open()`
   (`src/application.rs:127-141`). Verify `open 'matrix:…'` cold and warm. If GTK's macOS
   backend does not forward `kAEGetURL` to `GApplication::open`, add
   `src/utils/macos_url_events.rs` (an `NSAppleEventManager` handler →
   `Application::default().open(..)`; deps `objc2`, `objc2-foundation` in the macOS target
   table). Only if verification fails.
4. Verify that logging out removes the Keychain item and
   `~/Library/Application Support/commune-Devel/<id>` (`StoredSession::delete`,
   `src/secret/mod.rs`).

Verify: Cmd-Q/W/K/L/comma, Cmd-PgUp/PgDn (plus Shift); a background notification click opens the
room; a second launch with a `matrix:` URL focuses the existing instance and opens the room.

## M4 — stretch: camera QR scanning

Via GStreamer `avfvideosrc`.

1. New `src/components/camera/macos/{mod.rs,viewfinder.rs}`; `camera/mod.rs` `cfg_if!` adds
   `macos::MacosCamera`. Contract: `CameraExt` (`camera/mod.rs:26-36`) and a `CameraViewfinder`
   subclass (`CameraViewfinderImpl`, states `Loading → Ready/NoCameras/Error`,
   `emit_qrcode_detected(QrVerificationData::from_bytes(..))`) mirroring
   `camera/linux/viewfinder.rs`. `has_cameras()` → `gst::DeviceMonitor` with filter
   `Video/Source` and a 1 s timeout (as `linux/mod.rs:38`).
2. Pipeline: `avfvideosrc ! videoconvert ! tee name=t`, `t. ! queue ! gtk4paintablesink`,
   `t. ! queue leaky=downstream max-size-buffers=1 ! videoconvert ! video/x-raw,format=GRAY8 !
   appsink`. Decode QR with `rqrr` (already in `Cargo.lock`, 0.10.1, pure Rust) on
   `spawn_blocking`, throttled to about 5 fps. This avoids the `zbar` element, which most macOS
   GStreamer builds do not link; it is a one-line swap if present. `NSCameraUsageDescription` is
   mandatory.
3. `Cargo.toml`: `rqrr` in the macOS target table; README runtime-deps macOS paragraph.

Verify: Verification → Scan QR shows the camera; scanning another client's QR completes; the
camera is released on dialog close (`dispose` → `Null`).

## Dependencies and from-source recipes

Prefix-agnostic; build only what the probe flags.

Required in `$GTK_PREFIX`: glib/gio ≥ 2.82, gtk4 ≥ 4.20.2, libadwaita ≥ 1.8, gstreamer ≥ 1.20
with base/good/bad (`avfvideosrc`, `applemedia`, `osxaudio`), the gst-plugins-rs `gtk4` plugin,
gtksourceview-5, libshumate ≥ 1.1 **with vector renderer**, libsoup3, glib-networking, libwebp,
sqlite3, gdk-pixbuf + librsvg (SVG loader), adwaita and hicolor icon themes, shared-mime-info,
gettext tools, gobject-introspection, blueprint-compiler. Tooling: meson ≥ 1.4, ninja, rustup
stable plus nightly rustfmt, `cargo install cargo-nextest cargo-deny cargo-machete cargo-sort
typos-cli grass`, rumdl, dylibbundler, Xcode CLT.

All recipes: `meson setup _b --prefix=$GTK_PREFIX --libdir=lib …` with
`GI_TYPELIB_PATH=$GTK_PREFIX/lib/girepository-1.0`.

* **libshumate** (missing, or no `ShumateVectorRenderer`): tag 1.6.0,
  `-Dvapi=false -Dgtk_doc=false -Dvector_renderer=true` (same as the Flatpak module in
  `build-aux/io.github.steeb_k.Commune.json:74-89`); needs `protobuf-c` (manifest `:59-73`) and
  libsoup3. Homebrew's `libshumate` 1.6.3 exists but uses default options — the renderer is
  unverified there.
* **gtksourceview5**: gtksourceview.git tag ≥ 5.10, `-Dvapi=false -Dgtk_doc=false`.
* **gst-plugins-rs gtk4 plugin**: `cargo install cargo-c`; clone gst-plugins-rs at the branch
  matching the gst major.minor;
  `cargo cbuild -p gst-plugin-gtk4 --release --prefix=$GTK_PREFIX --libdir=$GTK_PREFIX/lib`
  then `cargo cinstall …`. Homebrew's `gstreamer` 1.28 builds with `-Drs=enabled
  -Dgst-plugins-rs:gtk4=enabled -Dbad=enabled` — reference only.
* **blueprint-compiler** (absent or old): blueprint-compiler.git tag ≥ v0.18, meson install.
* **grass**: `cargo install grass` (`data/resources/meson.build:3` prefers it).
* **glib-networking** (no TLS gio module): `-Dgnutls=disabled -Dopenssl=enabled`.

## Docs and ledger updates

* `doc/macos.md` (new, in the voice of `doc/flatpak.md`): probe output, the `$GTK_PREFIX`
  convention, recipes, build/run/bundle commands, Keychain prompt mitigation, what is stubbed
  (location, clock) and unsupported (SVG/HEIC/AVIF/JXL images), the env vars the bundle sets,
  rebase notes (the `main.rs` and `image/mod.rs` indirections must be re-applied).
* `doc/rebrand.md`: identifiers table gains `CFBundleIdentifier=@APP_ID@`, the macOS data and
  cache paths `~/Library/Application Support|Caches/commune[-Devel]`, Keychain service
  `APP_ID` plus the label string, `.icns` derived from `assets/appicon*.svg`; the renamed-files
  list gains `build-aux/macos/Info.plist.in` and `src/secret/macos.rs`.
* `README.md`: Building gains a "macOS" subsection pointing at `doc/macos.md`; Runtime
  dependencies gains a macOS paragraph.
* `CONTRIBUTING.md`: a one-line pointer. `.gitlab-ci.yml` untouched (a macOS job is optional,
  later).

## Risks and open questions

* GTK macOS backend quirks (renderer, IME, drag and drop) — track in `doc/macos.md`.
* GApplication `open()` via Apple Events is unverified → fallback coded in M3.3.
* GLib's Cocoa notification backend uses the deprecated `NSUserNotification`; target forwarding
  is unverified.
* HEIC and SVG images are unsupported on macOS v1 (an ImageIO-backed decoder is a later
  option).
* The `image` crate adds compile time; the release profile already peaks around 6.5 GB
  (`doc/flatpak.md`).
* Prefix surprises (jhbuild layout, MacPorts gio modules, Nix store paths) — the `otool -L`
  audit loop is the safety net; pass extra `-s` dirs to dylibbundler.

## Critical files

* `src/utils/media/image/mod.rs` (plus new `decoder/`),
  `src/components/media/animated_image_paintable.rs`
* `src/secret/mod.rs` (plus new `src/secret/macos.rs`)
* `src/main.rs` (plus new `src/utils/app_bundle.rs`), `src/utils/mod.rs`,
  `src/utils/key_bindings.rs`
* `Cargo.toml`, `deny.toml`, `meson.build`, `data/meson.build`, `src/config.rs.in`
* new `build-aux/macos/{probe-env.sh,Info.plist.in,make-icns.sh,bundle.sh,make-dmg.sh}`
* `doc/macos.md` (new), `doc/rebrand.md`, `README.md`

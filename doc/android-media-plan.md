# S4 — Media on Android: context and a plan

Written 25 August 2026, before any of it is built. This is the first piece of the port that
genuinely needs something cross-compiled that does not exist yet, so the decision matters more than
the code. Everything below marked _measured_ was checked in the tree; everything marked _unverified_
is a claim to test before relying on it.

## What is actually broken today

Three separate things, and they are not equally broken.

| | today on Android | why |
| --- | --- | --- |
| Video in the timeline | error row, _"Videos are not supported on this platform"_ | `build_video` is gated out |
| Video duration + thumbnail | absent — outgoing videos carry neither | `load_video_info` gated out |
| Voice-message duration + waveform | absent — outgoing voice messages carry neither | `load_audio_info` gated out |
| **Playing a voice message** | **the player appears and silently does nothing** | see below |

That last row is the one worth knowing about, because it is not documented anywhere and it is not a
gate.

`src/components/media/audio_player/` is **not** gated for Android — it compiles and the UI is built.
It gets its stream from a two-line seam:

```rust
/// Create a stream that plays the given file.
#[cfg(not(target_os = "macos"))]
fn media_stream_for_file(file: &gio::File) -> gtk::MediaStream {
    gtk::MediaFile::for_file(file).upcast()
}
```

`GtkMediaFile` only has a backend where GTK was built against GStreamer, and pixiewood builds GTK
with `media-gstreamer = 'disabled'`. So on Android that call returns a media file with **no backend
at all** — which, in GTK, does not error. It reports a duration of zero and plays nothing.

This is precisely the macOS situation, and macOS already has the fix: the `#[cfg(target_os =
"macos")]` arm of that same function returns `GstMediaStream`, a `GtkMediaStream` of Commune's own
built on `gst_play::Play`. Android does not get it only because Android has no GStreamer either.

**The useful consequence: there is already a one-function seam, designed for exactly this problem,
that any new backend can be dropped into without touching the audio-player UI at all.**

## What the code needs, precisely

_Measured_ — the whole GStreamer surface used by the non-call code is small:

| file | lines | uses |
| --- | --- | --- |
| `utils/media/audio.rs` | 197 | `uridecodebin3 ! audioconvert ! level ! fakesink` for the waveform; Discoverer for duration |
| `utils/media/video.rs` | 287 | `gst_pbutils::Discoverer` for metadata; a pipeline to grab one frame for the thumbnail |
| `components/media/gst_media_stream.rs` | 274 | `gst_play::Play` rendering into `gtk4paintablesink` |
| `components/media/video_player.rs` | 225 | wraps the above in `GtkVideo` |
| `components/media/video_player_renderer.rs` | 68 | makes the `gtk4paintablesink` |

Against `session/calls/pipeline.rs` at 1839 lines plus `ringtone.rs` — **calls are two-thirds of the
GStreamer code and stay out of scope.**

Note `gtk4paintablesink` is not part of GStreamer proper: it lives in gst-plugins-rs and is obtained
here through `ElementFactory::make`, i.e. from a system plugin registry that Android does not have.

## The routes

### A — GStreamer from the official Android binaries (Cerbero)

Drop `gstreamer-1.0-android-universal-<ver>.tar.xz` into the Gradle project, call `GStreamer.init()`
from the Java glue, point the Rust build at the headers and libraries.

* **For:** one code path with desktop; keeps the existing 1051 lines nearly unchanged; the only
  route that leaves calls possible later.
* **Against:** a foreign binary blob in a build that is otherwise all source; pkg-config and link
  plumbing that meson and pixiewood know nothing about; `gtk4paintablesink` still has to come from
  somewhere; APK growth on top of an APK that is already 366 MB and takes ~20 minutes to install
  over nullgate.
* _Unverified:_ whether the Cerbero layout can be made to satisfy the `PKG_CONFIG_LIBDIR` view that
  `pkgconfig-stubs.sh` builds for the Rust side.

### B — GStreamer as meson subprojects

The GStreamer monorepo builds with meson, and Commune already adds its own wraps outside
pixiewood's fixed list of eight — `gtksourceview.wrap` and `adwaita-icon-theme.wrap` are both ours.
So this is structurally possible.

* **For:** native to the build; no blobs; the pattern is already established twice.
* **Against:** gstreamer + plugins-base + plugins-good is an order of magnitude more than anything
  wrapped so far, `orc` and the codec libraries each bring their own cross-compilation questions,
  and the failure modes are unknown until tried. **Highest risk of the three by a wide margin.**

### C — Android's own media stack, no GStreamer at all

`MediaPlayer` (or ExoPlayer) behind a `GtkMediaStream`, via JNI, in the seam described above.
`MediaMetadataRetriever` for duration and thumbnail frames.

* **For:** nothing to cross-compile; negligible APK growth; hardware-accelerated decode; handles
  whatever the phone handles, which _unverified but likely_ includes Opus-in-Ogg, i.e. Matrix voice
  messages. Slots into `media_stream_for_file` with **no UI change**.
* **Against:** Android diverges from desktop; the waveform needs another source entirely; and video
  **rendering** is the hard part — getting decoded frames into a `GdkPaintable` means either
  SurfaceTexture → GL texture → `GdkGLTexture` interop with GTK's context, or pulling frames to CPU
  memory and pushing `GdkMemoryTexture`, which is simple but expensive.
* **Closes the door on calls** without a later GStreamer effort anyway.

## Recommendation: staged, C-first

Ordered by value-per-unit-risk rather than by tidiness.

1. **Audio playback via `MediaPlayer` + JNI.** Fills the existing seam, no UI work, and fixes the
   silently-inert player. Voice messages are the common case in a Matrix client and this is the
   cheapest thing on the list.
2. **Video duration and thumbnails via `MediaMetadataRetriever`.** Android's own API returns both,
   including frame extraction, which is exactly what `video.rs` wants from Discoverer plus a
   thumbnailer pipeline. Restores timeline thumbnails and outgoing metadata.
3. **Waveform.** Decide separately once 1 is in. Options: decode with `MediaCodec` and compute RMS
   in Rust; a pure-Rust decoder; or ship without a waveform at first, which is a cosmetic loss next
   to a voice message that will not play.
4. **Video playback.** The genuinely hard one, and best decided _after_ 1–3, because by then we will
   know whether the JNI-to-paintable interop is tractable in practice.
5. **Calls.** Out of scope. Different project, needs WebRTC, libnice, DTLS/SRTP.

The honest cost of this recommendation is **divergence**: Android would play media through a path
desktop does not use, and that path has to be maintained. The `#[cfg]` seam already exists and macOS
already uses it, so the shape is not new — but two backends is two backends.

## The decision that changes everything — settled

**Are voice and video calls on Android a real goal?** Asked 25 August 2026, answered: **calls
matter eventually.**

That settles it for route A. GStreamer becomes mandatory at some point, and doing C now and A later
does not replace the first effort, it adds to it — two backends, two sets of bugs, and the call
pipeline still unbuilt at the end of it. The staged plan above keeps its _order of value_ (audio
first, video playback last) but is delivered through GStreamer rather than around it.

### One thing to try before anything else

pixiewood builds GTK with `media-gstreamer = 'disabled'` **because GStreamer is not there**. If it
becomes there, that option can be flipped — and then `GtkMediaFile` has a backend, and
`media_stream_for_file`'s ordinary non-macOS arm works on Android with **no Commune change at all**.

Commune's own `GstMediaStream` exists only because the conda-forge GTK on macOS was built without
GStreamer. Android need not inherit that workaround. Worth establishing early, because it decides
whether `gtk4paintablesink` is needed at all — _measured:_ the current path reaches it even for
audio, since `GstMediaStream` always builds a `VideoPlayerRenderer` and that calls
`ElementFactory::make("gtk4paintablesink").expect(...)`.

### Staging for route A

0. **Spike: make `gst::init()` succeed on the device.** Nothing else matters until GStreamer links
   and initialises inside the APK. This is the whole risk of the route, concentrated in one step.
1. **Flip `media-gstreamer` in the GTK build** and see whether the audio player simply works.
2. **Waveform and duration** — un-gate `utils/media/audio.rs`.
3. **Video metadata and thumbnails** — un-gate `utils/media/video.rs`.
4. **Video playback** — un-gate the player; needs `gtk4paintablesink` if step 1 did not make it
   moot, most likely via the `gst-plugin-gtk4` crate registered statically rather than as a plugin
   `.so`.
5. **Calls** — a separate project, on top of a GStreamer that by then exists.

Steps 1–4 are almost entirely _un-gating code that already exists and already works on desktop_.
The work is step 0.

## Is this more work than a Kotlin UI?

Asked directly, and worth answering with numbers rather than instinct.

_Measured, 25 August 2026:_

| | lines |
| --- | --- |
| Rust in `src/` | 113,520 |
| Blueprint UI | 13,460 |
| of which `session_view/` (timeline, message rows, sidebar) | 38,302 |
| `components/` (widgets) | 15,915 |
| `session/` (GObject models, not portable to Kotlin) | 27,360 |
| `login/` | 2,154 |

A Kotlin front end replaces `session_view` + `components` + `login` + the Blueprints — **about
70,000 lines** — and most of `session/` besides, because those are GObject subclasses and list
models, not portable logic. What it would reuse from "our Rust repos" is smaller than it sounds:
`matrix-sdk` is upstream, and already ships official UniFFI Kotlin bindings that Element X Android
uses. So a Kotlin Commune would mostly be _a new Matrix client_, not a re-skin of this one.

Against that, the media work is: get one prebuilt library to link, then remove `#[cfg]` gates from
1,051 lines that already work.

**These are not close, and the media question should not be what reopens the Kotlin decision.**

What _would_ legitimately reopen it is a pattern, not a task: if step 0 proves not merely hard but
impossible, or if the list of things GTK-on-Android cannot do keeps growing faster than it shrinks.
So far it has shrunk — S6, S7, S8 and S9 each ended with the thing working, and three of the four
were unwired code rather than missing capability. The honest counterweight is that libshumate and
GStreamer are both genuinely absent, and that is a real cost, not a rhetorical one.

The way to keep this decision evidence-based is to **timebox step 0**. If GStreamer links inside a
day or two, the rest is deleting `#[cfg]` lines. If it does not, that is a real signal about the
platform and worth acting on — and it will have cost a day rather than a rewrite.

## Open questions to settle before starting

1. Does Android's `MediaPlayer` play Matrix voice messages as sent — Opus in an Ogg container?
   Cheap to test with a file already on the phone.
2. Can a `GdkPaintable` be fed from a `SurfaceTexture` without GTK's GL context fighting it? This
   is the crux of step 4, and worth a spike before committing to it.
3. Would GStreamer via Cerbero actually link here, given `pkgconfig-stubs.sh` and pixiewood's
   `PKG_CONFIG_LIBDIR`? Worth a timeboxed attempt before ruling route A in or out on cost.
4. How much APK does route A add? Relevant because installs already run ~20 minutes over nullgate
   and only ~2 minutes over LAN.

## Step 0 reconnaissance, 25 August 2026

All _measured_ against `gstreamer-1.0-android-universal-1.28.6`, extracted to
`~/android/gstreamer/{x86_64,arm64}` on the Arch host (2.0 GB per architecture).

### The tarball is static-only

**391 static archives and zero `.so` files.** There is no `libgstreamer-1.0.so` to ship; there is
`libgstreamer-1.0.a`. This kills the tidy idea of dropping plugin `.so`s flat into `lib/<abi>/` and
pointing `GST_PLUGIN_PATH` at `nativeLibraryDir` — there is nothing to drop in. GStreamer has to be
**linked statically into `libcommune.so`**, and its plugins are static too, so each one has to be
registered explicitly rather than discovered.

The mechanism is shipped with the tarball. `gst_init()` calls `gst_init_static_plugins()`, which the
application provides; `share/gst-android/ndk-build/gstreamer_android-1.0.c.in` is the template the
official ndk-build flow fills in, and it is short enough to write by hand:

```c
void gst_init_static_plugins (void) {
  GST_PLUGIN_STATIC_REGISTER (coreelements);   /* one per plugin */
  ...
}
```

### The two-GLib problem, and why it looks survivable

The tarball bundles **GLib 2.82.4**; pixiewood builds **2.89.4** for GTK. Linking both would put two
GObject type systems in one process, which is fatal — so GStreamer must resolve GLib against
pixiewood's shared `libglib-2.0.so` rather than its own `libglib-2.0.a`.

The tarball cooperates. `gstreamer-1.0.pc` says:

```text
Requires: glib-2.0 >=  2.64.0, gobject-2.0
Libs: -L${libdir} -lgstreamer-1.0
```

It requires `glib-2.0` **by module name**, not by its own copy. So a pkg-config search path with
pixiewood's `meson-uninstalled` **first** and GStreamer's `lib/pkgconfig` second resolves
`gstreamer-1.0` to the tarball and `glib-2.0` to pixiewood — exactly the split needed. 2.89.4
satisfies `>= 2.64.0`, and newer-GLib-under-older-GStreamer is the safe direction, since GLib does
not remove symbols.

_Unverified:_ whether the link actually succeeds. That is the whole of step 0.

### The four places this touches

| what | where |
| --- | --- |
| pkg-config search path for Cargo | `meson.build:193`, `cargo_env` — `PKG_CONFIG_LIBDIR` **replaces** the path, so GStreamer's dir must be appended with `:`, after `meson-uninstalled` |
| the dependency Meson links | `meson.build:124`, `android_deps` |
| `gst_init_static_plugins()` | a new C file beside `../build-aux/android/stub.c` in `src/meson.build:155` — that `executable()` is the one real link |
| the Rust crates | `Cargo.toml:132`, currently `[target.'cfg(not(target_os = "android"))'.dependencies]` |

The shape of the build helps here: Cargo produces `libcommune.a` and never links, so it only needs
pkg-config to _describe_ GStreamer; Meson performs the single real link. That is the same property
that made the port work at all.

_Measured:_ `gstreamer-sys` 0.25.2 requires `gstreamer-1.0 >= 1.14` and no higher feature is enabled
in `Cargo.toml`, so 1.28.6 is comfortably compatible — no version work needed.

### What step 0 actually is

The smallest thing that proves the route: link GStreamer core statically, call `gst::init()`, log
`gst::version_string()`, see it on the emulator. Not the media code — none of the gates come off
until this returns a version string.

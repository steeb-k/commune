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

## The decision that changes everything

**Are voice and video calls on Android a real goal, or not?**

* **If yes** — GStreamer is mandatory eventually, and doing route A now is probably better than
  doing C now and A later, because the second one does not replace the first, it adds to it.
* **If no** — route C is dramatically cheaper, and the case for cross-compiling GStreamer is much
  weaker than it looks, since two-thirds of the GStreamer code in this tree is the part we would not
  be building.

This is a product question, not a technical one, which is why it is not answered here.

## Open questions to settle before starting

1. Does Android's `MediaPlayer` play Matrix voice messages as sent — Opus in an Ogg container?
   Cheap to test with a file already on the phone.
2. Can a `GdkPaintable` be fed from a `SurfaceTexture` without GTK's GL context fighting it? This
   is the crux of step 4, and worth a spike before committing to it.
3. Would GStreamer via Cerbero actually link here, given `pkgconfig-stubs.sh` and pixiewood's
   `PKG_CONFIG_LIBDIR`? Worth a timeboxed attempt before ruling route A in or out on cost.
4. How much APK does route A add? Relevant because installs already run ~20 minutes over nullgate
   and only ~2 minutes over LAN.

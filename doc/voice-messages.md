# Voice messages — downstream implementation notes

Recording and sending a voice message, per MSC3245. Playback needed nothing:
a voice message is an audio message with two marker fields, and upstream's
`MessageAudio` row already plays audio messages. What this fork adds is the
sending half — a microphone button in the message toolbar, a recording page
in its stack, and the send. Built in round 6 of `doc/gap-closing-plan.md`,
26 August 2026.

## The wire format is the SDK's problem

`AttachmentInfo::Voice` exists in the pinned SDK (`db02d6c`) and writes the
whole MSC3245 shape when handed a `BaseAudioInfo`: the `m.voice` /
`org.matrix.msc3245.voice` marker, the `org.matrix.msc1767.audio` block, the
duration, and the waveform (the SDK is built with ruma's
`unstable-msc3245-v1-compat`, so both the stable-intended and unstable names
go out). So the app's job ends at handing over bytes, a MIME type, and a
filled `BaseAudioInfo` through the same `send_attachment` path every other
attachment takes.

The waveform and the duration come from `load_audio_info`
(`src/utils/media/audio.rs`), which upstream already wrote for the attachment
dialog: it plays the file through a muted GStreamer pipeline with a `level`
element and folds the loudness into the 30–120 normalized samples the MSC
asks for. The recorder therefore needs no `level` element of its own — the
finished file is analyzed after the fact, which also means the analysis and
the recording cannot disagree.

## The recorder

`src/session_view/room_history/message_toolbar/voice_recorder.rs` holds
`VoiceRecorder`, a plain GObject around the pipeline

```text
autoaudiosrc ! audioconvert ! audioresample ! opusenc ! oggmux ! filesink
```

writing to a random-named `commune-voice-message-*.ogg` under the system
temporary directory. Opus-in-Ogg is the one encoding every Matrix client's
voice implementation reads, and both elements ship in `gst-plugins-base`
(mux) and the Opus plugin already required for calls.

* `start()` builds the pipeline and starts a one-second `glib` tick that
  formats the `elapsed` property ("0:07") the recording page binds to. A
  pipeline that will not go to `Playing` — no capture device — surfaces as
  `VoiceRecorderError::NoMicrophone`, which the toolbar turns into a toast.
* `stop()` sends EOS and waits for the muxer to flush it — an Ogg file cut
  off without EOS is missing its last page — through a oneshot resolved by
  the bus watch, bounded at three seconds so a wedged pipeline cannot hang
  the send. It returns the path; the caller owns the file from then on.
* `cancel()` tears the pipeline down and deletes the file.
* The "failed" signal fires if the bus reports an error mid-recording; the
  toolbar resets to the composer.

## The toolbar

The microphone button sits next to the sticker button and swaps the
toolbar's stack to a `VoiceRecording` page: a red record icon, the elapsed
label, a Cancel button, and a send button. The page is one of
`MessageToolbarPage`, so everything that already reasons about the visible
page keeps working; losing the permission to send (or to send non-text
events) while recording cancels it, and so does leaving the room or the
thread — a recording does not follow across timelines. `send_voice_message`
stops the recorder, runs `load_audio_info` on the file, reads the bytes,
deletes the temporary file, fills in the size, and sends with the filename
"Voice message.ogg" (translated) and MIME `audio/ogg`.

## Testing

The pipeline is validated on Windows (UCRT64 GStreamer) and Linux (WSL Arch)
— but with `audiotestsrc`, because a remote desktop session exposes no
capture device (`doc/windows.md` records the same limit for calls).
`autoaudiosrc` fails cleanly there, which is exactly the NoMicrophone path.
Real-microphone recording needs a console session or RDP audio-capture
redirection, and is on the eyeball list.

On macOS the first real-microphone test recorded silence in the signed
build while the timer ticked and the file grew: the hardened runtime denies
audio input unless the signature carries the audio-input entitlement, and
the denial sits beneath the TCC permission the user grants, so nothing in
the pipeline errors — CoreAudio just delivers zeroes. Fixed by
`build-aux/macos/entitlements.plist`; the story lives in `doc/macos.md`
under "Signing".

## Rebase guide

* `voice_recorder.rs` is new and upstream-agnostic; it only touches GStreamer
  and glib.
* The toolbar changes live in `message_toolbar/mod.rs` and `mod.blp`: the
  `VoiceRecording` page variant, the button, the recording page, and the
  three callbacks. If upstream reshapes the toolbar stack, the page and the
  cancel-on-change rules in `update_visible_page`/`set_timeline` are what to
  carry over.
* If a rebase moves `load_audio_info` or `send_attachment`, follow them; the
  recorder itself does not care.

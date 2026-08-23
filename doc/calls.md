# Voice and video calls — downstream implementation notes

This file is the ledger for one-to-one calls: what the fork added, the
decisions behind it, and what to check when rebasing onto a new Fractal
release. See `fork.md` for why none of this goes upstream.

**Status: an invite goes out and the call window works; nothing has answered
one yet.** Seen on screen on 23 August 2026: the buttons appear on the right
rooms and not on the wrong ones, pressing one builds the pipeline, opens the
camera, renders a live self-view, sends `m.call.invite` into the room and sits
in `Dialing` with a working mute/camera/hang-up row. "Live" is doing work in
that sentence — until the two pipeline bugs below were found it was one frozen
frame, which looks close enough to working to be reported as working. What has **not** been seen is
any of the second half — an answer, ICE connecting, media flowing, or a call
ending in anything but a hangup or a timeout. Read the last section before
trusting any of that.

Testing it needs two clients, and two accounts inside one Commune are not two
clients: `Calls` holds one call per session, so a second call from the same
account is refused by design, and two pipelines in one process would be
fighting over one camera and one microphone anyway. Use two machines, or
Commune against Element.

## Scope

The Voice over IP module of the Client-Server API as written: `m.call.invite`,
`m.call.candidates`, `m.call.answer`, `m.call.select_answer`, `m.call.reject`,
`m.call.hangup` and `m.call.sdp_stream_metadata_changed`, version `1`, carrying
WebRTC between exactly two devices. Audio and video, placed and answered, in a
window of its own.

Not in scope, and not the same thing: MatrixRTC and Element Call. Those are
`m.rtc.*`, an SFU, and a set of proposals rather than a module of v1.19. The
one-line "started a call" row that upstream draws for an `m.rtc.notification`
is untouched.

## Why the legacy module and not MatrixRTC

MatrixRTC is where the ecosystem went, and it is where group calls live. It is
also not in the spec this fork measures itself against, it needs an SFU to be
useful, and every client that has it embeds Element Call in a web view — which
would mean a `WebKitGTK` dependency and a web application inside a GTK client.

The module below is what v1.19 defines, it is peer to peer, and `webrtcbin`
already ships in `gst-plugins-bad`. It closes the row on the spec page. It also
does not interoperate with Element's current calling, which is the honest cost
and is worth saying out loud.

## The shape of it

| Piece | What it does |
| --- | --- |
| `session/calls/mod.rs` | `Calls`: one per session, holds the call that is happening, registers the seven event handlers, routes by `call_id` |
| `session/calls/call.rs` | `Call`: the state machine, and everything that goes on or comes off the wire |
| `session/calls/pipeline.rs` | `CallPipeline`: `webrtcbin`, the encoders, the sinks |
| `session/calls/turn.rs` | The credentials, and the URI conversion that is not string concatenation |
| `session/calls/state.rs` | `CallState` and `CallEndReason` |
| `session_view/call_view/` | The window |

There is at most one call at a time. Two calls means two microphones, two sets
of speakers, and no way to say which one a hangup was about. A second invite
gets `m.call.hangup` with `user_busy`, which is the nearest thing the spec has
to a busy signal — there is no `m.call.busy`.

## The TURN URI is a different grammar

The most likely thing to get wrong quietly. The homeserver answers
`GET /_matrix/client/v3/voip/turnServer` with RFC 7065 URIs —
`turn:host:port?transport=udp` — and the credentials **alongside** them.
`webrtcbin` wants them **inside**, as `turn://user:password@host:port`.

Synapse's username is a timestamp joined to a full user ID:
`1787551090:@alice:localhost`. Both `:` and `@` are in there, so pasting it
into the authority produces a URI that parses as something else entirely.
`webrtcbin_turn_uri()` percent-encodes the userinfo, and there is a test with
that exact string in it.

`turns:` maps to `turns://`. `stun:` is refused rather than guessed at, since
it takes no credentials.

**No default STUN server.** The usual choice is Google's, and pointing every
call at a third party to learn our own address tells that third party that a
call is happening at all. A TURN server answers STUN binding requests too, so
the homeserver's own covers it — and on a homeserver with no TURN, host
candidates are what is left, which works on a LAN and not through a NAT.

## The offer decides the media sections

`webrtcbin` numbers its sink pads in the order they are requested, and that
number is the m-line index. An answer whose sections do not line up with the
offer's, section for section, is one `webrtcbin` refuses.

So the caller chooses — audio, then video, which is what every other client
offers — and the answerer reads the offer with `gst_sdp::SDPMessage` and builds
to match. A section it can name but cannot source, because there is no camera,
becomes a receive-only transceiver rather than a missing m-line.

An offer that yields no sections at all is rejected rather than guessed at.
That includes one that does not parse: `SDPMessage::parse_buffer` reports a
garbage SDP by handing back an empty message rather than an error, which is why
the emptiness is what gets checked and not the `Result`.

## Batching, and the empty candidate

The spec asks for candidates to be batched, and it is worth following: a
candidate per event is a dozen events in the room for one call, all of which
the other end has to sync before it can use any of them. Two seconds after the
invite — there is a natural pause anyway while the other end decides — and half
a second after the answer, where every one of them is between two people and
hearing each other.

> An ICE candidate whose value is the empty string means that no more ICE
> candidates will be sent. Clients **must** send such a candidate.

`ice-gathering-state` reaching `Complete` is what triggers it, and it flushes
whatever is still queued at the same time.

**The two ends spell end-of-candidates differently.** The spec says the empty
string; `webrtcbin` says a NULL candidate, and hands an empty string to its
parser like any other, where it is not a candidate and fails. So the two are
translated in `add_ice_candidate()` rather than passed through.

This landed once as a commit that did not contain it — an edit script asserted
its way out before writing, and the message described code that was not there.
Verify a change of this kind by grepping the built binary for the strings it
adds, not by trusting that the edit applied.

Candidates can arrive before the call is answered, when there is no pipeline to
give them to. They are kept and replayed after `set-remote-description`.
Dropping them costs a round trip at best and the call at worst.

## Both queues into and off the camera leak

The send queue leaks because `webrtcbin` consumes nothing until the call
connects; that is recorded above. The **preview** queue leaks for a second
reason, and it is the one that is easy to miss.

Both queues sit _before_ their `videoconvert`, so what they hold are the
camera's own buffers. A camera hands out a small fixed pool — four or eight —
and a queue that holds even a few and does not give them back starves the
source. The whole tee stops, preview included.

`videotestsrc` allocates a fresh buffer every time and has no pool, so a test
pipeline built on it cannot reproduce this. Measured against one, the topology
with a plain preview queue held a steady 30 fps for twelve seconds with
`webrtcbin` consuming nothing — which cleared the tee, the send queue and the
sink, and could say nothing at all about the camera.

Also measured, and also not the cause: `Gtk.Image` and `Gtk.Picture` repaint
identically for this sink — 241 snapshots each against 240 paintable
invalidations over eight seconds. The self-view being a `Gtk.Image` is a
sizing decision and nothing more.

## Both connection states are watched, not only the aggregate

`webrtcbin` has two that matter, and only one of them is any use for telling a
person that their call has started.

`connection-state` is the aggregate `RTCPeerConnectionState`, and it moves only
once every transport underneath it has reported in — ICE **and** DTLS.
`ice-connection-state` moves as soon as a candidate pair starts carrying
traffic.

Watching only the aggregate is how a call with a perfectly good path sits on
"Connecting…" for ever. That is what it did, on the same wifi and across
networks alike — and the sameness across both is the tell, because a NAT
problem would not behave identically on a LAN.

Either one reaching a usable value is now taken as connected, whichever gets
there first, and the second is a no-op. `Completed` counts alongside
`Connected`, being the same thing plus "and nothing better is coming".
`Disconnected` is deliberately not a failure: it is usually a handful of lost
packets on a wifi handover, and ICE recovers from it on its own.

Every one of these states is logged, along with how many candidates go out and
come in, because none of it was visible and every guess about ICE costs a round
trip through somebody else's machine.

## Two devices, one answer

Version 1 exists mostly for this. Every event carries a `party_id`, parties are
`(user_id, party_id)`, and a user can call themselves — so "ignore events from
our own user" is wrong and the pair is what gets compared.

When two of the callee's devices answer, the caller takes the first and sends
`m.call.select_answer` naming it. The devices that did not win see their own
party ID is not the selected one and stop. That is `CallEndReason::AnsweredElsewhere`,
which is not a hangup reason at all — nothing goes on the wire for it.

## Glare

Two people calling each other at the same moment. The spec settles it with a
rule both ends can run without talking: of the two call IDs, the lesser wins.
Both ends reach the same answer, so the two people end up in one call rather
than two half-calls, and it should look to them as though the person they
called simply picked up.

Only for calls to the same room, which is what makes it glare rather than a
second call. A second call in a different room is refused as busy.

## What is deliberately refused

* **Rooms we cannot send a message to.** A call is message-like events in the
  room; a room that refuses them refuses the call. This is what keeps the
  buttons off the server notices room.
* **Rooms with more than two people.** "Calls should only be placed to rooms
  with one other user in them. If they are placed to group chat rooms it is
  possible that another user will intercept and answer the call." The invite
  goes to the room, not to a person. So the call buttons appear on a two-person
  room and nowhere else. The count is the test, not whether the room is in
  `m.direct`: two people in a room nobody marked as a direct chat can still
  call each other, and a direct chat that grew a third member cannot. That is
  why `other_member()` exists rather than `Room::direct_member()`, which only
  answers for the former.
* **Invites in public rooms.** "As a starting point, it is RECOMMENDED that
  clients ignore call invites in rooms with a join rule of `public`." Anybody
  can walk into one, and a ringing phone is not a thing a stranger should be
  able to cause.
* **Invites addressed to somebody else.** The `invitee` field, when it is set.
* **Expired invites**, by the event's `age` rather than its timestamp — the
  spec's own reason, and a good one: a client with a wrong clock would
  otherwise discard every invite or none of them. An invite with no `age` is
  rung for, because not ringing loses a call in silence where ringing for a
  dead one ends with the other side hanging up.

## Muting

The microphone is a `volume` element with `mute`, so the track keeps flowing
and only silence goes down it. The camera is a `valve` with `drop`.

Both are announced with `m.call.sdp_stream_metadata_changed`, keyed on the
stream ID taken out of our own SDP. **There are two places an SDP can carry
one, and the one the spec's examples show is not the one we write.** A browser
puts it at media level, `a=msid:<stream> <track>`; `webrtcbin` writes no such
line anywhere, only the per-source form:

```text
a=ssrc:2181993077 msid:user217149580@host-e5ab91bf webrtctransceiver0
```

Reading only the first form left the stream ID empty, and an empty one means
the metadata map is empty, and an empty map is never sent — so the microphone
and the camera stopped and the far side was never told why. `first_stream_id()`
reads both forms. `a=ssrc:N cname:…` has the same line shape and is not an
msid; there is a test for that too, and one asserting that both media sections
of a real offer name the same single stream, which is what one `m.usermedia`
is supposed to look like.

Setting the ID instead of reading it would be better, and is not available:
`GstWebRTCRTPTransceiver` has no `msid` property in GStreamer 1.28.6.

A mute made while the call is still ringing is announced once it connects. The
invite or the answer carries the state as it stood when the description was
made, and `send_stream_metadata()` will not send to a party that has not
answered — so without that the far side would join already wrong.

The spec's asymmetry is followed on the receiving side. A remote `video_muted`
hides the picture and shows the avatar, because the alternative is a frozen
frame or a black rectangle. A remote `audio_muted` does **not** mute the
incoming audio: unmuting takes a round trip and the words spoken in between
would be lost.

## Four things seen on screen, and none of them visible to the compiler

**The self-view is a `Gtk.Image` with `pixel-size`, not a `Gtk.Picture`.** A
Picture takes the natural size of its paintable, and a camera's paintable is as
big as the camera, so the corner thumbnail filled the whole window and covered
the person being called. `width-request` is a minimum, not a maximum, and does
nothing about it. This is the same trap `url-previews.md` records for preview
images; it cost a round trip here too.

**The call buttons need `can_send_message`, not just a member count.** A call is
a stream of message-like events into the room. The server notices room has two
members and puts the recipient at power level −10, so the buttons appeared,
the invite was refused with `M_FORBIDDEN`, and the call rang for nobody while
four events failed to send in a row.

**The queues in front of `webrtcbin` have to leak, or the self-view freezes.**
`webrtcbin` consumes nothing at all until the call is connected — no answer, no
transport, nowhere for a packet to go. A plain `queue` in front of it fills
while the other end is still deciding whether to pick up and then blocks what is
behind it, which for audio is the microphone and for video is a `tee` whose
other branch is the self-view. A `tee` runs no faster than its slowest branch,
so the preview stopped too: the caller's own face frozen on its first frame for
as long as the call rang, while the camera light stayed on. Measured on the
outgoing chain with nothing answering, the preview got about two seconds of
frames and then none, and audio reached `webrtcbin` once and then never again;
with `leaky=downstream` on both it holds a steady 30 fps for as long as it is
left running. Dropping the oldest is the right end to drop from — a frame that
could not be sent while nobody was listening is of no use once somebody is —
and nothing is dropped after the call connects, because from then on
`webrtcbin` is taking them. This was on Linux as well; it is not a macOS bug.

**The self-view sink has to be asked for a format by name, or the camera dies.**
`gtk4paintablesink` offers `video/x-raw(memory:GLMemory)` ahead of everything
else, and `videoconvert` passes a memory feature it does not recognise straight
through rather than refusing it — so left alone the preview branch negotiates GL
textures and the camera is asked to produce them. `avfvideosrc` accepts that and
then fails on the first buffer, and what arrives on the bus is
`Internal data stream error … streaming stopped, reason error (-5)`, attributed
to the source. It is not a negotiation error, so there is nothing in it pointing
at the sink that asked for GL, and the element it names is the one element that
is not at fault. A `capsfilter` of `video/x-raw, format=RGBA` before the sink
settles it, because naming a format makes `videoconvert` convert instead of pass
through. A bare `video/x-raw` does **not** work — that was tried, and the GL
feature still won. The remote video sink needs none of this: it is fed by a
decoder that only ever produces system memory, so there is no GL path to prefer.

Both of those took reproducing outside the application to find. The bus error
names a source that is working correctly, and a frozen preview looks like a
camera problem rather than a queue three elements away — `doc/macos.md` has the
`GStreamer` debug settings, and `error.debug()` is now logged beside
`error.error()` because the reason lives in it and nowhere else.

## The window

A window rather than a page in the session view. A call outlives whichever room
the person is looking at, and a window is the thing their compositor already
knows how to keep on top, move to another workspace, or put away.

Closing it hangs up. Leaving somebody on a call that only one end thinks has
ended is worse than an unexpected hangup.

Video is a `gtk4paintablesink` and its `paintable`, the same way
`video_player_renderer.rs` does it for recorded video. The self-view is a
second `gtk4paintablesink` on a `tee` off the camera, before the encoder, so
the preview is what the camera sees rather than what the far end will get.

## Files

Integration points, which are where a rebase will conflict:

| File | Change |
| --- | --- |
| `src/session/calls/` | All of it, new |
| `src/session_view/call_view/` | All of it, new |
| `src/session/mod.rs` | The `calls` property, and `init()` in `prepare()` |
| `src/session/room/mod.rs` | `handle_member_event()` tells `Calls` when somebody leaves |
| `src/session_view/mod.rs` | Opening the window when a call appears |
| `src/session_view/room_history/mod.rs` | The two header buttons and when they are shown |
| `src/session_view/room_history/mod.blp` | The buttons themselves |
| `data/resources/stylesheet/_session_view.scss` | `call-button`, `call-self-view` |
| `Cargo.toml`, `meson.build` | `gstreamer-webrtc` and `gstreamer-sdp` |
| `testing/local-homeserver.sh` | The coturn container and the TURN check |

## Runtime requirements

`webrtcbin` is in `gst-plugins-bad`, and it needs `libnice` for ICE. The codecs
are Opus (`gst-plugins-base`) and VP8 (`gst-plugins-good`, via `libvpx`), and
`dtlssrtpenc` comes from `gst-plugins-bad` as well. None of these are new
Meson dependencies — they are plugins, found at runtime — so a build that
succeeds on a machine without them produces a client whose calls fail with a
missing-element error rather than one that fails to link.

**That is not hypothetical, and macOS is where it happened.** conda-forge's
`gst-plugins-bad` ships the `libgstwebrtc-1.0` library and the
`gstreamer-webrtc-1.0.pc` this fork links against, but not the `webrtc`, `nice`
or `srtp` plugins — and the channel has no `libnice` or `libsrtp` package at
all. The build was clean, every test passed, and `webrtcbin` did not exist. So
`build-aux/macos/setup-conda-macos.sh` builds libsrtp2, libnice and those two
plugins from source into the environment, and `build-aux/macos/bundle.sh` lists
`webrtc nice srtp dtls rtp rtpmanager` among the plugins it copies — `rtp` and
`rtpmanager` were missing from that list too, so even the payloaders that _did_
exist in the environment were being left out of the app. `macos.md` has the
whole recipe. Check with:

```sh
gst-inspect-1.0 webrtcbin
```

## Testing

`./testing/local-homeserver.sh up` now runs a coturn beside the Synapse and
points the two at each other. `turn` prints what a client is handed; `check`
allocates a relay with those credentials and pushes packets across it, because
credentials that parse are not credentials that work and the failure mode of a
mismatched secret is a call that works on one machine and fails for anybody
behind a NAT.

Two clients on the same machine will never need the relay — host candidates
win. It is there so that the code that reads the credentials, hands them to
`webrtcbin` and gathers relay candidates runs at all.

## Not done, and not yet seen working

**Nothing past the invite has been exercised.** The outgoing half is seen
working; the answering half, ICE and media are not. The parts most likely to be
wrong first, in the order they will show up:

1. **The answerer's pad ordering.** Building sink pads to match the offer is
   the fiddliest thing in `webrtcbin`, and getting it wrong shows up as an
   answer the other end refuses rather than as an error here.
2. **`add-turn-server` accepting the URI.** It returns a boolean and the code
   logs a warning on `false`; watch for that before blaming ICE.
3. **`autoaudiosrc` and `autovideosrc` under a portal.** On a sandboxed desktop
   the camera wants `pipewiresrc` through the portal, and `autovideosrc` may
   pick a `v4l2src` that cannot open the device. On the machine this was first
   run on — an IPU6 camera on Arch — `autovideosrc` opened it and the frames
   were fine, so this is not a given failure.
4. **Renegotiation.** `m.call.negotiate` is parsed by ruma and is not handled
   here at all, so a call that renegotiates mid-flight — which is what adding
   video to a voice call looks like — will not follow.
5. **Muting is never announced, because `first_stream_id()` finds nothing.**
   This one is no longer a guess. Driving `webrtcbin` 1.28.6 through the same
   element chain `new_for_offer()` builds produces an offer whose only msid is
   an **ssrc attribute**:

   ```text
   a=ssrc:3324831192 msid:user3428219828@host-8d6a82da webrtctransceiver0
   ```

   There is no media-level `a=msid:` line anywhere in it. `first_stream_id()`
   strips exactly that prefix, so it returns `None`, so — by the design
   recorded under "Muting" above — `m.call.sdp_stream_metadata_changed` is
   never sent. The microphone and the camera still stop locally; the other end
   is simply never told, and never hides the picture. Reading the ssrc form as
   well is the fix, and it is not macOS-specific: it is what this version of
   `webrtcbin` writes. **Fixed on 23 August 2026**, against an offer dumped
   from `webrtcbin` 1.28.6 on Linux that matched the macOS one line for line;
   see "Muting" above.

   The rest of that offer was as intended — `m=audio` then `m=video` in the
   order the answerer is expected to match, both `sendrecv` and in
   `a=group:BUNDLE`, a DTLS fingerprint, and 13 ICE candidates gathered. The
   `m=video 0` port is `a=bundle-only`, which is what `max-bundle` is supposed
   to produce, not a rejected section.

Also absent on purpose:

* **DTMF.** The spec allows sending it; there is nothing in this client that
  would want to.
* **Screen sharing.** A second stream with `purpose: m.screenshare`. The
  metadata plumbing is there for it, the pipeline is not.
* **Call history in the timeline.** The events are filtered out of the
  timeline entirely, so a missed call leaves no trace once the window is
  closed. That is the next thing worth building.
* **Ringing.** No sound, and no notification. An incoming call opens a window,
  which is not enough if the client is on another workspace.

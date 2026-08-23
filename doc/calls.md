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

**No STUN server of our own choosing, and the homeserver's is used.** The
usual default is Google's, and pointing every call at a third party to learn
our own address tells that third party that a call is happening at all. So
this client names none.

A `stun:` URI in the homeserver's own list is a different thing: its operator
chose it, and until 23 August 2026 we threw it away. That cost more than it
looks like. A TURN server does answer STUN binding requests — but only from
where the client can reach it, and on a deployment whose TURN server sits
inside the same NAT as the client, the binding response reports the client's
_private_ address. `webrtcbin` then drops a reflexive candidate identical to a
host candidate it already has, and the client goes into the call with no
address the other end can reach: relay or nothing.

That was this deployment, and it is why one end being co-located with coturn
turned every cross-network call into relay-to-relay. `stun:` is honoured now,
converted to `webrtcbin`'s `stun://host:port` and set on its `stun-server`
property. `stuns:` is refused — `webrtcbin` has no spelling for STUN over TLS
— and a homeserver that names no STUN server behaves exactly as before.

## Opus is offered with two channels, whatever the microphone has

`a=rtpmap:111 OPUS/48000` is a codec libwebrtc does not have. RFC 7587 §7:
"The RTP clock rate ... MUST be 48000, and the number of channels MUST be 2",
and libwebrtc holds the other end to it — the channel count is part of the
codec's identity, so an offer that leaves it off matches nothing and the whole
media section is answered with `port 0`.

That is what every call to Element for Android did, and it is why they failed
in ways that looked like ICE. A voice call came back:

```text
a=group:BUNDLE
m=audio 0 UDP/TLS/RTP/SAVPF 0
```

Nothing accepted, an empty bundle group, no transport — so the far end never
sent a candidate, nothing ever failed, and the call sat in `Connecting` until
somebody hung it up. A video call came back with the audio rejected and the
video kept, which failed differently and worse; the next two sections are
both about that call.

The fix is in the caps, in two places. `encoding-params=2` goes on the
`application/x-rtp` capsfilter, because **the offer is made before the audio
chain has negotiated** — `webrtcbin` writes the SDP from whatever the sink pad
carries at that moment, which for a call placed the instant the pipeline plays
is the capsfilter's own caps and nothing the encoder would have added. And the
raw side is pinned to `channels=2` so the declaration is true rather than
merely correct.

Measured against the real chain: with the mic negotiated first, `rtpopuspay`
writes `encoding-params=(string)2` and `sprop-stereo=0` on its own and the SDP
says `OPUS/48000/2`; the offer that went on the wire said `OPUS/48000` and
carried no `a=fmtp` line at all, which is the signature of an SDP built from
static caps. Both descriptions are logged now — ours and theirs — because the
one thing this could not be diagnosed without was seeing them side by side.

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

## The answer is created from inside the offer's promise

`set-remote-description` is asynchronous. Emitting it and calling
`create-answer` on the next line asks `webrtcbin` to answer an offer it has not
applied yet, and what comes back is an **empty answer**. An empty answer is
never sent, so the caller sits on "Connecting…" until its invite expires, and
the callee sits there with a pipeline that will never negotiate. That is what
every incoming call did.

So `answer_remote_offer()` emits the description with a promise and creates the
answer from inside it, which is the only ordering `webrtcbin` guarantees. The
same care is not needed for the caller taking the answer: nothing follows it.

The symptom in the log is one line — `the answer came back empty` — and it took
this long to see because the failure is on the _callee_, while the complaint a
person makes is about the caller's window.

## A remote candidate needs a remote description first

`webrtcbin` discards ICE candidates added before the remote description is
applied — there is no remote ICE agent to give them to yet. And
`set-remote-description` is asynchronous, so the line after it runs long before
that is true.

On the answering side that was every candidate the call had. The buffered ones
were handed over on the line after the description was emitted, and the log
shows them going in ahead of `HaveRemoteOffer`:

```text
Setting the remote offer, and answering it when it is applied
Adding remote candidate for m-line 0   ×22
Signalling state is now HaveRemoteOffer      <- only now does a remote agent exist
```

So candidates are held until the description's promise fires and are added
then. Both descriptions carry a promise for it: the answerer's offer and the
caller's answer, since the caller receives candidates before the answer just as
often.

This is the same fault as the empty answer, one line further down, and it
survived that fix because the answer was the visible half.

## A candidate says which section it belongs to, by name as well as by index

`m.call.candidates` carries `sdpMid` and `sdpMLineIndex`, and the spec asks
for at least one. We sent only the index, and for a long time nothing said
otherwise — a candidate with an index is well-formed and this client reads
its own incoming candidates by index too.

The other end does not. A client on libwebrtc turns each of them into
`IceCandidate(sdpMid, sdpMLineIndex, line)`, and a null mid is a candidate
that goes no further. Nothing is logged, nothing is rejected on the wire, and
what it looks like from here is a network that will not carry the call:

* the far end has no remote candidates, so it never sends a check;
* it never installs a relay permission for us either, so the checks _we_ send
  arrive at its relay and are dropped there — which is exactly what the
  relay's own counters said, packets in and none out;
* and ICE on both sides runs its retransmissions out against silence.

So the mid goes out too, read from our own description: `section_mids()` takes
the `a=mid:` of each section in m-line order, and the candidate signal looks
its own index up in that. An empty end-of-candidates needs neither field and
keeps the index it always had.

## Candidates can arrive before the invite they belong to

Not before the answer — before the **invite**. The router looks the call up by
`call_id`, and a batch for a call this session is not in looked exactly like a
batch for a call that has not been invited yet, so both were dropped.

Measured on 23 August 2026, on an incoming call from Element for Android: all
twenty-four of its candidates arrived one sync ahead of the invite and were
thrown away, and the call that followed had not a single remote candidate to
pair with. It rang, it was answered, an answer went back, and nothing after
that could ever have connected.

The peer sends both within a few hundred milliseconds of each other and the
order they reach us is not the order they were sent: in an encrypted room the
invite is the event that has to wait for a megolm session and its key to go
out, and the candidates that follow ride a session that already exists.

So a batch whose call is unknown is kept — for thirty seconds, eight batches at
most — and replayed the moment an invite claims it. Everything else about it is
unchanged: `Call` holds them again until there is a pipeline, and the pipeline
holds them again until the remote description is applied.

Anything nothing claims belongs to a call this session is not in, which is the
other half of what this path always saw, and it is forgotten.

## Every remote candidate goes on the bundled section

Which is section 0 in every call where the other end accepts what was offered,
and was hard-coded as 0 until one did not. Element's answer to the video call
above rejected the audio section and kept the video one:

```text
a=group:BUNDLE video1
m=audio 0 …   a=ice-ufrag:zHw1   a=mid:audio0
m=video 9 …   a=ice-ufrag:dv9I   a=mid:video1
```

Section 0 has no transport, so every candidate put there named nothing, and
the `ice-ufrag` read from the top of the file — `zHw1`, belonging to the
rejected section — made all eleven of the other end's candidates look like
they came from a stranger's call. Both readings now come from the section
`a=group:BUNDLE` names first, which is the one that owns the transport.

The reasoning behind ignoring the index the other party sends holds as it did.
This client always negotiates `max-bundle`, so a call has exactly one transport
and only one section owns it. Our own second section is the `a=bundle-only`
video one, advertised with `port 0`: a candidate placed there names nothing and
`webrtcbin` drops it without a word.

A caller whose peer labels every candidate `1` therefore collects none at all,
and `ice-connection-state` never leaves `New` — it does not reach `Checking`,
let alone fail. Nothing moving at all is the signature of no remote candidates;
a relay that refused to allocate would still show `Checking`, because there
would be pairs to try.

**Clamping only out-of-range indices was tried first and was not enough.** On an
audio-only call there is one section, so `1` is out of range and gets caught. On
a video call `1` is in range and still wrong, because being in range is not the
same as having a transport. That distinction cost a round trip.

## libnice aborts the process when it nominates a pair that stopped succeeding

Seen on 23 August 2026, on the first call Element ever accepted both media
sections of:

```text
libnice:ERROR:../libnice/agent/conncheck.c:959:
  priv_conn_check_tick_stream_nominate: assertion failed:
  (p->state == NICE_CHECK_SUCCEEDED)
Bail out!
```

The whole client dies — this is `g_assert` in a dependency, so there is
nothing to catch and nothing to report to the person on the call.

It happens at the last step of ICE and only on the controlling side, which is
the one that placed the call. `priv_conn_check_tick_stream_nominate` walks the
valid pairs looking for the one to nominate, and asserts that a valid pair is
either succeeded or a discovered pair whose parent succeeded. A pair that
succeeded and then failed — a later check on it timing out, a TCP connection
dropping — is valid, has no parent, and is in neither state.

The assertion is on libnice's `master` as well as in 0.1.23, so there is no
version to move to. Which pair gets into that state, though, is decidable from
this side, and the second run said which one: the crash came in the same
millisecond as `Checking`, before any check could have succeeded and then
failed, so the "succeeded and then died" reading was wrong.

**Two candidates for one transport address is the way in.** libnice resolves
the response to a check by looking up the remote candidate _by address_
(`nice_component_find_remote_candidate`), so when two remote candidates share
one address:port, the response to the second pair's check is credited to the
first pair — `SET_PAIR_STATE (new_pair, NICE_CHECK_DISCOVERED)` on a pair that
is already `valid` and has no parent pair. That is precisely the pair the
nomination tick asserts cannot exist.

Element for Android sends exactly that: `192.168.50.234 59098 typ host` and
`192.168.50.234 59098 typ srflx`, its own address seen through a STUN server
that did not translate it. So one candidate per transport address is kept —
the second offers no path the first does not, and costs the process.

Remote loopback candidates are dropped for the lesser version of the same
reason: `127.0.0.1` is a path to us, not to them, and every pair that cannot
work is another chance at a bug in the checking.

Reaching this at all is what it looks like to get everything else right. The
nomination tick only runs when a pair has actually succeeded, so the call had
a working path and died deciding which one to use.

## Both connection states are watched, not only the aggregate

This was chased first, on the theory that a call with a working path was
sitting on "Connecting…". It was not the cause — nothing was connecting at all
— but it stays, because the aggregate state is still the wrong thing to drive
an interface from.

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

## Telling a missing relay from a broken one

A call between two people on one network needs no TURN at all — host
candidates find each other. A call between two networks needs a relay, and
there are three separate ways for that to be absent, which look identical from
the outside:

* the homeserver offers no TURN server, or one this client cannot use;
* `webrtcbin` refuses the URI we build from it;
* the TURN server is reached and refuses to allocate.

All three end the same way: ICE stops at `Checking` and the call never starts.
So each is logged separately — how many URIs the homeserver offered and what
they were, whether `webrtcbin` accepted each one, and a warning when a call is
placed with none at all. The URIs carry no credentials, those being separate
fields, so logging them is safe; and a `turn_uris` pointing at a LAN address
looks exactly like a working one until somebody calls in from outside.

**The percent-encoded URI works, end to end, and was measured doing so.**
Against the throwaway coturn, with credentials the throwaway Synapse minted,
`webrtcbin` gathered `host: 15, srflx: 2, relay: 1` — a relay candidate means
the allocation was made and the credentials authenticated, not merely that the
URI parsed.

The unencoded form is not an alternative: `add-turn-server` returns **false**
for it, because a Synapse username carries `:` and `@` and the authority does
not survive them. So the encoding is what makes the URI parse, and the parse is
not costing the authentication. Both halves of that were guesses before they
were measured.

## A relay cannot reach another allocation on the same server

Measured on 23 August 2026 against the real deployment, with credentials the
homeserver minted for a throwaway account:

```text
allocation A: 174.74.218.66:49182
allocation B: 174.74.218.66:49189
permission A -> B's relay 174.74.218.66 : refused: 403
permission A -> a public host 8.8.8.8   : granted
```

Two allocations are made, both succeed, and neither is allowed to name the
other as a peer: `CreatePermission` for the server's own public address comes
back `403 Forbidden`, while the same request for an unrelated address is
granted. So relay-to-relay does not work on this server, and that is a coturn
configuration — a `denied-peer-ip` range covering its own address — not
anything a client can route around.

It is the whole of the difference between the two calls of that run. On one
LAN the call connected on host candidates and carried audio and video. Over
mobile data it reached `Checking` and failed, because every pair it had left
was relay-to-relay:

* the phone is behind carrier NAT, so its host candidates are unreachable and
  it needs its relay;
* **and so do we**, because `turn.kzenjak.com` resolves to `192.168.50.35`
  inside this network. A STUN binding request answered from the LAN reports
  our own private address, `webrtcbin` drops a reflexive candidate identical
  to a host candidate it already has, and the relay is the only candidate we
  have that anything outside can reach.

The second of those is worth knowing on its own: split-horizon DNS for the
TURN host costs the client its server-reflexive candidate, and with it every
direct path a NAT would otherwise have allowed.

`allowed-peer-ip` for the server's own address is what makes the pair legal;
it is checked ahead of the denied ranges.

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

## Where this stands, and how to test it

Written down because the debugging ran long and the context it lived in is not
durable. This section is the state of it.

### What is known to work

* A call between Commune and Element for Android on one network, audio and
  video, `Checking` to `Connected` to `Completed`. Seen on 23 August 2026 at
  21:13, with both media sections accepted — the first call to that client
  that ever negotiated audio.
* A call between networks, Commune here and the phone on mobile data, with a
  TURN server outside both. Seen at 23:04 the same evening: `Checking` at
  27.400, `Connected` at 28.051, `Completed` a second later. Six hundred
  milliseconds, where every attempt before it spent eight seconds failing.
* Both ends obtaining a TURN relay from the homeserver's coturn: `add-turn-server`
  accepts the percent-encoded URI, and `host`/`srflx`/`relay` candidates are all
  gathered.
* Offer, answer, `select_answer`, hangup and the rest of the signalling.

### What is not

* Nothing known, as of the run above. What was here — a call between networks
  — now works.

### The diagnosis as it stands

**The far end's candidates name an ICE session its own answer does not.**
Measured on 23 August 2026 against Element for Android, twice in one run —
once over the internet and once with both machines on one switch:

```text
Answer for call kHUhimu2SaCX5nwT from party Some("LYEBQWFTPB")
The remote description carries ice-ufrag +sSs
Adding remote host candidate … 192.168.50.234 34239 typ host … ufrag Jlts
        ×11, every one of them Jlts
ICE connection state is now Checking      → Failed, five seconds later
```

Both values are four characters, which is what libwebrtc writes and not what
`webrtcbin` does, so both are the phone's: it answers out of one ICE session
and trickles out of another. libnice discards a candidate whose tag is not the
remote description's, so the pair list was empty — with the two machines on
one network and `192.168.50.234` sitting there in the list.

Not two devices. This was one, `party_id LYEBQWFTPB`, the only one signed in,
and the split appears on every call it answers. It does _not_ appear on calls
it places: the invite it sent us carried `ice-ufrag 9ENC` and every candidate
behind it said `ufrag 9ENC`.

So the tag is dropped and the address kept — see "Every remote candidate goes
on section 0" for the other half of the same argument. An address is what a
candidate is for; the answer is what says which session the call is.

**And a call is no longer hung up on the first `Failed`.** libnice reports it
the instant it has nothing left to check, which on a trickling call describes
that instant and not the call. The other end's next batch comes through a
room, at whatever pace its client batches and the homeserver syncs. Fifteen
seconds, and a `Connected` in between cancels it.

**Both of these were reached by watching a call that should not have failed.**
The two ends were on one switch, both had the other's host address, and it
still ended in five seconds — which is what makes the pair list, rather than
the network, the thing to look at.

### The one procedure to run

One account signed in on exactly two clients, and nothing else ringing:

1. `RUST_LOG=commune=debug commune 2>&1 | tee /tmp/call.log`
2. Place **one voice call** from Commune to the other client.
3. Answer it there. Leave it for twenty seconds whether or not it connects.
4. Hang up. Stop Commune.

Then:

```sh
grep -E "Placing call|Sent m.call|from party|Replaying|not the call in progress|ice-ufrag|gathered a (relay|srflx)|ICE connection state" /tmp/call.log
```

Those lines say, in order: which room the call went into and who it was
addressed to, the event ID of every event we sent and whether it was
encrypted, which party answered, whether any candidates had to wait for their
invite, which ICE session they belong to, whether a relay was obtained, and
where ICE ended up.

When the other end never rings, the log above has said all it can. The next
question is whether the invite reached that device at all, and it is answered
there: look for the event ID in the room on the other client — Element draws a
timeline row for a call it received and ignored, and nothing at all for one it
never got.

**Sign every other client out first.** A second session of the same account
that rings and does not answer is what produced the split above, and it is
indistinguishable from a network fault unless it is ruled out deliberately.

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

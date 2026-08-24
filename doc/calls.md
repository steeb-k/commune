# Voice and video calls — downstream implementation notes

This file is the ledger for one-to-one calls: what the fork added, the
decisions behind it, and what to check when rebasing onto a new Fractal
release. See `fork.md` for why none of this goes upstream.

**Status: every event the module defines is now handled, and the last three
additions have not been exercised against another client.** Placed and
answered against Element for Android on 23 August 2026, carrying audio and
video: on one network at 21:13, and between two — the phone on mobile data, a
relay outside both — at 23:04, where `Checking` became `Connected` in six
hundred milliseconds. Offer, answer, `select_answer`, candidates, hangup and
muting all behave.

Written on 23 August 2026, and **the camera was added to a live voice call
against Element for Android the same evening**. The `m.call.negotiate` carrying
the new offer went out, Element's answer came back 850 ms later having accepted
the video section as `recvonly` — it agreed to receive a camera it has no
control to send one back from — and a second later every element the camera
brought with it had settled at `Playing`, the self-view included. That took
three attempts, and the section on adding a camera says what the first two got
wrong. What has still never happened is a renegotiation arriving _at_ this
client, since nothing in that peer's interface would send one.

The ringtone, the notification with its two buttons, and the fullscreen window
were all exercised the same evening and behave. What has not been seen work is
the badge saying the other end muted something — the peer to hand sends no
stream metadata at all — the answered and missed cases of the row a call leaves
in the room, and the rollback that settles two renegotiations crossing. Nothing
is left of the module that this client does not answer; what is left out on
purpose is DTMF and screen sharing, and the last section says why.

The evening that produced this is worth a sentence of warning: five separate
faults, four of which looked identical from here — ICE reaching `Checking` and
dying — and each of which hid the next. Anything below that reads like a
diagnosis is one that was measured, not inferred.

Testing it needs two clients, and two accounts inside one Commune are not two
clients: `Calls` holds one call per session, so a second call from the same
account is refused by design, and two pipelines in one process would be
fighting over one camera and one microphone anyway. Use two machines, or
Commune against Element.

## Scope

The Voice over IP module of the Client-Server API as written: `m.call.invite`,
`m.call.candidates`, `m.call.answer`, `m.call.select_answer`, `m.call.reject`,
`m.call.hangup`, `m.call.negotiate` and `m.call.sdp_stream_metadata_changed`,
version `1`, carrying WebRTC between exactly two devices. Audio and video,
placed and answered, in a window of its own, with a row in the room afterwards
saying what became of it.

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

**Element for Android sends none of this.** Measured on 23 August 2026: its
answer to a call of ours, and an invite it placed, both arrived with no
`sdp_stream_metadata` at all. The spec's instruction for that is to assume the
other party does not support the property — which is followed — and it means
the badge below can never appear against that client, in either direction.
What cannot be told apart from here is a peer that does not support it from
one that sends it under `org.matrix.msc3077.sdp_stream_metadata`, the name it
had while it was a proposal, so the names of the fields that _did_ arrive are
logged whenever the stable one is absent. Reading the unstable name as well
would be in keeping with how this fork treats image packs — read both, write
the specified one — and is not worth writing until a log says a peer uses it.

**Metadata arrives on four events and all four are read.** It is a property of
the invite, of the answer and of an `m.call.negotiate` as much as of the event
named after it, and until 23 August 2026 only the last of those was read — so a
call answered by somebody whose microphone was already off looked, from here,
like a call with a working microphone. `apply_stream_metadata()` is now what
all four go through. An empty map is not "nothing is muted": the spec says to
read the absence of the property as the other end not supporting it, so an
empty one changes nothing rather than clearing what an earlier event said. Only
`m.usermedia` streams are counted, because a purpose this client does not know
is one the spec asks it to ignore.

**And the mute is said out loud.** A camera going off shows as an avatar, which
on its own is indistinguishable from a call that has stopped working, and a
microphone going off has nothing to show at all — so the window carries a badge
over the picture saying which of the two it was, in the words of the person who
did it: "Alice muted their microphone". Both directions of the module's
asymmetry end up visible: what we mute goes out as metadata, and what they mute
comes back as a sentence.

## Renegotiation

`m.call.negotiate` is the one event the module defines that this client did not
answer until 23 August 2026. It is a call being described again after it is
established — adding video to a voice call, hold and resume, an ICE restart —
and it carries **both halves**: an offer first and then an answer, in two
events of the same type, told apart only by the `type` of the description
inside them.

Receiving one is the part that matters for conformance, and it is a straight
path: apply their offer, answer it from inside the promise of
`set-remote-description` for the same reason the first answer is made there,
and send the answer back as another `m.call.negotiate`. An answer to an offer
of ours is applied and nothing is sent.

Three things are worth knowing about it:

* **A renegotiation that fails does not end the call.** The media that was
  flowing before it carries on flowing, and hanging up because a camera could
  not be added would take away a working call over an optional extra. Failures
  are logged and the call is left alone.
* **A renegotiation is refused before the call is established.** "This event is
  sent by either party after the call is established" — and applying a second
  description on top of one still in flight is how a call that was about to
  connect stops instead.
* **A stale one is dropped.** `m.call.negotiate` carries a `lifetime` like an
  invite does, and one that expired on the way here describes the call as it
  was. `is_stale()` is now shared by both.

**Politeness is the callee's.** Two offers can cross, and WebRTC's perfect
negotiation settles it without either end asking the other: "the callee is
always the polite party", so the callee rolls its own offer back and takes
theirs, and the caller ignores theirs and lets its own stand. Both ends run the
same rule and reach the same answer, exactly as glare does one layer up. The
rollback is `set-local-description` with a description of type `rollback`;
`webrtcbin` accepts the type and this path has never been exercised, because
reaching it takes two people adding video in the same second.

## Adding the camera to a call that was placed without one

The reason to send a renegotiation at all, and the only thing in this client
that does. The button appears in a connected call with no video section, and
what it does is put the camera into the pipeline: `enable_video()` builds the
same chain `add_video_source()` builds at setup, links it to a new
`webrtcbin` sink pad — a new transceiver — and syncs every element with the
parent, because everything added to a pipeline arrives in `Null` however the
pipeline itself is doing.

**Everything added to a playing pipeline arrives in `Null`**, and an element in
`Null` produces nothing — so the new elements are synced with the parent
afterwards.

**The new ones, and only the new ones.** `sync_state_with_parent()` sets an
element to whatever state the parent is in _at that moment_, and adding a live
source makes the pipeline drop briefly back to `Paused` while it prerolls.
Syncing every child during that window pushes the whole call down with it.
Measured on 23 August 2026, in a call that had been carrying audio for fourteen
seconds:

```text
capsfilter5, rtpvp8pay0, vp8enc0, capsfilter4, videoscale0, ...  Playing
gtk4paintablesink1                                               Ready
autovideosrc1, valve0, tee0, queue3, videoconvert0, capsfilter3  Paused
autoaudiosrc0, opusenc0, volume0, decodebin0, webrtc             Paused
```

Two elements of the new chain reached `Playing`, and everything after them —
the camera included, the microphone and `webrtcbin` included — was set to
`Paused` behind them. That is what a black self-view looks like from the
inside, and it is why the pipeline's own state changes are logged now: one
line saying `Paused` beats thirty saying nothing.

Syncing only the new elements fixed the audio and not the camera, because the
same trap is one level down: the sink this adds has to preroll, the pipeline
sits in `Paused` while it does, and the source synced during that window is set
to `Paused` — so it produces nothing, so the sink never prerolls, so the
pipeline never leaves `Paused`. The two wait for each other for the rest of the
call, and the log said so in one line: `The pipeline is now Paused`, and never
another state change after it. **So the state is named rather than read**:
every new element is set to `Playing` outright and the pipeline is put back to
`Playing` behind them. Measured settling a second later, which is also why that
dump is taken a second later — every state read at the instant of the change is
a state on its way somewhere, and reading those is what made the first fault
look like the second.

`children()` and not `iterate_elements()` for a second reason: a `GstIterator`
can ask to be resynced when the bin changes underneath it, and the obvious
`while let Ok(Some(_))` loop reads that request as the end of the list.

Nothing there writes SDP. `webrtcbin` notices that what it sends no longer
matches what it last described and emits `on-negotiation-needed`, and that is
what makes the offer. The signal is **armed only once the call is connected**:
it fires for the first negotiation of every call as well, which this client
drives by hand, and honouring that one would put a second offer on the wire for
every call placed.

A camera that is not there is refused rather than renegotiated: without one,
`add_video_source()` falls back to a receive-only section, which is the right
answer while the sections are still being chosen and the wrong one here — the
other end was never asked for video, so a section to receive it in would carry
nothing.

An offer of ours that goes unanswered clears itself after thirty seconds.
Without that, one lost renegotiation leaves the call unable to attempt another
for as long as it lasts, and the call carries on working so nothing else would
ever notice.

**Proven, on 23 August 2026 at 01:49.** The offer went out, Element answered in
850 ms accepting the video as `recvonly`, and a second later every element of
the camera chain — `autovideosrc` and the self-view sink included — was in
`Playing`, with the pipeline back in `Playing` behind them. Whether that peer
_draws_ the video it agreed to receive is its own business; the call carries
it.

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

**The video page waits for video, not for a paintable.** The remote paintable
exists from the moment the pipeline does and has nothing to draw until a
decoded video pad is attached to its sink, so a voice call that switched on the
paintable's existence showed an empty rectangle where the other person's face
goes. `PipelineEvent::RemoteVideo` is sent when that pad is attached, and it is
what the window switches on — which is also what makes a call that gains video
partway through show it without anything else having to notice.

**Fullscreen** is a button in the header bar, `F11`, or a double click on the
picture. Going fullscreen hides the header bar, which is where that button
lives, so a second one appears over the picture: `Escape` is the other way out
and a screen with no keyboard has to have one. The double click is wired in
Rust rather than in the template — a template callback's arguments are checked
at run time and only at run time, and `pressed` has three of them.

## Ringing

A window on its own is not enough. It opens behind whatever is on screen, on
whichever workspace the client happens to be on, and a call nobody is looking
at rings for ninety seconds and is gone.

**The sound is the desktop's own.** The freedesktop sound theme names the event
this needs — `phone-incoming-call` — so `Ringtone`
finds the file under `<data dir>/sounds/<theme>/stereo/` and loops it on a
`playbin3`, looping being what turns one ring into a ringing telephone. The
theme comes from `org.gnome.desktop.sound`, which is looked up in the schema
source first because constructing a `gio::Settings` for a schema that is not
installed aborts the process. `event-sounds` set to false is honoured: the
person who switched the desktop's sounds off switched this one off too, and the
notification still arrives. A theme without that event is silent and says so in
the log — no beep of our own invention. **macOS has no such theme, so macOS
does not ring.**

**Only calls coming in.** The theme has `phone-outgoing-calling` for the other
direction, it was wired up on 23 August 2026, and it was taken out again the
first time anybody placed a call with it: a telephone plays a ringback because
the caller has nothing to look at, and here the window is open in front of them
saying `Calling…`. All the sound added was a noise in the caller's own room.

**And not at the person who placed the call.** This client holds several
accounts at once, and a call from one of them to another is a real call — the
window opens, the invite is real, either end can answer. But the person being
rung at is the person who just pressed the button, in the same window, and
telling them about it is telling them nothing. So an invite from an account
that is open in this application rings for nobody and raises no notification.

That one took a log to see. It looks exactly like a ringback that would not go
away — the sound is the same file — and the first two attempts at a fix were
aimed at the wrong direction entirely. `is_logged_in_here()` is the test, and
the log says so when it suppresses.

**The notification is not the push path's.** The homeserver's `.m.rule.call`
push rule fires for an `m.call.invite` and reaches `show_push()`, which until
now dropped it as an event of unexpected type — that is why there was no
notification at all. It could have been given a body there, and it is not:
`Calls` owns this one, because it has to be withdrawn the moment the call stops
ringing and it carries **Answer** and **Decline** buttons, which want an app
action rather than a room to open. `show_push()` now returns early for a call
invite with a comment saying so, and the two cannot end up as two notifications
for one call.

The buttons go through a new `SessionIntent::CallAction`, carrying the call ID.
The ID is checked and not trusted: a notification outlives the call it is about
— it can sit in a notification centre for an hour — and answering "the call
that is happening" would answer whichever one happens to be happening now.

## The call in the timeline

A call used to leave nothing behind. The whole of `m.call.*` was filtered out
of the timeline, so a missed call was gone the moment the window closed, and
the spec asks for the opposite in the one case where this client does not ring:
"when clients suppress ringing for an incoming call invite, they SHOULD still
display the call invite in the room and annotate that it was ignored".

`m.call.invite` is now shown, and only the invite. The SDK gives a timeline
item for exactly that one — `TimelineItemContent::CallInvite`, a unit variant
carrying nothing — and none for the answer, the hangup or the reject, which is
just as well: a dozen rows for one call is not history, it is a log.

What became of the call is not in the invite, so it comes from `Calls`, which
watches every `m.call.*` event the session sees whether or not it is in the
call — a call answered on another device is still one the room has to describe.
`note_outcome()` keeps one enum per call ID, bounded at 256 and never written
to disk, and `merge_outcome()` is the whole of the logic: a hangup means the
call is over, so on its own it says nobody answered, unless an answer came
first — every call ends with a hangup, and without that rule every call in the
timeline would end up saying it was missed.

The row's own facts come back out of the event's JSON, since the SDK's item
carries none: the call ID, and whether the offer had a video section, because
whether a call is a video call is in its SDP and nowhere else. In an encrypted
room that JSON is the decrypted event, which is what makes this work in the
rooms calls actually happen in.

**A call from before the client was running has no outcome and the row says so
in what it leaves out**: "Incoming call from Alice." rather than a guess at
whether she was answered. Nothing here is persisted, and inventing an answer
would be worse than a row that only repeats what the invite said.

## Files

Integration points, which are where a rebase will conflict:

| File | Change |
| --- | --- |
| `src/session/calls/` | All of it, new |
| `src/session_view/call_view/` | All of it, new |
| `src/session/mod.rs` | The `calls` property, and `init()` in `prepare()` |
| `src/session/room/mod.rs` | `handle_member_event()` tells `Calls` when somebody leaves |
| `src/session_view/mod.rs` | Opening the window when a call appears; `handle_call_action()` |
| `src/session_view/room_history/mod.rs` | The two header buttons and when they are shown |
| `src/session_view/room_history/mod.blp` | The buttons themselves |
| `src/session_view/room_history/call_row.rs` | The `m.call.invite` row, beside the `m.rtc.notification` one it already drew |
| `src/session/room/timeline/mod.rs` | `show_in_timeline()` lets `m.call.invite` through |
| `src/session/room/timeline/event/mod.rs` | `is_call_event()`, and `call_invite()` reading the event's own JSON |
| `src/session/notifications/mod.rs` | `show_incoming_call()`, its withdrawal, buttons on a notification, and the early return for a call invite in `show_push()` |
| `src/intent.rs`, `src/application.rs` | `SessionIntent::CallAction` and the app action behind it |
| `data/resources/stylesheet/_session_view.scss` | `call-button`, `call-self-view`, `call-badge` |
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
it is checked ahead of the denied ranges. Adding it fixed the `403`, measured
the same evening — two allocations, permissions granted both ways, and a
packet delivered between them.

**It was not, in the end, what the calls were failing on.** With relay-to-relay
open they still failed, and so did calls through a hosted relay outside both
networks, which has none of this deployment's problems. The fault was the
missing `sdpMid` two sections up. This section stays because the measurement
is sound and the topology is real — a TURN server behind the same NAT as one
of its callers is a bad place for one to be — but it is a thing to fix for its
own sake, not a thing that was stopping a call.

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

### What five failures turned out to be

Written out because each one masked the next, and because four of the five
looked identical from here — ICE reaching `Checking` and dying.

1. **`a=rtpmap:111 OPUS/48000`.** Element rejected the audio section of every
   call this fork ever placed to it. A voice call was answered with nothing
   accepted at all; a video call came back video-only. See "Opus is offered
   with two channels".
2. **Remote candidates hard-coded onto section 0.** When the peer rejects the
   audio section its transport is on section 1, and everything we added named
   a section with no transport. This one was a regression, and the two video
   calls that worked that morning worked because the code then used the index
   the peer sent.
3. **Candidates dropped when they arrived before their invite**, which on the
   answering side was every candidate the call was going to get.
4. **Two remote candidates on one transport address**, which aborted the
   process inside libnice. See the section on `conncheck.c:959`.
5. **`m.call.candidates` sent with no `sdpMid`.** The last one, and the one
   that made the other four so hard to see: the far end discarded every
   candidate we sent, so it never checked and never opened its relay to us,
   and every diagnosis pointed at the network.

**The ufrag mismatch was ours.** An answer whose candidates all carried a tag
the description did not name looked like the far end trickling from a second
ICE session. It was not: the answer had two `a=ice-ufrag` lines, one per
section, and we were reading the rejected section's. Reading the bundled
section's fixed it. Dropping a mismatched tag rather than the candidate is
kept, because libnice discards such a candidate itself and the address is
never the part in doubt.

### The procedure when one does not connect

1. `RUST_LOG=commune=debug commune 2>&1 | tee /tmp/call.log`
2. Place or answer **one** call. Leave it twenty seconds whether or not it
   connects. Hang up, stop Commune.

```sh
grep -E "Placing call|Sent m.call|Our own (offer|answer)|Setting the remote|\
transport is on m-line|Adding remote|Ignoring a|gathered a (relay|srflx)|\
ICE connection state" /tmp/call.log
```

Both descriptions are logged in full, which is what finally made the difference:
the opus rtpmap, the rejected section and its stale `ice-ufrag` are all things
that can only be seen by reading the two SDPs side by side.

When ICE stops at `Checking`, the next instrument is libnice's own:

```sh
NICE_DEBUG=nice G_MESSAGES_DEBUG=all RUST_LOG=commune=debug commune 2>&1 | tee /tmp/ice.log
```

It prints every pair it builds and what each check did. `Failed pair is …`
with `timer=3/3` against every pair, and no inbound STUN packet from anywhere
but the TURN server, is the signature of a far end that never answered — which
means it never had our candidates, not that the path was broken.

**Test against a homeserver whose TURN is known to work before blaming the
client.** An evening went into faults that were not the client's: a coturn
behind the same NAT as one caller, split-horizon DNS that cost that caller its
reflexive candidate, `denied-peer-ip` covering the server's own address, and a
relay port range that had to be moved. A free account elsewhere would have put
the one client bug in plain sight.

## What is written and not proven

Every event in the module is answered, and most of what was written on
23 August 2026 has now been watched working. What follows has not.

1. **A renegotiation arriving here.** Ours is sent and answered; the reverse
   needs a peer whose interface offers to add video to a call in progress, and
   Element for Android's does not. Two Communes on two machines would do it.
2. **The rollback when two renegotiations cross.** Reaching it takes two people
   pressing the same button in the same second, and `webrtcbin`'s handling of a
   `rollback` description has not been watched.
3. **The badge for what the other end muted.** Our half goes out and is logged
   going out; the peer to hand sends no stream metadata at all, so there has
   never been anything to draw. Commune against Commune would settle it.
4. **The row in the timeline, past the unknown case.** A call from before the
   client started draws correctly. Answered, missed and declined have not been
   read off a screen.

And the older list of things nothing here has been able to exercise:

1. **`autoaudiosrc` and `autovideosrc` under a portal.** On a sandboxed
   desktop the camera wants `pipewiresrc` through the portal, and
   `autovideosrc` may pick a `v4l2src` that cannot open the device. On the
   machine this was built on — an IPU6 camera on Arch — `autovideosrc` opened
   it and the frames were fine, so this is not a given failure, only an
   untested one.
2. **A second answering device.** `select_answer` and `AnsweredElsewhere` are
   written and have never had two devices to exercise them.
3. **macOS.** Everything above was measured on Linux. `doc/macos.md` records
   what had to be built by hand to make `webrtcbin` exist there at all, and
   the ringtone is silent there for want of a freedesktop sound theme.

Absent on purpose:

* **DTMF.** Not a Matrix event at all: "Matrix clients can send DTMF as
  specified by WebRTC", which means RFC 4733 telephone-events in the RTP. The
  spec allows it and requires nothing; there is nothing in this client that
  would want to send one, and a bridge that wants to receive one is not a
  thing this client talks to.
* **Screen sharing.** A second stream with `purpose: m.screenshare`. The
  metadata plumbing is there for it, the pipeline is not.
* **Persisted call history.** What became of a call lives in memory for as long
  as the session does. A client that was not running when a call happened
  cannot know how it ended, and the row says only what the invite said rather
  than guessing.

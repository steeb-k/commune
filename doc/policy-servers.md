# Policy servers — downstream implementation notes

What the timeline says when a room gains, changes, or loses a policy server
(MSC4284, the `m.room.policy` state event). Built in round 6 of
`doc/gap-closing-plan.md`, 26 August 2026.

## The client role is one state event

The plan originally said "mark spam-checked events", and the spec says no
such thing: "Clients do not interact with the Policy Server directly." The
checking happens between homeservers; a client never learns which events
were checked, and events a policy server flags are redacted or rejected
before a client would draw them. The whole client-side surface of the
feature is the `m.room.policy` state event — optionally helping set it, and
saying what it means when it appears in the timeline. This fork does the
saying; setting one is a moderation action that can join the room settings
if a round ever wants it.

## What is drawn

`m.room.policy` joins the state allow-list in
`src/session/room/timeline/mod.rs` — which server checks a room's messages
is an act of moderation too, and one worth a sentence. The sentence is built
in `src/session_view/room_history/state/content.rs`: the pinned ruma
(`db24422`) has `RoomPolicyEventContent`, but the SDK's state-change enum
does not carry the type, so the event lands in `update_with_other_state`'s
raw arm and the content is read off `event.raw()`. An event whose `via`
parses names the server: "{sender} made {server} check the messages of this
room." Everything else — a content written empty to unset the policy server,
and a redacted one — reads as the removal: "{sender} stopped the checking of
this room's messages."

## Rebase guide

* The allow-list arm in `timeline/mod.rs` is one line next to its siblings.
* `update_with_other_state` took a new `event: &Event` parameter to reach
  the raw JSON; its one caller passes it through. If a later SDK adds
  `m.room.policy` to `OtherState`, the raw-arm guard can become an ordinary
  match arm and the parameter can go away.

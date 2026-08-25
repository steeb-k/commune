# Presence — downstream implementation notes

This file is the ledger for the Presence module: what the fork added, the
decisions behind it, and what to check when rebasing onto a new Fractal
release. See `fork.md` for why none of this goes upstream.

## Scope

* A badge on an avatar saying whether that person is around, in the member
  list and on a user's profile page.
* The status message a person set to go with it, on their profile page.
* A switch for whether this client tells the homeserver that _you_ are here.

Upstream reads no presence, publishes none deliberately, and shows none.
The word does not appear anywhere in its source.

## We were already publishing it, and could not stop

The interesting half. `GET /sync` takes a `set_presence` parameter, and the
spec says that omitting it marks the client online. The SDK agrees: its
client-owned sync presence starts at `PresenceState::Online` and every sync
request carries it. So every sync this client has ever made has told the
homeserver its user is here — on a homeserver that keeps presence, other
people have been watching a green dot next to this fork's users all along.

That decides the default. The switch is on, because the behaviour it controls
is what already happens, and defaulting it off would quietly change what other
people see about somebody who never asked for that. **The setting adds an off
switch that did not exist, not an on switch.** Sketched the other way round in
the plan for this work, on the grounds that publishing was "a wasted request";
it is not a request at all, it is a query parameter, and it was never optional.

`Client::set_presence(presence, None, true)` does both halves: it sets the
value future syncs carry, and `immediate` pushes it to
`PUT /presence/{userId}/status` so flipping the switch is visible without
waiting for a sync. On a homeserver without the module that call fails; it is
logged at debug, not warn, because it is the common case rather than a fault.

## Unknown is not Offline

`Presence` has four values where the spec has three. A homeserver that runs the
module says `offline` out loud; a homeserver that does not says nothing at all,
and the two must not be confused in the model.

They are drawn the same, though — `Presence::is_visible()` is true only for
`Online` and `Unavailable`. Drawing "offline" apart from "unknown" would ask a
person to tell a server that keeps presence from a server that does not, from
looking at a dot. Absence of a badge means "not known to be here", which is
honest in both cases.

The distinction is kept because the model is where it is cheap to keep and
because a profile page could one day say "Offline" in words, where there is
room to be precise.

## One registry, not a lookup per user

`PresenceList` hangs off the session next to `IgnoredUsers` and is the same
shape: one object the whole session shares, and every `User` watches it for
its own ID. Presence is a top-level field of the sync response rather than
anything belonging to a room, so an event handler on `PresenceEvent` is the
whole of the live half.

Sync only sends presence when it _changes_, so somebody who has not moved since
this client started would have none at all. `PresenceList::load()` reads what
earlier syncs put in the store, once per user, the first time a `User` object
for them is built.

The `changed` signal carries the user ID so a `User` can drop what is not
about it without touching the map. A plain signal would have every member of a
large room re-read on every heartbeat from anyone.

## The badge is opt-in, and the model is not

`AvatarData` carries the presence, so setting it once on a `User` reaches every
avatar bound to that user anywhere in the application — there are twenty-odd
such places. `Avatar` then draws the badge only when the site asked for it with
`show-presence`, which is off by default.

Opt-in because most of those places are not asking the question. An inline
mention, a read receipt, an invite picker, a permissions picker and an ordinary
room's avatar do not want a dot. Three sites are opted in: the member list, a
profile page, and the sidebar row of a direct chat.

The direct chat one is not a user's avatar at all — it is the room's. A direct
chat is the one room that _is_ a person, so `Room` mirrors its `direct_member`'s
presence onto its own avatar data, the same way it already borrows that
member's picture when the room has none. Every other room stays at
`Presence::Unknown` and so draws nothing, which is why the sidebar can opt in
wholesale rather than per row.

The overlay carrying the badge has to hug the avatar — `halign: center` on it —
or it fills whatever the avatar widget was given and the badge lands at the
right edge of the container instead. On a profile page that is a few hundred
pixels from the avatar. This was wrong on the first pass and is the kind of
thing only a screenshot catches.

`OverlappingAvatars` is the one that could not have it anyway: it crops its
children, so a badge in the corner would be cut in half.

## Files

| File | What |
| --- | --- |
| `src/session/presence.rs` | `Presence`, `UserPresence`, `PresenceList`, the sync handler, the store read, the share setting |
| `src/session/mod.rs` | `presence-list` on the session |
| `src/session/user.rs` | `presence`, `presence-status-message`, the watcher, pushing into `avatar_data` |
| `src/components/avatar/data.rs` | `presence` on `AvatarData` |
| `src/components/avatar/mod.blp`, `mod.rs` | The overlay, `show-presence`, the size rule |
| `data/resources/stylesheet/_components.scss` | `.presence-badge` and its two colours |
| `src/session_view/room_details/member_row.blp`, `src/components/user_page.blp` | The two sites that opted in; the profile also shows the status message |
| `data/…gschema.xml.in`, `src/account_settings/general_page/` | `share-presence` and its switch |

## Rebase guide

1. If upstream ever sets `SyncSettings::set_presence`, it is deciding the same
   thing this setting decides and one of them has to go.
2. `AvatarData` gaining a property is the kind of thing that conflicts quietly.
   If the badge stops appearing, check that `Avatar::set_data()` still connects
   `connect_presence_notify` — without it the badge is right once and never
   updates.
3. `show-presence` defaults to off, so a lost line in a `.blp` removes a badge
   silently. The two sites are named in the table above.
4. `Presence::is_visible()` is the single place that decides what is drawn.
   Change it rather than the four call sites.

## Not done

* **`last_active_ago` and `currently_active` are parsed and never shown.** A
  "last seen four minutes ago" label has to tick, and a static one is a small
  lie that gets larger the longer the page is open. The values are in
  `UserPresence` for whoever wants to build that.
* **No presence on message rows.** Deliberate: a dot per message is noise, and
  the sender's state now is not the sender's state when they wrote it.
* **You cannot set your own status message.** The switch is binary. Sending one
  means `set_presence` with a `status_msg` and somewhere to type it.
* **No idle detection.** This client says `online` while it syncs and `offline`
  when told to; it never says `unavailable` about itself. The homeserver's own
  idle timeout is what produces that for other people.

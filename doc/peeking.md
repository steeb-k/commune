# Peeking — downstream implementation notes

This file is the ledger for reading a room without joining it: what the fork
added, the decisions behind it, and what to check when rebasing onto a new
Fractal release. See `fork.md` for why none of this goes upstream.

The Matrix specification calls this **peeking**, and it applies to a room whose
`m.room.history_visibility` is `world_readable`. Everything here is read-only
and always will be: there is no `Room`, so there is nothing to send with, mark
as read, or react to.

## What it looks like

`RoomPreviewDialog` grew a **Preview** button beside _Join_, and a page behind
it holding the last twenty messages. `PublicRoomRow` — the row used by both
Explore and the space page since `doc/spaces.md`'s slice 2 — grew the same
button, which opens the dialog directly on that page.

Both buttons appear only when the room summary says `world_readable`, the room
is not encrypted, and it has not already been joined. That last condition is
not squeamishness: a joined room has a room history, which is better than this
in every way.

## Why `/messages` and not the endpoints named "peek"

The specification has three ways to read a room you are not in, and two of them
are dead ends:

* `GET /rooms/{roomId}/initialSync` (`peeking::get_current_state::v3` in ruma)
  is the legacy peek endpoint. It works, and it is the one the module is named
  after, but it is deprecated and returns a shape nothing else in this tree
  reads.
* `GET /events` with a `room_id` is the long-polling half of the same legacy
  API. It is for _watching_ a room, which is more than a preview needs and more
  than the SDK will help with.
* `GET /rooms/{roomId}/messages` is the ordinary pagination endpoint, and
  Synapse answers it for a non-member when the room is `world_readable`
  (`check_user_in_room_or_world_readable`). It returns the same event shapes as
  every other timeline in the tree.

The third one wins. It is sent raw through `client.send(request)`, the pattern
already used for `get_summary::v1` in `remote/room.rs` — the SDK's timeline
machinery is not involved, because all of it starts from a `Room` and there is
no `Room` for a room nobody has joined.

## It will usually fail, and that is the protocol

A homeserver can only answer `/messages` for a room it has. Synapse does not
peek a **remote** room over federation, so previewing a `world_readable` room
on another homeserver — which is most of them — answers with an error. The
peek page says so in as many words, and the _Join_ button stays on it, because
joining is what still works.

This is why the failure is logged at `debug` and not `warn`: it is the expected
outcome, not a fault.

## What is deliberately not drawn

The row (`components/dialogs/peek_row.rs`) shows a sender name, a timestamp and
the plain-text body. It does **not** show avatars, images, files, replies,
reactions or formatted text.

That is a decision, not a shortcut. Everything missing from that list would
mean fetching media out of a room the person looking at it has not joined, and
the tree already has a safety setting for exactly that worry —
`AvatarImageSafetySetting::MediaPreviews`, which cannot be used here because it
takes a `Room`. Rather than reimplement the setting against an object that does
not exist, the preview fetches no media at all. A preview is a taste of who is
talking and about what; it is not the room.

The consequence to know: an image-only message shows as its fallback body
(`image.png`), which is what the sender's client put there.

## Sender names come from lazy loading, or not at all

There is no member list for a room nobody has joined, so the names come from
the `state` block of the `/messages` response, which the server fills when the
filter asks for `lazy_load_members`. The filter also restricts `types` to
`m.room.message`, so the preview never has to decide how to draw a topic
change.

A name claimed by two people is dropped for **both** of them and each falls
back to their user ID. The specification requires that disambiguation, and in a
room whose member list cannot be read the user ID is the only thing left that
tells two Alices apart. `Member::disambiguated_name` does the same job for a
joined room and could not be reused, for the usual reason.

Edits are folded in by `original_message_event_from_raw`, the same helper
message search uses, so a preview shows the current text of a message and not
both versions. Redacted messages fall out of the list entirely, which is
correct.

## Twenty messages, once

`PEEK_LIMIT` is 20 and there is no scrollback: the peek is loaded once when the
button is pressed, backwards from the end of the room, and reversed for
display. Pressing _Preview_ again after a failure retries, because `set_room`
re-enters when the state is `Error`.

## Files

| File | What |
| --- | --- |
| `src/session/remote/room_peek.rs` | `RoomPeek` and `PeekedMessage`: the request, the sender names, the list |
| `src/components/dialogs/peek_row.rs`, `peek_row.blp` | One message, three labels |
| `src/components/dialogs/room_preview.rs`, `.blp` | The _Preview_ button, the `peek` page, the back stack |
| `src/session_view/explore/public_room_row.rs`, `.blp` | The same button on a row, in Explore and on a space page |
| `src/session/remote/room.rs` | `is-encrypted`, beside slice 2's `is-world-readable` |
| `data/resources/stylesheet/_components.scss` | `.room-peek` |

## Rebase guide

1. **`Raw::cast_ref_unchecked` is load-bearing.** `/messages` returns
   `Raw<AnyTimelineEvent>` and `original_message_event_from_raw` takes
   `Raw<AnySyncTimelineEvent>`. The JSON of the first is the JSON of the second
   plus a `room_id`, so the cast is sound, but ruma has no `JsonCastable` impl
   for the pair and will not tell you if that ever stops being true.
2. The dialog's stack now has four pages, and `go_back` walks
   `peek → details → entry` by name. An upstream change that renames a page or
   adds one has to be read against `can_go_back` as well.
3. `fill_details` calls `show_details_page`, which refuses to act while the
   peek is on screen. Without it, a room whose summary arrives late yanks the
   preview away as it lands.
4. `PublicRoomRow` has three callers' worth of buttons on one line now. It is
   also used by the space page — see `spaces.md`.

## Not done

* **No pagination.** Twenty messages and that is the room.
* **No formatted text, no media, no avatars.** See above; this is a decision.
* **The legacy peek endpoints are not used at all**, so a homeserver that
  answers `/initialSync` but not `/messages` for a non-member gets nothing.
  No such homeserver is known.
* **Nothing peeks over federation**, because Synapse does not. If a homeserver
  ever does, this code needs no change — it will simply start working.
* **The space page does not peek its own children inline.** The button on each
  row opens the dialog. A peek drawn into the space page itself would need the
  list to be reusable outside a dialog, which it is, but nothing asks for it
  yet.

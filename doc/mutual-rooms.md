# Mutual rooms — downstream implementation notes

The "Shared Rooms" section on a user's profile page: the rooms both accounts
are joined to, from `GET /_matrix/client/v1/mutual_rooms`. The endpoint
started as MSC2666 and became stable in spec v1.19. Built in round 6 of
`doc/gap-closing-plan.md`, 26 August 2026.

## The request is spelled out by hand

The pinned ruma (`db24422`) only knows the unstable MSC2666 path, and its
`#[request]`/`#[response]` proc-macros panic when expanded outside ruma's own
tree ("Failed to parse Cargo.toml"), so defining the stable endpoint the
polite way is off the table at this pin. Hand-implementing `OutgoingRequest`
against this pin's path-builder machinery was the other option considered and
rejected as more code than the endpoint deserves.

Instead `src/utils/matrix/mutual_rooms.rs` builds the request over the SDK's
own reqwest client (`client.http_client()`, `client.access_token()`): the
stable path first, then the `uk.half-shot.msc2666` unstable path when the
stable one 404s — a homeserver that has not caught up to v1.19 still answers
the old name. Any other failure, and a server that knows neither path,
returns `None`, which the page reads as "do not draw the section": the
endpoint is recent, and its absence should look like nothing rather than an
error. Pagination is followed up to five pages; a pair sharing more rooms
than that is not going to read the whole list anyway.

When ruma's pin gains the stable endpoint, delete this file and use it.

## The section

`src/components/user_page.rs`/`.blp` gain a "Shared Rooms" box between the
identity rows and the direct-chat button. It is loaded when the page is set
to a user other than our own (the endpoint 400s on the asking account's own
ID, per spec), guarded by a generation counter so a stale answer for the
previous user cannot draw on the next one. Only rooms the session can
resolve through `room_list().get()` are drawn — the server can name a room
the local store has not caught up with — each as an `ActionRow` with the
room's avatar; activating one selects the room and closes the profile
window. The rows and the room list are kept in index order rather than
stashing IDs on the row widgets.

## Rebase guide

* `mutual_rooms.rs` is new, self-contained, and touches no upstream code.
* The `user_page` changes are additive: two template children, two fields,
  three methods. If upstream reworks the profile page, the section rides
  along; the one ordering constraint is that `load_mutual_rooms` runs when
  the user is (re)set.

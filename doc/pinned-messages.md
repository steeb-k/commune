# Pinned messages — downstream implementation notes

This file is the ledger for `m.room.pinned_events` in any room: what the fork
added, the decisions behind it, and what to check when rebasing onto a new
Fractal release. See `fork.md` for why none of this goes upstream.

## Scope

* Pin and unpin a message from its context menu, subject to the power level.
* A view listing the pinned messages of the room, reached from the header bar,
  where each row can be clicked to jump to the message and unpinned in place.
* A line in the timeline saying who pinned, unpinned or reordered — instead of
  nothing at all.

Upstream reads `m.room.pinned_events` in exactly one place: the server notices
room, to find which notice is still active. That reader stays where it is, for
the reason in the next section.

## The server notices room keeps its own UI

The Server Notices module says the active notices are "represented by the
pinned events of the room" and asks that they be shown "through a special UI,
and not the normal pinned events interface". So the two do not share code and
must not:

* `Room::update_active_server_notice()` reads the pinned IDs of the notices
  room and collapses them to the one banner above the timeline.
* `Room::update_pinned_events()` reads the pinned IDs of every _other_ room and
  is hard-coded to return an empty list in the notices room.

The two are siblings in the same file and both run from
`update_with_room_info()`. Neither is a generalisation of the other, and trying
to merge them would put a notice into the ordinary pinned list.

## The pinned list is a timeline, not a hand-rolled list

`TimelineFocus::PinnedEvents` builds an SDK timeline from the room's
`m.room.pinned_events` and follows it as the state changes. That gets the whole
existing stack for free — decryption, edits, redactions, the sender profile —
where a hand-rolled list of `load_or_fetch_event` calls would get none of it.

`Timeline::new_pinned()` is the third constructor beside `new` and
`new_focused`, and all three funnel through `construct()`. Two things about the
pinned focus are unlike the others and both are handled where the timeline is
built:

* **It cannot be paginated.** The SDK answers `PaginationError(NotSupported)`.
  So `has_reached_start` and `has_reached_end` are both set to `true` as soon as
  it is built, and `clear()` does not reset the start for it.
* **It is not live.** It gets no new events from sync, so it must not move a
  read receipt, must not be preloaded, and must not show who is typing. Those
  three were guarded by `!is_focused()`, which was the same thing as "is live"
  until this landed; they now ask `is_live()`, which is neither focused nor
  pinned.

Date dividers still arrive in it, and say nothing where every row carries its
own date — a `GtkFilterListModel` keeps only the `Event`s.

## A header toggle, not a banner

Sketched as a fifth `Adw.Banner` over the timeline and built as a
`GtkToggleButton` in the header bar instead. Three reasons:

* Four banners already stack above the timeline. A fifth that is shown in every
  room that has ever pinned anything, forever, is not a banner any more.
* A banner's button is one-way. The pinned list takes over the whole stack, the
  same way the search results do, so it needs a way back — and a toggle is that
  way back, in the same place the button that opened it was.
* It sits next to the search toggle, which does exactly the same thing to the
  same stack. The interaction is already learned.

The cost is real and worth naming: a pin is something one person wants everyone
to see, and an icon in the header does not announce itself the way a banner
does. The button is hidden entirely when the count is zero, so its appearing is
the only announcement there is.

## Pin and Unpin are two actions, not one toggle

`event.pin` and `event.unpin` are separate entries in the context menu, each
`hidden-when: "action-missing"`, and only one of them is ever added to the
action group. A single toggling entry would have to relabel itself, and
`GMenuModel` items do not have dynamic labels.

This is also why `EventRow` listens for the room's `pinned-events-changed`: the
action group is rebuilt when the event, its permissions or its target user
change, and pinning is none of those. Without that handler the menu keeps
offering _Pin_ on a message that is already pinned.

## The state event was invisible, and so was the ACL one

`show_in_timeline()` is an allow-list of state events ending in a match on four
types. `m.room.pinned_events` is now a fifth.

Adding it turned up a pre-existing bug: `m.room.server_acl` was never in that
list either, so the sentence `update_with_other_state()` has written for an ACL
change since the ACL editor landed had no way of being drawn — the event was
dropped before it ever reached the row. `doc/server-acls.md` claims that line
exists. It does now. Both types went into the list in the same commit.

## What the sentence can and cannot say

The SDK reduces the previous and current pinned lists to
`RoomPinnedEventsChange` — `Added`, `Removed` or `Changed` — and that is all the
row says: "{user} pinned a message." Not _which_ message, and not how many.

Two reasons not to do better. The event IDs in the content are not the pinned
events' content, so naming the message means fetching it, from a row that may
be scrolled past in a second. And the SDK reports a reorder, and a no-op
re-send, as `Changed`, so any sentence more specific than "changed the pinned
messages" would sometimes be a lie.

## Files

| File | What |
| --- | --- |
| `src/session/room/mod.rs` | `pinned_event_ids`, `pinned-count`, `update_pinned_events()`, `is_pinned()`, `pin_event()`, `unpin_event()`, the `pinned-events-changed` signal |
| `src/session/room/permissions.rs` | `can-pin-events`, from `SendState(RoomPinnedEvents)` |
| `src/session/room/timeline/mod.rs` | `Timeline::new_pinned()`, the `pinned` flag, `is_live()`, `RoomPinnedEvents` and `RoomServerAcl` in `show_in_timeline()` |
| `src/session_view/room_history/pinned/` | The view and its row |
| `src/session_view/room_history/mod.blp`, `mod.rs` | The header toggle, the stack page, `is-showing-pinned`, `init_pinned()` |
| `src/session_view/room_history/event_actions/` | `event.pin`, `event.unpin`, `set_message_pinned()` |
| `src/session_view/room_history/event_row.rs` | `pinned_events_handler`, so the menu keeps up |
| `src/session_view/room_history/state/content.rs` | The `RoomPinnedEvents` arm |
| `src/ui-blueprint-resources.in`, `po/POTFILES.in` | The new files |

## Rebase guide

1. The `_ =>` arm in `update_with_other_state()` is where upstream adds its own
   state events. Take theirs and re-add the `RoomPinnedEvents` arm above it — a
   lost arm is silent, it just goes back to "An unsupported state event was
   received."
2. The state allow-list at the bottom of `show_in_timeline()` is the other half
   of that, and losing it is equally silent: the arm stays and never runs. Both
   `RoomPinnedEvents` and `RoomServerAcl` belong in it.
3. Clicking a pinned message goes through `RoomHistory::focus_on_event()`,
   which no longer always builds a focused timeline: when the live timeline
   already holds the message it stays live and the message is highlighted in
   place. A pinned message is often recent, so this is the common case here.
   `doc/search.md` carries the argument and the mechanism.
4. `is_live()` replaced three `!is_focused()` checks. If upstream adds a fourth
   thing that only the live timeline should do, it wants `is_live()`, not
   `!is_focused()`.
5. If upstream adds its own pinned-events support, this whole approach is
   superseded and the interesting question is which UI survives. Keep the
   notices-room exclusion in `update_pinned_events()` either way; that one is
   the spec's ask, not a preference.
6. `TimelineFocus::PinnedEvents` is a unit variant on this SDK pin. Older
   revisions carried `max_events_to_load` and `max_concurrent_requests`, and a
   `PinnedEventsRoom` trait that no longer exists — if the pin moves backwards,
   this is where it breaks.

## Not done

* **No preview of the pinned message in the timeline row.** See above.
* **No reordering.** The spec gives the list an order and the client writes it
  in the order the SDK hands back — appending on pin, removing in place on
  unpin. Nothing drags.
* **The pinned view does not paginate**, because the focus cannot. A room with
  more pinned events than the SDK loads in one go shows what it loaded.

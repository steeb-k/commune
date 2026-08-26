# Threads — downstream implementation notes

Threading is a stable module of the Client-Server API (`m.thread` relations,
the `/threads` endpoint, per-thread receipts), not a proposal. This ledger
covers what this fork draws of it. The build order and the reasoning are in
`doc/gap-closing-plan.md`, round 4: three slices, and only the third flips the
row in `doc/spec-gaps.html`.

## Scope

**Slices 1 and 2 are built: a thread announces itself, and can be read and
written.** A message that is the root of a thread carries a chip under its
content — a thread icon and "N replies" — in every timeline the ordinary
message row draws. The chip opens the thread: the room history swaps to a
timeline holding the thread and nothing else, a banner over it names the state
and offers the way back, the composer at the bottom sends into the thread,
read receipts sent while reading it are the thread's own, and the thread keeps
a draft of its own. With somewhere to read them, threaded replies no longer
land inline in the main timeline: `hide_threaded_events` is on.

Not built yet: the thread list (slice 3), which is what flips the row in the
ledgers — a thread can be found only through its root today.

## The summary is the server's, not ours

`ThreadSummary` (reply count, latest event, the user's own per-thread
receipts) arrives on `MsgLikeContent::thread_summary` without any request from
us. The SDK extracts it from the bundled `m.thread` aggregation the server
attaches under `unsigned.m.relations` — `extract_bundled_thread_summary` runs
in `TimelineEvent::new` (`matrix-sdk-common/src/deserialized_responses.rs:619`)
on every event constructed from raw, so it is present even when none of the
thread's replies has ever been loaded. The event cache, which the session
already subscribes to sync (`src/session/mod.rs:500-504`), recomputes the
summary as replies arrive and republishes the root item; that reaches the row
through `Event`'s unconditional `item-changed` emission
(`src/session/room/timeline/event/mod.rs:234`), which `MessageRow` already
listens to for related-event changes. The chip therefore updates live with no
plumbing of its own.

`Event::thread_summary()` is the only new accessor, beside `reply_to_id`,
reading `msg_like.thread_summary` the same way that one reads
`msg_like.in_reply_to`.

## The chip is the way in

The chip was a passive `Gtk.Box` for the day slice 1 stood alone — a button
that does nothing reads as broken — and became the way in when slice 2 landed:
a flat `Gtk.Button` whose action is `room-history.show-thread` with the root's
event ID as the target, set in `update_thread_chip`. The same action backs the
_View Thread_ context-menu entry, which appears on any event that names a
thread root or carries a thread summary.

Two visibility rules, both in `MessageRow::update_thread_chip`:

* **Zero replies means no chip.** `num_replies` can be zero when every reply
  in the thread has been redacted; announcing a thread with nothing to read is
  worse than announcing none.
* **Compact rows hide it**, under exactly the condition that hides the
  reaction list (`ContentFormat::Compact | Ellipsized`) — those formats are
  previews, and a preview does not need a reply count.

The row's grid grew a row: the chip sits at row 3 between the reactions and
the read receipts, which moved to row 4, with the avatar's `row-span` grown to
match. The count string is `ngettext_f("1 reply", "{n} replies", …)`, which
put `message_row/mod.rs` into `po/POTFILES.in`.

## The thread view is the room history, not a sibling of it

The room history already knows how to display a timeline that is not the live
one — that is what a focused timeline is, swapped in by `focus_on_event()` and
swapped back out by `return_to_live()`. The thread view rides exactly that
mechanism rather than building a second stack of scrolling, pagination, sticky
and row machinery: `show_thread()` constructs `Timeline::new_threaded(room,
root)` — `TimelineFocus::Thread` underneath — and sets it as the displayed
timeline. Every row is the real `EventRow`, so context menus, reactions,
editing and read receipts all work in a thread without a line of new row code.

What tells the person where they are is an `Adw.Banner` — _Viewing a thread_,
with _Back to All Messages_ — revealed by binding to the timeline's
`is-thread` property. `return_to_live()` covers threads now, so the banner's
button, and anything else that returns to live, drops back to the room.

A thread timeline is deliberately **not** "live" in the app's sense
(`Timeline::is_live()` excludes it), but it is not focused either:

* Its bottom **is** the present — new thread events arrive from sync — so the
  history sticks to the bottom and scrolls down on entry, unlike a focused
  timeline.
* It paginates backwards through the SDK's thread pagination and never
  forwards; `has_reached_end` is true from birth.
* It does not show the room-wide typing row, is not preloaded, and does not
  drive the room's unread bookkeeping — those belong to the room's own live
  timeline.

## Composing, receipts and drafts are the thread's own

* **Sending needs no new send code.** The composer at the bottom of the room
  history is pointed at the thread timeline while one is shown — the
  `$compose_timeline` closure in `mod.blp` picks the displayed timeline when
  it is a thread and the room's live timeline otherwise. `Timeline::send` on a
  thread focus attaches the `m.thread` relation itself, and the thread
  timeline receives its own local echoes, which the live timeline never did
  for focused views.
* **Read receipts are scoped by the timeline they travel through.**
  `Room::send_receipt` became a thin wrapper over the new
  `Timeline::send_receipt`, and the room history sends `Read` receipts through
  the _displayed_ timeline — the SDK's `infer_thread_for_read_receipt` then
  stamps them with the thread. The `FullyRead` marker is _not_ sent from a
  thread view: it belongs to the room, and a thread event can sit far back in
  the room's own order, so moving the marker there would rewind it.
* **Drafts are per-thread, server-side.** The SDK's
  `save_composer_draft`/`load_composer_draft`/`clear_composer_draft` take an
  optional thread root, and `ComposerState` now carries one. The composer
  states map is keyed by `(room, thread root)`, so the room's half-typed
  message and each thread's are separate states — a draft typed for the room
  is never one thread-open away from being sent into the thread — and they
  survive restarts independently, compatible with other clients that do the
  same.

## `hide_threaded_events` is on, everywhere it exists

With somewhere to read a thread, threaded replies are hidden from the live
timeline (`TimelineFocus::Live { hide_threaded_events: true }`) and from the
non-threaded case of a focused timeline (`Automatic { hide_threaded_events:
true }`). The plan's ordering rule — never hide them before there is somewhere
else to read them — is satisfied in the same commit. A focused timeline whose
_target_ is a threaded event still shows the thread, which is what `Automatic`
means: a permalink or notification for a thread reply lands in the thread's
context rather than nowhere.

## The icon is ours

`thread-symbolic.svg` (a speech bubble with two lines knocked out,
`fill-rule="evenodd"`) is hand-drawn for this, since neither the app's icon
set nor the subset of the Adwaita theme we can rely on across the three
platforms carries a thread glyph. It lives with the other action icons and is
registered in `resources.gresource.xml`.

## Files

* `src/session/room/timeline/mod.rs` — `new_threaded()`, the `thread_root`
  state and `is-thread` property, the `TimelineFocus` branches with
  `hide_threaded_events`, and `send_receipt()` (moved here from `Room`, which
  now delegates).
* `src/session/room/timeline/event/mod.rs` — `thread_summary()` and
  `thread_root()` accessors.
* `src/session_view/room_history/mod.rs` + `mod.blp` — the
  `room-history.show-thread` action, `show_thread()`, the thread banner, the
  `$compose_timeline` closure, thread-aware `return_to_live()`, and the
  receipt changes.
* `src/session_view/room_history/message_row/mod.blp` + `mod.rs` — the chip
  (a button since slice 2), and the re-rowed grid.
* `src/session_view/room_history/event_actions/group.rs` +
  `context_menu.blp` — the _View Thread_ entry.
* `src/session_view/room_history/message_toolbar/mod.rs` +
  `composer_state.rs` — per-thread composer states and drafts.
* `data/resources/icons/scalable/actions/thread-symbolic.svg` +
  `resources.gresource.xml`.
* `data/resources/stylesheet/_room_history.scss` — `.thread-chip`.
* `testing/local-homeserver.sh` — `seed_thread()`: alice roots a thread in
  Invite Room, bob sends three `m.thread` replies, behind a `thread_root`
  marker of its own so an old seed picks it up on the next `up`.

## Rebase guide

* Upstream Fractal has no thread UI at all; every file above is either ours or
  carries a small addition. The grid rows in `message_row/mod.blp` are the
  likely conflict — upstream editing the read-receipts row will collide with
  its move to row 4.
* The `item-changed` emission being unconditional is what keeps the chip live.
  If upstream ever makes that emission conditional on content it knows about,
  the chip stops updating and nothing else visibly breaks — check it after
  any rebase that touches `Event::update_item`.
* `ThreadSummary` is re-exported from `matrix_sdk_ui::timeline`; an SDK bump
  that moves or renames it fails the build loudly, which is fine.
* The composer states map key grew from `Option<OwnedRoomId>` to
  `Option<(OwnedRoomId, Option<OwnedEventId>)>`. Upstream changes to
  `MessageToolbar::composer_state` will conflict there; the thread half of the
  key is the part to preserve.
* `Room::send_receipt`'s body lives in `Timeline::send_receipt` now. An
  upstream change to how receipts are sent lands there, and the room history's
  `update_read_receipts` must keep sending through the _displayed_ timeline,
  not the room.

## Not done

* **Slice 3** — the thread list, wrapping `ThreadListService`. Until it
  exists, a thread can only be entered through its root — there is no page
  listing the threads of a room.
* Thread subscriptions (MSC4306) are out — off spec.
* The latest-event preview and per-thread unread state that `ThreadSummary`
  carries are not drawn; the chip is count-only.
* The room-wide typing row is not shown inside a thread view, and typing
  notifications sent while composing into a thread are the room's ordinary
  ones — the protocol has no per-thread typing.
* A notification for a thread reply opens a focused timeline filtered to the
  thread (the SDK's `Automatic` mode), not the thread view proper — close
  enough to read, and the chip on the root is the way into the real one.

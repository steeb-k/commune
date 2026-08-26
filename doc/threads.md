# Threads — downstream implementation notes

Threading is a stable module of the Client-Server API (`m.thread` relations,
the `/threads` endpoint, per-thread receipts), not a proposal. This ledger
covers what this fork draws of it. The build order and the reasoning are in
`doc/gap-closing-plan.md`, round 4: three slices, and only the third flips the
row in `doc/spec-gaps.html`.

## Scope

**Slice 1 is built: a thread announces itself.** A message that is the root of
a thread carries a chip under its content — a thread icon and "N replies" —
in every timeline the ordinary message row draws (live, focused, pinned,
search results).

Not built yet: opening a thread, writing into one from a thread view, the
thread list, and `hide_threaded_events`. Threaded replies still land inline in
the main timeline as ordinary messages. Sending is already correct without any
of this — the SDK's `send_reply` forwards an existing thread relation — so
nothing goes out malformed today; the gap is presentational, and slice 1
closes only the "you cannot even see that a thread exists" half of it.

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

## A chip, not a button

The chip is a passive `Gtk.Box`, deliberately. Slice 2 builds the thread view;
until it exists there is nothing a click could do, and a button that does
nothing reads as broken. When the view lands, the chip becomes the way in —
expect `mod.blp` to swap the box for a button and `mod.rs` to gain a callback,
and nothing else about it to change.

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

## The icon is ours

`thread-symbolic.svg` (a speech bubble with two lines knocked out,
`fill-rule="evenodd"`) is hand-drawn for this, since neither the app's icon
set nor the subset of the Adwaita theme we can rely on across the three
platforms carries a thread glyph. It lives with the other action icons and is
registered in `resources.gresource.xml`.

## Files

* `src/session/room/timeline/event/mod.rs` — `thread_summary()` accessor.
* `src/session_view/room_history/message_row/mod.blp` — the chip, and the
  re-rowed grid.
* `src/session_view/room_history/message_row/mod.rs` — `update_thread_chip`,
  wired to `set_event`, `item-changed` and the content format.
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

## Not done

* **Slice 2** — a thread view (`TimelineFocus::Thread`), composer routing,
  per-thread receipts, and only then `hide_threaded_events: true` on the live
  timeline.
* **Slice 3** — the thread list, wrapping `ThreadListService`.
* Thread subscriptions (MSC4306) are out — off spec.
* The latest-event preview and per-thread unread state that `ThreadSummary`
  also carries are not drawn; the chip is count-only until there is a thread
  view for them to point into.

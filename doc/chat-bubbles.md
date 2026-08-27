# Chat bubbles — downstream implementation notes

An optional presentation of the timeline: each message drawn inside a
bubble, your own on the right, the way Element's "bubbles" layout does it.
Asked for by name on 26 August 2026, recorded in `doc/gap-closing-plan.md`
as its own piece outside the spec-gap board, and built the same day. The
spec has no opinion about any of this — it is pixels, not protocol.

## One switch, no third layout

_Chat Bubbles_ sits in the Appearance group of Account Settings ▸ General,
under Dark Mode: a `SwitchRow` bound to the `chat-bubbles-enabled` GSettings
key, off by default so the timeline looks like it always has. Element offers
three layouts (modern, bubbles, IRC); this fork offers two, because the flat
layout it already draws _is_ the modern one and nobody has asked for IRC.

## Where the work lives

The row already had everything a bubble needs — the grid, the header
states, the sender — so the feature is a presentation pass over
`MessageRow`, not a second row widget:

* `src/session_view/room_history/message_row/mod.rs` reads the setting once
  and watches it (`connect_changed`, disconnected in `dispose` — the
  settings object outlives every recycled row, so a leaked handler would
  accumulate). `update_bubbles()` toggles the classes and the alignments:
  `bubble` on the row and `bubble-surface` on the content in bubble mode,
  `bubble-own` when the sender `is_own_user()`; the content hugs its
  natural width (`halign` start, or end for own messages) instead of
  filling the line, and the reactions and thread chip follow to the same
  side. The header is the sender's whole cluster, corrected twice on the
  user's word (26 August 2026): an own bubble keeps its avatar — moved to
  the far grid column, its side of the line — **and its name**, so it is
  clear which account sent it, and the timestamp sits beside the name
  instead of in the far corner. The name is always the innermost piece,
  against the avatar; the timestamp always on the outside — "Alice 14:32"
  after the avatar on the left, "14:32 steeb" before it on the right, the
  children reordered in `update_bubbles()`. The flat view keeps its two
  corners untouched.
* `data/resources/stylesheet/_room_history.scss` draws the bubble:
  padding, a 12px radius, `currentColor` at 8% for everybody else so it
  reads in both themes, the accent at 25% for your own — the same borrowing
  the focused-event highlight already does.

Turning the switch moves every row that is alive, recycled rows included,
because each row watches the key itself; nothing needs the history to be
reopened. The thread view is the same rows, so it bubbles too. The pinned
and search lists use widgets of their own and are untouched.

## Decisions worth keeping

* **An own bubble keeps its name, timestamp and state.** The first pass
  hid the name and left the timestamp in the far corner; the user asked
  for the sender's data in one cluster, and for the exact order — time,
  name, avatar — before it was built, so this is settled, not up for
  re-derivation. The delivery checkmark (`MessageStateStack`) already sat
  at the line's end and reads as belonging to the bubble beside it.
* **A sticker gets a bubble too**, and so does an emoticon sent alone.
  Element strips the bubble from stickers; this fork does not yet — one
  rule for every content keeps the pass small, and the eyeball run will
  say whether a bubbled sticker looks wrong enough to earn the special
  case.
* **The 54px continuation margin is untouched.** Rows without an avatar
  keep the indent that aligns them under their group's first row; for an
  own bubble it only narrows the line a little from the far side, which is
  invisible in practice.

## Rebase guide

Everything here is additive. If upstream reshapes `MessageRow`'s grid, the
things to carry over are: the two classes, the halign trio in
`update_bubbles()`, and the own-bubble exception in `update_header()`. The
SCSS block is keyed on `message-row.bubble` and moves wherever
`_room_history.scss` goes.

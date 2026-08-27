# Room tags — downstream implementation notes

Round 9 of `doc/gap-closing-plan.md`, first item, built 26 August 2026:
the `order` field of the `m.tag` account data is read, so the manual
ordering another client gave the favorites is respected here instead of
silently discarded.

## What order means, and where it lands

The specification keeps a tag's `order` in `[0, 1]`, smaller first, and
asks that rooms carrying one come before rooms without. `Room` gains a
`tag-order` property: the order of the tag that put the room in its
section — favourite or low-priority, the only two that place a room in a
sorted section — and `NO_TAG_ORDER` (2.0, past the whole legal range) for
a room without one, refreshed on the same path that keeps the category
current. The sidebar's sections sort by it first and by latest activity
second (`gtk::MultiSorter` in `sidebar_data/section/mod.rs`); outside the
tagged sections every room reads as unordered, so activity keeps deciding
there, and within them the ordered rooms come first in the account's
order and the unordered ones follow by activity.

## What stays out, and why

* **Writing `order`.** Nothing here offers drag-to-reorder, and writing
  an order without a way to choose one would be noise. Moving a room
  between categories writes the tag the way it always did.
* **Arbitrary `u.*` tags.** A user-defined tag wants a sidebar section of
  its own, and the sidebar's sections are a fixed enum with hand-written
  offsets (`TOP_LEVEL_ITEMS_COUNT` and friends) — dynamic sections are a
  sidebar rework, not a tack-on, and the plan records the repricing
  rather than half-shipping it. The row on the spec-gaps page stays
  partial for exactly this.

## Rebase guide

The property and `update_tag_order` live in `session/room/mod.rs` beside
`update_category`, which calls it; the sorter change is one block in the
section model. If upstream reshapes the sidebar sorting, carry the
two-key sort: tag order ascending, then activity descending.

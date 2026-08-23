# Recently used emoji — downstream implementation notes

This file is the ledger for `m.recent_emoji`: what the fork added, the
decisions behind it, and what to check when rebasing onto a new Fractal
release. See `fork.md` for why none of this goes upstream.

## Scope

* Read and write the `m.recent_emoji` global account data event, so the list
  is the same list every other client on the account is using.
* Fill the seven quick reaction buttons in a message's context menu from it.

Not the full emoji picker, and not the composer. Both reasons are in "What
this cannot reach" below, and neither is a thing this fork can fix.

## The list is shared, so ruma owns the bookkeeping

`RecentEmojiEventContent` already carries the two operations that matter:

* `increment_emoji_total()` — bumps an emoji's count and moves it to the front,
  adding it if it is new, and truncating at the 100 the spec recommends.
* `recent_emoji_sorted_by_total()` — the same list ordered by count instead.

Neither is reimplemented here. The order the event stores is *most recently
used first*, and the totals ride along; the two orderings are two readings of
one list, which is why the sort is a separate call rather than a separate
field.

## Sorted by count, not by recency

The buttons are aimed at, not read. A grid ordered by recency reshuffles after
almost every reaction, which moves the button out from under the pointer of
someone who just learned where it was.

So the grid uses `recent_emoji_sorted_by_total()`. A top seven by count changes
rarely, and settles into whatever the account actually reacts with. Ties fall
back to the list order, which is recency — so of two emoji used as often, the
one used later is the one shown first.

## The defaults do not go away, they get pushed out

`DEFAULT_QUICK_REACTIONS` is the seven that used to be hard-coded in the
widget. They are now the tail of the list rather than the whole of it:
`quick_reactions_from()` takes the used emoji first and fills the rest from the
defaults, so

* a fresh account looks exactly like the old build,
* the first emoji someone reacts with displaces the last default, and
* the grid is always seven, never a short row.

**The two spellings problem.** 👍 and 👍️ differ by one invisible code point,
U+FE0F, and most clients send the first while our default is the second.
Without care the grid would show what looks like the same button twice.
`emoji_key()` strips the variation selectors for the purpose of deduplication
only — what gets _sent_ is always the spelling the account itself uses, never a
normalised one.

## Not every reaction key is an emoji

A reaction key is an arbitrary string, and clients do send words. `is_emoji()`
rejects anything containing an ASCII letter, which is enough: no emoji carries
one, while `lol` and an `mxc:` URI are nothing but letters. Digits are
deliberately left alone, because the keycap emoji are built out of them —
`1️⃣` is a digit, U+FE0F and U+20E3.

## Only a reaction being added is a use

`Room::toggle_reaction()` reads `has_own_user()` on the reaction group
_before_ it toggles, because afterwards there is no way to tell which
direction it went. Taking a reaction back is not a use of the emoji and must
not bump its count, or taking back a reaction sent by accident would promote it.

The recording lives in `Room::toggle_reaction()` rather than in the widget, so
every path into it counts — the quick buttons, the full emoji chooser, and
anything added later.

## What this cannot reach

* **The full emoji picker.** It is `GtkEmojiChooser`, which keeps its own
  recents in the `org.gtk.gtk4.Settings.EmojiChooser` GSettings schema. There
  is no API to seed it, so its "Recently Used" section stays per-device and
  per-desktop and has nothing to do with the account. What we do get is the
  pick itself, which arrives as a reaction and is recorded like any other.
* **The composer.** "Insert an Emoji" calls `GtkText`'s built-in chooser
  through `emit_insert_emoji()`, which inserts straight into the buffer. What
  was picked is not reported anywhere, and diffing the buffer to find out would
  be guessing.
* **Stickers.** The sticker picker keeps no history either, but a sticker is an
  `mxc:` URI and `m.recent_emoji` is a list of emoji. It needs a list of its
  own, which is not this.

## Files

Integration points, which are where a rebase will conflict:

| File | Change |
| --- | --- |
| `src/session/global_account_data.rs` | The list, the watcher, `quick_reactions_from()`, `is_emoji()`, `emoji_key()` and their tests |
| `src/session/room/mod.rs` | `toggle_reaction()` records the emoji when it adds one |
| `src/session_view/room_history/event_actions/quick_reaction_chooser.rs` | The grid is built from the list rather than from a constant |

## Rebase guide

1. The widget used to hold a `QUICK_REACTIONS` constant of
   `{key, column, row}`. That constant is gone; the geometry is
   `QUICK_REACTION_CELLS` and the emoji come from the session. If upstream
   touches the chooser, keep the split — a merge that restores the constant
   silently pins the grid again.
2. `build_buttons()` is called both from `constructed()` and when the account
   data changes, and it early-returns when the emoji have not moved. Removing
   that guard rebuilds seven buttons on every sync that touches the event.
3. The account data is bound the first time a `ReactionList` arrives, not in
   `constructed()`, because the chooser is one object reused for every message
   and has no session until then. The `if self.account_data.obj().is_none()`
   guard is what stops it binding again for the next message.
4. `is_emoji()` rejecting letters and accepting digits is deliberate and
   tested. Tightening it to a real emoji property check would drop the keycaps.
5. The event handler for `RecentEmojiEvent` uses the same drop-guard shape as
   `Permissions::init_power_levels()`. If the SDK reshapes
   `add_event_handler`, both move together.

## Not done

* Nothing shows the list, or lets it be cleared. It is only ever read seven at
  a time.
* Two clients reacting at once do a read-modify-write on the same account data,
  so one of the two increments is lost. Everyone else has this too; the cost is
  a count being low by one.
* The composer, the sticker picker and `GtkEmojiChooser`'s own ordering, for
  the reasons above.

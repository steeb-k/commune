# What still needs a pair of eyes

A running list of the things this fork has built that the compiler, clippy,
`cargo test` and `hooks/checks-bin` all pass on and none of them can judge.
`doc/calls.md` started the convention of naming what is _unwatched_; this file
collects it across features so it does not have to be hunted for.

**Add to it as work lands. Strike an entry only after it has actually been
looked at**, and say what was seen — "the banner appears" is worth more here
than "should appear".

Format: one section per feature, oldest first. `[ ]` not yet looked at, `[x]`
looked at and behaves, `[!]` looked at and wrong (with what happened).

---

## Calls — `doc/calls.md`

`doc/calls.md` carries the authoritative list at its top; these are the ones it
names as never having faced a second client.

* [ ] A renegotiation arriving **here** — needs a peer whose interface offers
      to add video mid-call, which Element for Android's does not.
* [ ] The rollback when two renegotiations cross. Takes two people pressing the
      same button in the same second.
* [ ] The badge for what the other end muted. The peer to hand sends no stream
      metadata at all.
* [ ] The answered and missed cases of the `m.call.invite` timeline row. Only
      the "no outcome" case has been seen.

## Server ACLs — `doc/server-acls.md`

* [ ] **The timeline line, which has never been drawn.** `show_in_timeline()`
      dropped `m.room.server_acl` before it could reach a row; that was fixed
      on 23 August 2026 alongside pinned messages. Change an ACL and check the
      sentence appears — "Alice blocked evil.example from taking part in this
      room" for a deny-list-only change, "changed which servers can take part
      in this room" otherwise.
      Needs `testing/local-homeserver.sh`; do not do this to a real room.

## Pinned messages — `doc/pinned-messages.md`

Nothing in this feature has been seen on screen yet. Every item below is new
drawing code.

* [x] **The header toggle appears** when a room has a pinned message, and only
      then. Seen 23 August 2026: pinning raised the icon, unpinning from the
      timeline took it away again.
* [x] **Pin from the context menu.** Seen 23 August 2026. Not yet seen: that
      the entry is _absent_ without the power level for `m.room.pinned_events`,
      which needs a room where we are not the moderator.
* [x] **The menu keeps up.** Seen 23 August 2026 — the same message was
      unpinned from its context menu straight after being pinned, so the entry
      had flipped. The `pinned-events-changed` handler in `EventRow` works.
* [x] **The pinned view lists them.** Seen 23 August 2026 and reported as
      looking right. The lazy build — on first opening, not on entering the
      room — was not separately checked and is not visible anyway.
* [x] **Clicking a row jumps to the message** in the timeline and closes the
      pinned view. Seen 23 August 2026.
* [x] **The unpin button on a row works and does not also jump.** Seen
      23 August 2026 — the button claims the click, the row does not activate
      behind it. The risk was real and did not land.
* [ ] **The unpin button's icon reads as "unpin".** It was
      `list-remove-symbolic` on first pass and read as a stray horizontal rule;
      changed to `close-symbolic`, which is what `removable_row.blp` and the
      explore server row already use for taking an item off a list. Not a
      wastebasket on purpose: the context menu's _Remove_ redacts the message,
      and a wastebasket here would read as that. Needs another look.
* [ ] **Unpinning the last message _while the pinned view is open_** leaves
      the toggle visible and shows the empty page, rather than hiding the only
      way back. Unpinning from the timeline with the view closed correctly
      hides the toggle, and that much was seen on 23 August 2026 — this is the
      other case.
* [ ] **The empty page** — `view-pin-symbolic`, "No Pinned Messages".
* [x] **The timeline sentence**: "{user} pinned a message." / "unpinned a
      message." Seen 23 August 2026. The third case, "changed the pinned
      messages", needs a reorder or a simultaneous pin-and-unpin and has not
      been seen.
* [ ] **The server notices room is unaffected**: its pinned events still raise
      the notice banner and must not appear in a pinned messages view. The
      toggle should never appear in that room.
      Needs `testing/local-homeserver.sh` — `./testing/local-homeserver.sh
      notice` then `limit on`.
* [ ] **Nothing regressed in the live timeline.** `is_live()` replaced three
      `!is_focused()` checks: read receipts still move, the typing row still
      appears, and the timeline still preloads.

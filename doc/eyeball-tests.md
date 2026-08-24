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

* [ ] **The header toggle appears** when a room has a pinned message, and only
      then. It is next to the search button and uses `view-pin-symbolic`.
* [ ] **Pin from the context menu.** Right-click a message → _Pin_. The entry
      is only there with the power level for `m.room.pinned_events`.
* [ ] **The menu keeps up.** Pin a message, then reopen its context menu: it
      must now say _Unpin_, not _Pin_. This is the `pinned-events-changed`
      handler in `EventRow`, and it is the most likely thing to be wrong.
* [ ] **The pinned view lists them** — avatar, sender, timestamp, body — and
      the list is built lazily, on first opening, not on entering the room.
* [ ] **Clicking a row jumps to the message** in the timeline and closes the
      pinned view.
* [ ] **The unpin button on a row works and does not also jump.** The row is
      `single-click-activate` and the button is a child of it; if GTK does not
      let the button claim the click, unpinning will also navigate away. This
      is a known risk, not a hypothetical.
* [ ] **Unpinning the last message** leaves the toggle visible and shows the
      empty page, rather than hiding the only way back.
* [ ] **The empty page** — `view-pin-symbolic`, "No Pinned Messages".
* [ ] **The timeline sentence**: "{user} pinned a message." / "unpinned a
      message." / "changed the pinned messages."
* [ ] **The server notices room is unaffected**: its pinned events still raise
      the notice banner and must not appear in a pinned messages view. The
      toggle should never appear in that room.
      Needs `testing/local-homeserver.sh` — `./testing/local-homeserver.sh
      notice` then `limit on`.
* [ ] **Nothing regressed in the live timeline.** `is_live()` replaced three
      `!is_focused()` checks: read receipts still move, the typing row still
      appears, and the timeline still preloads.

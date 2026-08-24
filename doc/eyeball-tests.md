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

## Presence — `doc/presence.md`

Nothing here has been seen on screen. The user's homeserver runs the Presence
module, so unlike most of the field this can be exercised on a real account.

* [x] **A badge appears** for people who are around, green for online. Seen
      23 August 2026 on a profile page. Amber for idle, and the absence of a
      badge for offline, are not yet seen.
* [ ] **The badge sits on the avatar's corner.** It did not on first pass: the
      overlay filled the space the avatar widget was given rather than hugging
      the avatar, so on a profile page the dot landed a few hundred pixels to
      the right. Fixed with `halign`/`valign` on the overlay — check the corner
      is right at size 24, 32 and 128, and check the fix did not move any
      avatar that was relying on being stretched.
* [ ] **A badge appears in the member list** specifically.
* [ ] **The badge scales.** It is sized from the avatar, a third of it clamped
      to 8–24px. Check it at size 32 in the member list and at size 128 on a
      profile page; the failure modes are a smudge on the big one and a dot
      covering the initials on the small one.
* [ ] **The badge has a ring** in the window colour, so it reads as sitting on
      the avatar rather than as part of the picture.
* [ ] **A direct chat's sidebar row carries the other person's badge**, and an
      ordinary room's does not. Added on 23 August 2026 after the first pass
      left it out; the room's avatar mirrors its `direct_member`.
* [ ] **The badge appears nowhere else** — not on inline mentions, read
      receipts, the typing row, the New Direct Chat picker, message rows or
      ordinary room avatars. It is opt-in and three sites opted in.
* [ ] **A profile page shows the status message** when the person set one, and
      shows nothing rather than an empty gap when they did not.
* [ ] **It changes live.** Go idle or online in another client on the same
      account, or ask somebody to, and watch the badge follow without a
      restart.
* [ ] **Somebody who has not moved still gets a badge.** This is the store
      read: sync only sends presence when it changes, so a person who was
      already online before Commune started would otherwise have none until
      they did something.
* [ ] **The switch is in Account Settings ▸ Privacy and starts on.** Turning it
      off should make you go offline for other clients within a moment — this
      is the half that was never optional before, so it is worth confirming
      from a second client rather than trusting it.
* [ ] **Turning it back on** makes you online again without restarting.

## Signing up and resetting a password — `doc/registration.md`

The sign-up half was run on 24 August 2026 and reported as going well end to
end. Struck below are the things that run says were seen; the edge cases and
the ones with no local harness are still open, and the reset half has not been
touched at all.

Use `testing/local-homeserver.sh` for all of it — it runs with open
registration, so Synapse asks for `m.login.dummy` and no stage needs input.
Registering on a public server leaves a junk account behind.

* [x] **The _Create Account_ button is there at all**, under _Log In_ on the
      greeter, and is a plain pill rather than a suggested one. Seen
      24 August 2026.
* [x] **It leads to the same homeserver page** as logging in, with the same
      domain entry and the same advanced switch. Seen 24 August 2026 — and the
      thing that page needs is a default, which is the next piece of work.
* [x] **The register page draws**: the title says "Create an account on
      localhost", the homeserver URL sits under it with the house icon, and
      there are three rows — username, password, confirm password.
      This page is `form-page`-styled but sits in the login flow, so the
      margins and the 24px spacing come from CSS rather than the template;
      that combination had never been rendered. Seen 24 August 2026.
* [x] **The strength meter fills** as the password gets better, in five
      discrete blocks, and turns green at full. The offsets are added from Rust
      here rather than from the template. Seen 24 August 2026.
* [x] **The username check says something.** Type a name that exists (`alice`
      after seeding) and the row should go amber with "This username is already
      taken" about half a second after the last keystroke; a free name should
      go green with no message. Seen 24 August 2026. The fast-typist case — an
      answer arriving for a prefix of what was typed — was not separately
      provoked.
* [x] **The button is insensitive** until the username has been answered for,
      the password is at full strength and the confirmation matches. Seen
      24 August 2026. Not seen: that it is _sensitive_ on a server which
      refuses to answer the availability question at all, which is the case the
      four-state machine exists for and which the local harness does answer.
* [x] **The account is actually created** and lands in the encryption setup
      pages, the same as a password login. The UIAA dialog should flash past
      without asking anything, because the only stage is `m.login.dummy`. Seen
      24 August 2026 — an account was made and the session came up.
* [ ] **A taken username refused at the last moment** — register `bob` in two
      windows at once, or take the name between the check and the button — says
      "This username is already taken" from the new error mapping.
* [ ] **Going back cleans the page.** Leave the register page, come back, and
      the three rows should be empty with no leftover green or amber, and the
      meter at zero.
* [x] **Registration switched off** says "This homeserver does not allow
      creating an account", not "Invalid credentials". Seen 24 August 2026.
* [x] **The registration token page.** `./testing/local-homeserver.sh signup
      token` prints a token; the dialog should show a plain entry with
      _Continue_ insensitive until something is typed. Type the wrong token
      first: the toast should say "The registration token is invalid" and the
      entry should come back rather than the dialog closing. Seen
      24 August 2026.
* [ ] **The terms page**, which has no harness at all — Synapse only asks for
      `m.login.terms` with a `user_consent` block and its template files. A
      check button per policy document, each row with an external-link button
      that opens it, and _Agree_ insensitive until every box is ticked. The
      most likely things to be wrong here are the row layout (prefix check
      button, suffix link) and which language of the document is picked.
* [ ] **The fallback page**, for a stage this client still does not draw — a
      captcha, or an emailed token: the homeserver's own web page should appear
      inside the dialog. `matrix.org` asks for a captcha, which is one run
      through it and no more.
* [ ] **The OAuth path**, which needs a server with the OAuth 2.0 API: the
      browser should open the server's _sign-up_ form rather than its sign-in
      form, and a server that does not advertise `prompt=create` should say so
      with a toast instead of opening a browser at all. Nothing in the harness
      speaks OAuth, so this needs `matrix.org` or another real server.
* [x] **Nothing regressed in logging in.** The greeter's _Log In_ button and
      the password path go through the same code with the purpose left at
      `LogIn`. Seen 24 August 2026. The SSO path was not exercised.

### Choosing a homeserver

The page both flows share, reworked on 24 August 2026 after the sign-up run:
`matrix.org` is offered first and the old entry is behind a second row.

* [ ] **The two rows draw** as a boxed list with radio buttons, matrix.org
      checked, and the entry hidden underneath.
* [ ] **Picking _Another Homeserver_ reveals the entry** and puts the cursor in
      it; picking matrix.org again hides it.
* [ ] **_Next_ is sensitive immediately**, with nothing typed, and pressing
      Return goes straight on — the button takes the focus when the page is
      shown with the default picked.
* [ ] **The _Advanced…_ button is hidden** while matrix.org is chosen and comes
      back with the entry. Auto-discovery is meaningless for the default.
* [ ] **The next page says "Log in to matrix.org"** (or "Create an account on
      matrix.org"), which is the `server_name()` that moved onto the page.
* [ ] **A custom homeserver still works both ways** — a domain name with
      auto-discovery on, and a URL with it off through _Advanced…_. This is the
      path that used to be the only one, so it is the regression to watch.
* [ ] **Going back to the greeter and returning** puts the choice back on
      matrix.org with the entry empty.
* [ ] **Logging in against the local harness still works**, which now means
      picking _Another Homeserver_ and typing `localhost:8008` — the flow every
      other test here starts with.

### Resetting a password

Nothing here has been seen, and there is **no local harness for any of it** —
Synapse sends no email without an SMTP server. What can be checked without one
is everything up to the point where the email would arrive.

* [ ] **The _Forgot Password?_ link** is on the password login page, under the
      password row, flat and centred. **It cannot be reached on matrix.org**,
      which delegates authentication and so never shows that page — use
      _Another Homeserver_ and the local harness. That is behaviour, not a
      fault: `account.matrix.org` carries its own reset.
* [x] **The page draws**: title "Reset your password on localhost", the
      homeserver URL under it, an explanation, one email row and a _Send Link_
      button that is insensitive until something is typed. Seen 24 August 2026 —
      it was reached and used, which is what produced the answer below.
* [x] **A homeserver that cannot send email says so.** Seen 24 August 2026:
      _"Email-based password resets have been disabled on this server"_ — which
      is Synapse's own sentence under `M_UNKNOWN`, arriving through the
      catch-all rather than through this fork's `M_THREEPID_DENIED` arm. Good
      enough that the mapping was left alone; `registration.md` records why.
* [ ] **An address on no account** says "No account on this homeserver uses that
      email address."
* [ ] **The stack moves on** to the password half only once the server has
      answered, and the explanation names the address the link went to.
* [ ] **The strength meter and the confirmation** behave as they do on the
      other two pages. Half seen on 24 August 2026: Account Settings ▸ Change
      Password works end to end, so the shared helpers did not break the page
      that existed before them. The reset page's own copy is still unseen.
* [ ] **Pressing _Reset Password_ before opening the link** says "Open the link
      in the email first, then try again" and leaves the page as it was. This is
      the 401-with-a-UIAA-body case, and it is the most likely thing in the
      whole flow to be wrong.
* [ ] **The whole thing, end to end**, on a homeserver that does send email:
      the new password works, and every other session is logged out.
* [ ] **Going back and returning** empties both halves and starts at the email
      step again.

## Spaces — `doc/spaces.md`

Round 3, slice 1: spaces stop being invisible. Nothing here browses what is
_inside_ a space — that is slice 2 — so every check below is about a space
being present, openable, leavable and findable.

### Setting up

Everything except the last group runs against `testing/local-homeserver.sh`,
which already seeds a public space called **Test Space** (`#test-space:localhost`,
created by alice). Note it has **no child rooms**: `Restricted Room` and
`Knock Restricted Room` name the space in their join rule, which is not the
same as being in it. So an empty space is the case under test here, and that is
the right case for slice 1.

```sh
testing/local-homeserver.sh up        # or `reset` for a clean slate
```

Log in as alice — the greeter now offers matrix.org first, so this means
_Another Homeserver_ → `localhost:8008`. For the invite check, log in as bob in
a second run.

### The sidebar

* [ ] **A "Spaces" section appears**, between _Server Notices_ and
      _Favorites_, holding Test Space. This much was seen on 24 August 2026 on
      the user's own account — the rest of this list was not.
* [ ] **It is collapsed on an existing session and expanded on a new one.**
      Expected, not a fault: the expanded sections are stored as a set of
      names, and a session saved before this change has no `space` in it.
      Check both: an account already logged in, and one logged in fresh after
      `rm -rf ~/.local/share/commune` (or a second account).
* [ ] **The expander remembers.** Collapse it, quit, start again: still
      collapsed. This is the `space` string reaching `SessionSettings`, and a
      typo there fails silently.
* [ ] **The section disappears when it is empty.** Leave the last space and
      the header should go with it, the way _Favorites_ does.
* [ ] **The row carries a grid icon** with a "Space" tooltip on hover.
* [ ] **The icon did not steal anyone else's.** A direct chat still shows the
      person icon, a call room the video icon, the server notices room the
      warning triangle. The space branch was put _ahead_ of all three, so this
      is the regression to watch.
* [ ] **A space with unread state does not shout.** A space receives no
      messages, so its row should carry no unread dot and the section header no
      count. If a count appears, the aggregation in `SidebarSection` is
      counting something that is not a message.
* [ ] **The sidebar room search finds it.** Ctrl+K, type "Test" — the space
      should be among the results and selecting it should open the space page,
      not an empty timeline.

### The space page

* [ ] **Selecting the space opens a page with its name**, not an empty room
      history. This is the visible bug slice 1 exists to fix, so if anything
      here is wrong, this is the thing to report.
* [ ] **The header bar** shows the space name as the title and the word
      "Space" underneath as a subtitle.
* [ ] **The header bar is the same height as every other page's.** Switch
      between a room, Explore and the space with the sidebar visible. It was
      added to the size group by hand and the array's length is a literal.
* [ ] **The body**: a large avatar, the name in large type, the canonical
      alias `#test-space:localhost` under it, and the sentence saying the rooms
      inside cannot be listed yet.
* [ ] **A space with no topic hides the topic label** rather than leaving a
      gap — and a space _with_ one shows it. Set one from another client, or
      check against a space on matrix.org.
* [ ] **A topic containing a matrix.to link is clickable** and opens that room
      or user inside the app rather than a browser. Same handler as the invite
      page; it is wired separately here.
* [ ] **No dead controls.** There is no _Room Details_, no composer, no member
      list. If any of the room-history header bar buttons appear on this page,
      the wrong page is being shown.

### Leaving one, and not re-filing one

* [ ] **Right-click the space row: the menu offers _Leave Room_ and
      _Report Room_, and nothing else.** No _Favorite_, no _Low Priority_, no
      _Set as Direct Chat_, no _Mark as Unread_. Those are tags and a space
      takes none of them.
* [ ] **_Leave Room_ asks first**, then the row leaves the Spaces section and
      turns up under _Historical_. Then _Forget_ from there should work as it
      does for a room.
* [ ] **Re-joining it** — from Explore, or the alias — puts it back in the
      Spaces section rather than in _Rooms_.
* [ ] **Dragging the space row** highlights only _Historical_ as a valid drop
      target; every other section should go grey. Dropping it there leaves the
      space, the same as the menu item.
* [ ] **Dragging an ordinary room over the Spaces section does nothing.** The
      section must show as disabled and refuse the drop. Rooms are put into
      spaces with `m.space.child`, which does not exist here yet, and a drop
      that silently did nothing would be worse than one that refuses.

### Finding one in Explore

* [ ] **Explore lists spaces at all.** Search for "Test" on `localhost` — the
      space should be in the results next to the ordinary rooms. Before this
      change the directory was asked to exclude them.
* [ ] **A space row says "Space"** — a dimmed grid icon and the word, beside
      the member count. An ordinary room row must **not** show it; that is the
      half of this check that catches a property left always-true.
* [ ] **Joining from Explore** works and the room lands in the Spaces section,
      not in _Rooms_. The button should read _Join_, and _View_ once joined.
* [ ] **On matrix.org**, where the directory is large: search for a known
      space (`#space:matrix.org` and similar) and confirm the marker appears
      there too. The local harness has one space and one shape of summary; a
      real directory is where a missing `room_type` shows up.

### Invites, and what slice 1 deliberately does not change

* [ ] **An invite to a space still goes to _Invited_** and opens the ordinary
      invite page. As bob, have alice invite you to Test Space. Accepting it
      should drop the space into the Spaces section on the next sync; declining
      should behave like declining a room. The invite page says nothing about
      it being a space, which is known and recorded in `spaces.md`.
* [ ] **Nothing regressed for ordinary rooms.** Favorites, Low Priority,
      Historical and the drag-and-drop between them all still work; the
      _Forget_ target is still at the bottom of the sidebar. The section index
      map was rewritten by hand and it is exactly the kind of change that
      moves a section's contents into its neighbour.

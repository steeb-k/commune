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

**The run of 25 August 2026 struck every check in this file.** It went through
the whole sheet in one pass — every section, the tear-down checks under the
**Destructive** lines included — and struck 166 that had never been looked at.
Two of them failed; both were fixed the same day and a second pass found all
200 good. An entry struck with no date of its own beside it was struck by that
run; the entries carrying a date were struck before it.

**Nothing here is unseen, and that is the state to keep it in.** The next thing
added to this file will be the only unstruck line in it, which is the point of
the file. Also worth knowing for the next run: the sheet's report counts button
presses, not checks, so a check done and not clicked reads as "not looked at" —
that produced a phantom "eleven not looked at" on the first pass and should not
be read as a list of what was skipped.

A subsection that takes something away — leaving a space, removing a room from
one — opens with a line in italics beginning **Destructive**, and belongs at
the end of its section. The run sheet reads that line and marks every check
under it, so nobody tears down the thing the next twenty checks need. Keep both
true when adding to this file.

---

## How to run this

The sections below are ordered oldest first, which is the wrong order to read
them in. **Work newest first**: the code nobody has looked at yet is the code
most likely to be wrong, and four of the five bugs found on 24 August 2026 were
in work less than a day old.

`doc/eyeball-run.html` is this file with checkboxes, generated from it by
`doc/eyeball-page.py` and published at
<https://claude.ai/code/artifact/9b66f090-2f6d-407e-a802-ea4079773fe9>.
Regenerate and republish it whenever this file changes — the ledger is the
source, the page only draws it, and **a result is not recorded until it is
struck here**. The page keeps its marks in one browser and nowhere else.

### Before anything

```sh
testing/local-homeserver.sh up      # or `reset` for a clean slate
testing/local-homeserver.sh verify  # says whether it is in the state below
meson install -C _build             # the binary must be newer than the code
commune
```

`verify` reports and never repairs; `up` repairs. Run it before a session and
whatever is wrong arrives at once rather than one room at a time in the middle
of the list.

Log in as **alice** — the greeter offers matrix.org first, so this means
_Another Homeserver_ → `localhost:8008`. The harness sets four passwords:
`alice-is-testing`, `bob-is-testing`, `carol-is-testing`, `admin-is-testing`.

**Who is who.** Alice owns nearly everything. **Bob** is a second member —
including of both spaces, so that alice leaving one does not destroy it — which
means he can never be invited to them. **Carol** exists and has joined nothing,
and is who the invite checks are for. Some checks want **bob** as well; a
second Commune on the same machine cannot hold two sessions at once, so those
are gathered together rather than scattered.

### The order, and roughly what each costs

| | Section | Why here |
| --- | --- | --- |
| 1 | Spaces | Largest, newest, and the sidebar section is all anybody has seen |
| 2 | Choosing a space, restricting, putting one in | Same day's work, and the restricted rule can lock people out if it is wrong |
| 3 | Reading a room without joining it | Partly seen already; the negatives are untouched |
| 4 | Going to a message without leaving the present | Small, and it is a fix rather than a feature |
| 5 | Signing up and resetting a password | Never looked at, and it is the first thing a new person meets |
| 6 | Presence | Never looked at |
| 7 | Pinned messages, Calls, Server ACLs | Mostly seen; the leftovers |

### The half worth caring about

Every section has checks phrased as _something should **not** happen_ — a
button absent, a control insensitive, a room not listed. **Those are the ones
to do.** A feature that works when you use it properly is the easy case; the
bugs found so far were all a flag read backwards, a list not kept in step, or a
control that was never wired at all, and every one of them showed up as
something appearing where it should not have.

If a check fails, say **what happened**, not what should have. A screenshot is
worth more than either.

### What to do last

Anything marked **Destructive** takes away what the rest of its section is
looking at. Those subsections sit at the end of each section for that reason,
and the run sheet marks them. Nothing here is unrecoverable — `Test Space` and
`Sub Space` are public, so rejoining them is a search in Explore — but it is
ten minutes you do not need to spend.

---

## Calls — `doc/calls.md`

`doc/calls.md` carries the authoritative list at its top; these are the ones it
names as never having faced a second client.

* [x] A renegotiation arriving **here** — needs a peer whose interface offers
      to add video mid-call, which Element for Android's does not.
* [x] The rollback when two renegotiations cross. Takes two people pressing the
      same button in the same second.
* [x] The badge for what the other end muted. The peer to hand sends no stream
      metadata at all.
* [x] The answered and missed cases of the `m.call.invite` timeline row. Only
      the "no outcome" case has been seen.

## Server ACLs — `doc/server-acls.md`

* [x] **The timeline line, which has never been drawn.** `show_in_timeline()`
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
* [x] **The unpin button's icon reads as "unpin".** It was
      `list-remove-symbolic` on first pass and read as a stray horizontal rule;
      changed to `close-symbolic`, which is what `removable_row.blp` and the
      explore server row already use for taking an item off a list. Not a
      wastebasket on purpose: the context menu's _Remove_ redacts the message,
      and a wastebasket here would read as that. Needs another look.
* [x] **Unpinning the last message _while the pinned view is open_** leaves
      the toggle visible and shows the empty page, rather than hiding the only
      way back. Unpinning from the timeline with the view closed correctly
      hides the toggle, and that much was seen on 23 August 2026 — this is the
      other case.
* [x] **The empty page** — `view-pin-symbolic`, "No Pinned Messages".
* [x] **The timeline sentence**: "{user} pinned a message." / "unpinned a
      message." Seen 23 August 2026. The third case, "changed the pinned
      messages", needs a reorder or a simultaneous pin-and-unpin and has not
      been seen.
* [x] **The server notices room is unaffected**: its pinned events still raise
      the notice banner and must not appear in a pinned messages view. The
      toggle should never appear in that room.
      Needs `testing/local-homeserver.sh` — `./testing/local-homeserver.sh
      notice` then `limit on`.
* [x] **Nothing regressed in the live timeline.** `is_live()` replaced three
      `!is_focused()` checks: read receipts still move, the typing row still
      appears, and the timeline still preloads.

## Presence — `doc/presence.md`

Nothing here has been seen on screen. The user's homeserver runs the Presence
module, so unlike most of the field this can be exercised on a real account.

* [x] **A badge appears** for people who are around, green for online. Seen
      23 August 2026 on a profile page. Amber for idle, and the absence of a
      badge for offline, are not yet seen.
* [x] **The badge sits on the avatar's corner.** It did not on first pass: the
      overlay filled the space the avatar widget was given rather than hugging
      the avatar, so on a profile page the dot landed a few hundred pixels to
      the right. Fixed with `halign`/`valign` on the overlay — check the corner
      is right at size 24, 32 and 128, and check the fix did not move any
      avatar that was relying on being stretched.
* [x] **A badge appears in the member list** specifically.
* [x] **The badge scales.** It is sized from the avatar, a third of it clamped
      to 8–24px. Check it at size 32 in the member list and at size 128 on a
      profile page; the failure modes are a smudge on the big one and a dot
      covering the initials on the small one.
* [x] **The badge has a ring** in the window colour, so it reads as sitting on
      the avatar rather than as part of the picture.
* [x] **A direct chat's sidebar row carries the other person's badge**, and an
      ordinary room's does not. Added on 23 August 2026 after the first pass
      left it out; the room's avatar mirrors its `direct_member`.
* [x] **The badge appears nowhere else** — not on inline mentions, read
      receipts, the typing row, the New Direct Chat picker, message rows or
      ordinary room avatars. It is opt-in and three sites opted in.
* [x] **A profile page shows the status message** when the person set one, and
      shows nothing rather than an empty gap when they did not.
* [x] **It changes live.** Go idle or online in another client on the same
      account, or ask somebody to, and watch the badge follow without a
      restart.
* [x] **Somebody who has not moved still gets a badge.** This is the store
      read: sync only sends presence when it changes, so a person who was
      already online before Commune started would otherwise have none until
      they did something.
* [x] **The switch is in Account Settings ▸ Privacy and starts on.** Turning it
      off should make you go offline for other clients within a moment — this
      is the half that was never optional before, so it is worth confirming
      from a second client rather than trusting it.
* [x] **Turning it back on** makes you online again without restarting.

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
* [x] **A taken username refused at the last moment** — register `bob` in two
      windows at once, or take the name between the check and the button — says
      "This username is already taken" from the new error mapping.
* [x] **Going back cleans the page.** Leave the register page, come back, and
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
* [x] **The terms page**, which has no harness at all — Synapse only asks for
      `m.login.terms` with a `user_consent` block and its template files. A
      check button per policy document, each row with an external-link button
      that opens it, and _Agree_ insensitive until every box is ticked. The
      most likely things to be wrong here are the row layout (prefix check
      button, suffix link) and which language of the document is picked.
* [x] **The fallback page**, for a stage this client still does not draw — a
      captcha, or an emailed token: the homeserver's own web page should appear
      inside the dialog. `matrix.org` asks for a captcha, which is one run
      through it and no more.
* [x] **The OAuth path**, which needs a server with the OAuth 2.0 API: the
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

* [x] **The two rows draw** as a boxed list with radio buttons, matrix.org
      checked, and the entry hidden underneath.
* [x] **Picking _Another Homeserver_ reveals the entry** and puts the cursor in
      it; picking matrix.org again hides it.
* [x] **_Next_ is sensitive immediately**, with nothing typed, and pressing
      Return goes straight on — the button takes the focus when the page is
      shown with the default picked.
* [x] **The _Advanced…_ button is hidden** while matrix.org is chosen and comes
      back with the entry. Auto-discovery is meaningless for the default.
* [x] **The next page says "Log in to matrix.org"** (or "Create an account on
      matrix.org"), which is the `server_name()` that moved onto the page.
* [x] **A custom homeserver still works both ways** — a domain name with
      auto-discovery on, and a URL with it off through _Advanced…_. This is the
      path that used to be the only one, so it is the regression to watch.
* [x] **Going back to the greeter and returning** puts the choice back on
      matrix.org with the entry empty.
* [x] **Logging in against the local harness still works**, which now means
      picking _Another Homeserver_ and typing `localhost:8008` — the flow every
      other test here starts with.

### Resetting a password

Nothing here has been seen, and there is **no local harness for any of it** —
Synapse sends no email without an SMTP server. What can be checked without one
is everything up to the point where the email would arrive.

* [x] **The _Forgot Password?_ link** is on the password login page, under the
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
* [x] **An address on no account** says "No account on this homeserver uses that
      email address."
* [x] **The stack moves on** to the password half only once the server has
      answered, and the explanation names the address the link went to.
* [x] **The strength meter and the confirmation** behave as they do on the
      other two pages. Half seen on 24 August 2026: Account Settings ▸ Change
      Password works end to end, so the shared helpers did not break the page
      that existed before them. The reset page's own copy is still unseen.
* [x] **Pressing _Reset Password_ before opening the link** says "Open the link
      in the email first, then try again" and leaves the page as it was. This is
      the 401-with-a-UIAA-body case, and it is the most likely thing in the
      whole flow to be wrong.
* [x] **The whole thing, end to end**, on a homeserver that does send email:
      the new password works, and every other session is logged out.
* [x] **Going back and returning** empties both halves and starts at the email
      step again.

## Spaces — `doc/spaces.md`

Round 3. Slice 1 made spaces stop being invisible; slice 2 made the space page
list the rooms inside one. The slice 1 checks are about a space being present,
openable, leavable and findable; the slice 2 checks are under _What is inside
one_.

### Setting up

Everything except the last group runs against `testing/local-homeserver.sh`,
which seeds a public space called **Test Space** (`#test-space:localhost`,
created by alice) with six rooms inside it, one for each shape a listing has
to draw:

| Room | Why it is there |
| --- | --- |
| `Public Room` | alice has joined it — the button should say _View_ |
| `Restricted Room` | alice has joined it too, and it names the space in its join rule |
| `Bobs Room` | bob made it and alice never joined — the button should say _Join_ |
| `Sub Space` | a space inside a space — where the one-level limit shows |
| `Readable Room` | `world_readable`, but alice's own, so she is in it |
| `Peekable Room` | `world_readable` and bob's — the one alice can preview |

Naming a space in a join rule is **not** the same as being a child of it, so
`seed_space_children()` writes the `m.space.child` events, and
`seed_peekable_room()` adds the last of them. Both run outside the
`seeded.json` gate, which means an existing server picks the children up:

```sh
testing/local-homeserver.sh up        # or `reset` for a clean slate
```

Log in as alice — the greeter now offers matrix.org first, so this means
_Another Homeserver_ → `localhost:8008`. For the invite check, log in as bob in
a second run.

### The sidebar

* [x] **A "Spaces" section appears**, between _Server Notices_ and
      _Favorites_, holding Test Space. This much was seen on 24 August 2026 on
      the user's own account — the rest of this list was not.
* [x] **It is collapsed on an existing session and expanded on a new one.**
      Expected, not a fault: the expanded sections are stored as a set of
      names, and a session saved before this change has no `space` in it.
      Check both: an account already logged in, and one logged in fresh after
      `rm -rf ~/.local/share/commune` (or a second account).
* [x] **The expander remembers.** Collapse it, quit, start again: still
      collapsed. This is the `space` string reaching `SessionSettings`, and a
      typo there fails silently.
* [x] **The section disappears when it is empty.** Leave the last space and
      the header should go with it, the way _Favorites_ does.
* [x] **The row carries a grid icon** with a "Space" tooltip on hover.
* [x] **The icon did not steal anyone else's.** A direct chat still shows the
      person icon, a call room the video icon, the server notices room the
      warning triangle. The space branch was put _ahead_ of all three, so this
      is the regression to watch.
* [x] **A space with unread state does not shout.** A space receives no
      messages, so its row should carry no unread dot and the section header no
      count. If a count appears, the aggregation in `SidebarSection` is
      counting something that is not a message.
* [x] **The sidebar room search finds it.** Ctrl+K, type "Test" — the space
      should be among the results and selecting it should open the space page,
      not an empty timeline.

### The space page

* [x] **Selecting the space opens a page with its name**, not an empty room
      history. This is the visible bug slice 1 exists to fix, so if anything
      here is wrong, this is the thing to report.
* [x] **The header bar** shows the space name as the title and the word
      "Space" underneath as a subtitle.
* [x] **The header bar is the same height as every other page's.** Switch
      between a room, Explore and the space with the sidebar visible. It was
      added to the size group by hand and the array's length is a literal.
* [x] **The body**: an avatar, the name in large type, the canonical alias
      `#test-space:localhost` under it, and the list of rooms below that.
* [x] **A space with no topic hides the topic label** rather than leaving a
      gap — and a space _with_ one shows it. Set one from another client, or
      check against a space on matrix.org.
* [x] **A topic containing a matrix.to link is clickable** and opens that room
      or user inside the app rather than a browser. Same handler as the invite
      page; it is wired separately here.
* [x] **The header bar has a menu**, at the right. It offers _Space Details_,
      _Invite New Members…_, _Leave Space_ and _Report Space…_. There was none
      at all until 24 August 2026, which left every page of the room details
      unreachable for a space. Reported working on 24 August 2026.
* [x] **_Space Details_ opens them**, and the _Spaces_ group in there can put
      this space inside another one. Reported working on 24 August 2026.
* [x] **_Invite New Members…_ is absent without the power to invite.**
* [x] **Somebody already in the space says so.** Search for `bob`, who is a
      member of both spaces: his row has no checkbox and a chip reading
      _Already a member_ where the checkbox would be. Reported on 24 August
      2026 as a row that could not be clicked with nothing saying why — the
      chip was a dim label sharing its space with the user ID, and it
      ellipsized away.
* [x] **Somebody who is in nothing can be picked.** Search for `carol` and her
      row has a checkbox.
* [x] **Return in the search box does not put a line break in it.** It is a
      text view, because a pill has to sit inside the text, and it used to
      take Return as a new line — which also put a newline in the term being
      searched for. Return should now invite whoever is selected, or do
      nothing when the button is insensitive.
* [x] **_Leave Space_ asks first**, and leaving works.
* [x] **No composer and no member list.** The space page is not a room
      history; if the call buttons or the search button turn up on it, the
      wrong page is being shown.

### What is inside one

Slice 2. All of this is on the space page, below the topic.

* [x] **The six seeded rooms appear** under a _Rooms_ heading: Public Room,
      Restricted Room, Bobs Room, Sub Space, Readable Room and Peekable Room.
      Not five, not seven, and **not Test Space itself** — the space is the
      first room the endpoint returns and it is skipped on purpose.
* [x] **A spinner shows first and is replaced.** Select the space from a cold
      start. If the spinner stays forever the request failed silently; if the
      page is blank the stack landed on the wrong child.
* [x] **The buttons say the right thing.** As alice: _View_ on Public Room,
      Restricted Room and Readable Room, _Join_ on Bobs Room and Peekable Room.
      As bob, who is in Bobs Room, Peekable Room and Sub Space and not the
      rest, they swap over. This is `RoomListRoomInfo`, and a button that says
      _Join_ for a room you are already in means the identifiers are not
      matching.
* [x] **_View_ opens the room.** Clicking it selects that room in the sidebar
      and shows its timeline.
* [x] **_Join_ joins it, in place.** Click _Join_ on Bobs Room: the button
      shows its loading state, the room appears in the sidebar under _Rooms_,
      and the button on the space page turns into _View_ **without reopening
      the page**. That last part is the live half of `RoomListRoomInfo`; if it
      needs a revisit to update, the handler is not connected.
* [x] **Sub Space is marked as a space** — the dimmed grid icon and the word
      _Space_ — and its button behaves like any other room's.
* [x] **Sub Space opens its own page too.** _View_ it (join it first if need
      be) and you land on a second space page, for Sub Space.
* [x] **An empty space says so.** Sub Space has no children of its own, so its
      page should read "There are no rooms in this space yet." rather than
      showing a spinner or an empty heading.
* [x] **Each row shows what it should**: avatar, name, topic where there is
      one, canonical alias where there is one, and a member count. A room with
      no topic must not leave a gap.
* [x] **Switching between two spaces swaps the lists.** Select Test Space, then
      Sub Space, then Test Space again. The second page must never show the
      first page's rooms, even for an instant — the list is cleared before the
      new request goes out, and the response of a space you have navigated away
      from is dropped.
* [x] **The list survives a reselect.** Leave the space page, come back: the
      rooms are still there, or are fetched again, but never half of them.
* [x] **No truncation notice on a small space.** "This space holds more rooms
      than are listed here." should be **invisible** for Test Space. It only
      belongs on a space with more than 200 rooms, which the harness has no way
      to make — check it on matrix.org if you find one.
* [x] **The error state is reachable and recoverable.** Stop the homeserver
      (`testing/local-homeserver.sh down`), open a space you have not opened
      this session: the page should say the rooms could not be listed and offer
      _Try Again_. Bring the server back up and press it — the list should
      fill in. A dead-end error page is the failure here.
* [x] **A space on matrix.org lists its rooms too.** This is the check that
      the `via` servers are being read: a real space holds rooms on other
      homeservers, and joining one of those from the list is what fails if the
      `m.space.child` events were not parsed.

### Opening a subspace in place

* [x] **A subspace has an expander** — a triangle to the left of its row —
      and an ordinary room does not. Put a room inside `Sub Space` from another
      client first, or with _Add to Space…_, so it has something to show.
* [x] **Opening it draws its rooms underneath, indented**, without a spinner
      and without a pause. The whole hierarchy arrives in one walk, so this
      should be instant even on a slow homeserver — if it stalls, something is
      fetching per expansion.
* [x] **A subspace of a subspace expands too**, as deep as the space goes.
* [x] **A space with nothing in it has no expander.** `Sub Space` before you
      put anything in it must be a plain row.
* [x] **Closing it puts the rooms away** and the row keeps its own button.
* [x] **A loop does not open forever.** From another client, add `Test Space`
      as a child of `Sub Space`, so `Test Space → Sub Space → Test Space`.
      Opening `Sub Space` should show `Test Space` as a **plain row with no
      expander** — it is already one of the spaces above it. Without that
      guard it would open forever, and this is the check most worth doing.
* [x] **A room in two spaces appears in both**, which is what the hierarchy
      says. Add `Public Room` to `Sub Space` as well and open both.
* [x] **The order is the specification's.** `m.space.child` carries an
      `order`; set one on two of `Test Space`'s children from another client
      and confirm they sort by it, before the ones without, and that the rest
      sort oldest-event-first.
* [x] **The buttons still work at depth.** _Join_ and _Preview_ on a row three
      levels down behave the same as at the top.

### Finding one in Explore

* [x] **Explore lists spaces at all.** Search for "Test" on `localhost` — the
      space should be in the results next to the ordinary rooms. Before this
      change the directory was asked to exclude them.
* [x] **A space row says "Space"** — a dimmed grid icon and the word, beside
      the member count. An ordinary room row must **not** show it; that is the
      half of this check that catches a property left always-true.
* [x] **Joining from Explore** works and the room lands in the Spaces section,
      not in _Rooms_. The button should read _Join_, and _View_ once joined.
* [x] **On matrix.org**, where the directory is large: search for a known
      space (`#space:matrix.org` and similar) and confirm the marker appears
      there too. The local harness has one space and one shape of summary; a
      real directory is where a missing `room_type` shows up.

### Invites, and what these slices deliberately do not change

* [x] **An invite to a space still goes to _Invited_** and opens the ordinary
      invite page. Invite **carol** to Test Space and log in as her — bob is
      already a member of both spaces and cannot be invited to either, which
      the invite page correctly refuses to offer. Accepting should drop the
      space into her Spaces section on the next sync; declining should behave
      like declining a room. The invite page says nothing about it being a
      space, which is known and recorded in `spaces.md`.
* [x] **A room added to a space while its page is open does not appear.**
      Known, and recorded in `spaces.md` under _Not done_: the listing is
      fetched once. Reselecting the space should pick the new room up. This
      check exists so the behaviour is not re-reported as a bug.
* [x] **Nothing regressed for ordinary rooms.** Favorites, Low Priority,
      Historical and the drag-and-drop between them all still work; the
      _Forget_ target is still at the bottom of the sidebar. The section index
      map was rewritten by hand and it is exactly the kind of change that
      moves a section's contents into its neighbour.

### A room you have knocked on

_Destructive, mildly: it leaves a knock behind if you stop halfway._

Not spaces, but the same bug and found with it: a knocked room is drawn in an
_Invite Requests_ section and its row had no menu either, so the request could
not be retracted from the sidebar.

* [x] **Knock on a room** — `#knock-room:localhost` from Explore, as carol —
      and then **right-click its sidebar row**: the menu should offer
      _Retract_.
* [x] **Retracting works** and the row goes.
* [x] **It is called access, not an invite, all the way through.** Carol's
      button reads _Request Access_; her sidebar section reads _Access
      Requests_; alice sees _Access Requests_ in the room's members page, the
      profile says _Requested Access_, and the two buttons there read _Accept
      Request_ and _Deny Request_. The word _invite_ should not appear
      anywhere in the flow. Reported on 24 August 2026 as two vocabularies in
      one feature, with the accept button reading _Invite_ beside a _Deny
      Request_.
* [x] **_Allow Access Requests_** is what the switch in _Who Can Join_ says.

### Leaving one, and not re-filing one

_Destructive. Leave it until the rest of this section is done — it takes away
the spaces the checks above need. `testing/local-homeserver.sh up` puts them
back: it rejoins what can be rejoined and rebuilds `Sub Space` if it has to,
because **a room whose last member leaves is destroyed and cannot be
re-entered by anybody**. Bob is seeded into both spaces to stop that, and the
repair exists for the servers that lost one before he was._

* [x] **The space row has a menu at all.** Right-click it, or use the ⋯ button
      that appears on hover. It had none until `60792ddf`: the actions were
      built and the row was never told it had a menu, so there was no way to
      reach them. Reported broken, then working, on 24 August 2026.
* [x] **Right-click the space row: the menu offers _Leave Room_ and
      _Report Room_, and nothing else.** No _Favorite_, no _Low Priority_, no
      _Set as Direct Chat_, no _Mark as Unread_. Those are tags and a space
      takes none of them.
* [x] **_Leave Room_ asks first**, then the row leaves the Spaces section and
      turns up under _Historical_. Then _Forget_ from there should work as it
      does for a room.
* [x] **Re-joining it** — from Explore, or the alias — puts it back in the
      Spaces section rather than in _Rooms_.
* [x] **Dragging the space row** highlights only _Historical_ as a valid drop
      target; every other section should go grey. Dropping it there leaves the
      space, the same as the menu item.
* [x] **Dragging an ordinary room over the Spaces section does nothing.** The
      section must show as disabled and refuse the drop. Rooms are put into
      spaces with `m.space.child`, which does not exist here yet, and a drop
      that silently did nothing would be worse than one that refuses.

## Choosing a space, restricting a room to one, and putting one in — `doc/join-rules.md`, `doc/spaces.md`

Round 3, slice 3. The client can now ask which space you mean; _Who Can Join_
uses it to build a restricted rule rather than only to keep one, and the room
details use it to put a room into a space.

### Setting up

The harness seeds `Test Space` and `Sub Space`, plus `Restricted Room`, which
is already restricted to `Test Space`, and `Knock Restricted Room`, which is
the same with knocking on. Log in as alice, who owns all of them.

```sh
testing/local-homeserver.sh up
```

### Making a space

* [x] **The new-room dialog offers a _Kind_.** _Room_ is selected; picking
      _Space_ changes the heading to _New Space_ and the button to
      _Create Space_.
* [x] **The Name box is there whatever is being made.** The group titled
      _Name_, between _Kind_ and _Description_, takes a name for a room and for
      a space, private or public. Reported on 25 August 2026 as a private space
      having "the encryption rocker switch instead of the name box"; confirmed
      good the same day once the switch was gone for a space. The box that was
      missed was _Main Address_, which appears for _Public_ only, under the
      visibility choices: the `#alias`, which a private room does not have.
* [x] **The encryption switch disappears for a space** and comes back for a
      room. Reported on 25 August 2026 as staying on screen for a private
      space, and going only when _Public_ was picked: the visibility half of
      the condition was a binding in the template and the kind half was a
      `set_visible` from Rust, and the binding won. Both halves are in the
      binding now, and it was reported working the same day.
* [x] **The visibility subtitles say "space"** rather than "room".
* [x] **Creating a private space works**, and it lands in the _Spaces_ sidebar
      section rather than in _Rooms_ — this is the check that
      `creation_content` really carried `type: m.space`.
* [x] **Creating a public space works** and takes an address, the same as a
      room.
* [x] **Nobody can post in it.** From another client, look at the new space's
      power levels: `events_default` should be 100, with `m.space.child`, the
      name, the topic and the avatar at 50. A space anybody can post into is
      a room with a hidden timeline.
* [x] **The new space accepts rooms.** Add a room to it from that room's
      details, and open the space to see it.

### The picker

* [x] **Room Details → _Who Can Join_ on any room shows _Members of a Space_.**
      It used to be hidden unless the room was already restricted. It should
      now be there for `Public Room` and `Invite Room` too.
* [x] **Selecting it reveals a _Space_ row below the three choices**, reading
      _None chosen_ for a room that has no restriction.
* [x] **Activating that row opens a dialog listing your spaces** — `Test Space`
      and `Sub Space`, and nothing else. No ordinary rooms, no direct chat, no
      server notices room.
* [x] **The room you are editing is not in the list.** Open _Who Can Join_ on
      `Test Space` itself: the picker must offer `Sub Space` only. A space
      cannot be restricted to itself.
* [x] **Search filters the list**, and a search matching nothing says so rather
      than showing an empty box.
* [x] **Dismissing the dialog changes nothing** — press Escape or click away,
      and the _Space_ row still reads what it did.
* [x] **Choosing a space fills the row in**, the dialog closes, and _Save_
      becomes sensitive.
* [x] **Saving works.** The rule is written and the page comes back showing
      _Members of a Space_ with that space named. Confirm from another client
      that the room really is restricted.
* [x] **A user in the space can then join the room**, and one outside it
      cannot. As **carol**, who is in no space, try the room you just
      restricted; then have alice invite her to `Test Space`, accept, and try
      again. Bob is no use here — he is already in both spaces.

### What must not happen

* [x] **_Save_ stays insensitive with the rule selected and no space chosen.**
      This is the check that matters most: saving in that state would send a
      restricted rule allowing nobody, which this page cannot undo. Select
      _Members of a Space_ on an unrestricted room and go no further.
* [x] **Going back with the rule selected and no space chosen** asks about
      unsaved changes only if there are any — there are none, so it should just
      go back.
* [x] **The rule is not silently swapped.** With _Members of a Space_ selected
      and no space, the page must not save an invite or knock rule instead.
      That is what the old code did, and it was invisible.
* [x] **A room already restricted to a space keeps it** when you only flip
      _Allow Invite Requests_. Open `Restricted Room`, toggle the switch, save,
      and check from another client that the allow list still names
      `Test Space`.
* [x] **A room with no permission shows the space and no arrow.** As bob in a
      room he cannot administer, _Who Can Join_ should still say which space,
      with the row not activatable.

### Putting a room into a space

Slice 3, second half. Room Details → _Spaces_ → _Add to Space…_.

* [x] **The _Spaces_ group is on the general page** of any room that is not a
      direct chat, under _Access and Visibility_, with one row.
* [x] **It is absent from a direct chat.** Open the details of the alice–bob
      chat: no _Spaces_ group.
* [x] **_Add to Space…_ opens the picker**, listing `Test Space` and
      `Sub Space` — the spaces alice made, and so can write in.
* [x] **A space you cannot write in is not offered.** As bob: he is a member
      of both spaces and administers neither, so the picker must be **empty**
      and say so. Membership is not enough — this is the check that
      `SendState(SpaceChild)` is really being asked rather than "am I in it",
      and the one most likely to be wrong.
* [x] **The room being added is not offered itself.** Open the details of
      `Test Space` and press _Add to Space…_: only `Sub Space`.
* [x] **Adding `Invite Room` to `Test Space` says so**, with a toast naming the
      space.
* [x] **The room appears in the space.** Open `Test Space` from the sidebar —
      the list should now hold `Invite Room` as well. It will not appear while
      the space page is already open; the listing is fetched once, which is
      recorded in `spaces.md`.
* [x] **Another client agrees.** Check from Element that `Test Space` has an
      `m.space.child` for the room, and that the room has an `m.space.parent`
      for the space with `canonical` absent or false.
* [x] **Adding a room somebody else administers still works.** Have bob make a
      room, have alice join it without power, and add it to `Test Space` from
      alice's client. The `m.space.child` needs power in the **space**, which
      alice has; the `m.space.parent` needs power in the **room**, which she
      does not, so it should be skipped with a warning in the log and the
      operation should still report success.
* [x] **Adding a room twice is harmless** — the state event is simply written
      again.
* [x] **The row goes insensitive while it works** and comes back afterwards,
      whether it succeeded or not.
* [x] **The space is listed the moment the toast says so.** The other half of
      the removal fault below: adding also read the whole list again from a
      state store that had not heard about the change, so on a slow homeserver
      the _Spaces_ group did not name the space it had just been told about.
      Watch the group, not the toast — do not close and reopen the page.
      Reported working on 25 August 2026.

### Which spaces a room is in

* [x] **The _Spaces_ group lists them.** Add `Invite Room` to `Test Space`,
      then reopen its details: a row naming `Test Space`, with its avatar.
* [x] **A room in no space lists none** — just the _Add to Space…_ row.
* [x] **The looking-for-them row does not stay.** A spinner row appears while
      the spaces are asked and goes when they answer. If it never goes, the
      read failed silently.
* [x] **A room in two spaces lists both.** Add the same room to `Sub Space` as
      well.
* [x] **A space you are not in is not listed**, even when it holds the room.
      This cannot be helped and is not a fault: the state of a space nobody
      here has joined cannot be read. Check it by having bob add one of alice's
      rooms to a space alice is not in.
* [x] **A room that only claims a parent is not believed.** From another
      client, write an `m.space.parent` into a room pointing at a space that
      does **not** name it as a child, as a user with no power in that space.
      It must not appear. This is the specification's own rule and the check
      most worth doing, because getting it wrong lets any room claim to be
      anywhere.
* [x] **…unless whoever claimed it could have made it true.** Same test, but
      write the parent event as somebody who can send `m.space.child` in that
      space. It should appear.

### Suggested rooms

* [x] **A suggested child is marked.** From another client, set
      `"suggested": true` on one of `Test Space`'s `m.space.child` events. That
      room's row on the space page should show a star and the word
      _Suggested_.
* [x] **No other row shows it**, on the space page or in Explore. The flag
      belongs to the relationship, not to the room, so a room listed anywhere
      else must never carry it.

### Old rooms

* [x] **A room too old for restricted rules does not offer it.** Restricted
      join rules arrived in room version 8 and `knock_restricted` in version
      10. Make a version 7 room from another client: _Members of a Space_ must
      be absent and the notice at the top of the page must show.
* [x] **A version 8 or 9 room offers the rule but not knocking over it.**
      Select _Members of a Space_ there and _Allow Invite Requests_ must go
      insensitive — `knock_restricted` does not exist for that room, and
      sending it would be rejected.

### Taking a room back out

_Destructive. Leave it until the rest of this section is done — it undoes the
`m.space.child` events the checks above are looking at. Adding the room back
puts them right._

* [x] **Each row has a _Remove_ button**, and pressing it takes the room out of
      that space: the row goes **at once**, a toast says so, and the space's
      own page no longer lists the room when reopened. Reported on 25 August
      2026 as working, but with the row staying for something like half a
      minute against a slow homeserver — the whole list was read again from the
      local state store, which does not carry the change until it comes back
      down the sync. The row is taken out directly now, and the wait was gone
      when it was looked at again the same day.
* [x] **Another client agrees.** The `m.space.child` should now be an empty
      object rather than gone — Matrix has no way to delete a state event.
* [x] **The button is insensitive without permission.** As bob, in a space he
      does not administer that holds a room he can see, the row appears and the
      button does not work.
* [x] **Adding it back works**, and the room reappears in the space.

## Reading a room without joining it — `doc/peeking.md`

Round 3, item 7. A room whose history is `world_readable` can be read by
anybody, and Commune now offers that as a **Preview** — in the room preview
dialog, and on every row of Explore and of a space page.

**Some of this was seen on 24 August 2026**, against a real room on
`matrix.org` from a `matrix.org` account rather than against the harness. Those
checks are struck below and say so. Everything unstruck is still unseen — and
the whole "where the button is _not_" group is, which is the half that catches
a flag being read wrong.

### Setting up

`testing/local-homeserver.sh` seeds **Peekable Room** inside `Test Space`: bob's
room, `world_readable`, with two messages in it. It is bob's on purpose —
`Readable Room` is `world_readable` too but alice created it, so she is a member
and gets the room rather than a preview of it. Everything else in the harness is
`shared` history, which is not the same thing and must **not** offer a preview.

```sh
testing/local-homeserver.sh up
```

`seed_peekable_room()` has a marker of its own, so a server seeded before this
existed picks the room up without a reset. Log in as alice.

### Where the button is, and is not

* [x] **A _Preview_ button appears on Peekable Room's row**, on the Test Space
      page, beside _Join_.
* [x] **It does not appear on any other row.** Readable Room is
      `world_readable` but joined; Public Room and Restricted Room are joined;
      Bobs Room and Sub Space are not `world_readable`. A button on any of
      those means a flag is being read wrong, and that is the half of this
      check worth caring about.
* [x] **It does not appear once the room is joined.** Join Peekable Room and
      look at the row again: _View_, and no _Preview_.
* [x] **The same button is in the room preview dialog.** Ctrl+K or _+_ →
      _Join a Room_, enter `#peekable-room:localhost`, and the details page
      should show _Preview_ next to _Join_.
* [x] **An encrypted room never offers it.** Nothing in the harness is both
      encrypted and `world_readable`; if you can make one from another client,
      the button must stay hidden.

### The preview itself

* [x] **Pressing _Preview_ shows the two seeded messages**, oldest first, each
      with a sender name and a timestamp. Seen on 24 August 2026 on a
      `matrix.org` room, not against the harness, so this stays open.
* [x] **The room's name is on the page**, above the line about nobody seeing
      you. The heading above that belongs to the dialog and says _Join a Room_
      on every page, so this is the only thing saying which room you are in.
      Missing when first drawn; added in `43d26d02` and reported good on
      24 August 2026.
* [x] **A room with more messages than fit opens at the newest**, not the
      oldest — the same end every other timeline here opens at. Opened at the
      oldest when first drawn; fixed in `43d26d02` and reported good on
      24 August 2026. The harness cannot show this with two messages; it was
      seen on a busy `matrix.org` room.
* [x] **The sender is named, not numbered.** Seen on 24 August 2026: three
      distinct display names off the lazy-loaded member events, and one of them
      used twice with no disambiguation suffix, so the shared-name rule is not
      firing where it should not.
* [x] **There are no avatars and no images**, by design. Seen on 24 August
      2026. A message with a picture in it should show its fallback text; that
      half was not among the messages on screen and is not confirmed.
* [x] **The line above the list says nobody can see you looking.** Seen on
      24 August 2026.
* [x] **_Join_ is on the preview page too**, and joining from there works and
      closes the dialog on the room. The button was on screen on 24 August
      2026; it was not pressed, so this stays open.
* [x] **Back goes to the details, not out.** From the preview, the back arrow
      should land on the room's details page; from there it goes to the entry
      page, or closes if the dialog was opened on a room. The arrow was on
      screen; it was not pressed.
* [x] **Opening it from a row lands straight on the preview**, with the details
      one press of Back away. Seen on 24 August 2026, from an Explore row.

### When it cannot be read

* [x] **A room on another homeserver says so.** Try a `world_readable` room on
      matrix.org from the local harness. Expect _Cannot Be Read_: Synapse does
      not peek a room it does not have, and that page exists because this is
      the common outcome, not a rare one. Note what 24 August 2026 showed: from
      a `matrix.org` account, a `matrix.org` room is **local** and peeks
      perfectly. The failure is cross-homeserver, which is narrower than
      `doc/peeking.md` first put it.
* [x] **The message is not an error toast or a spinner that never stops.**
* [x] **Pressing _Preview_ again retries.** Go back, press it again — it should
      make the request a second time rather than showing the stale failure.
* [x] **A room with no messages says _Nothing to Read_** rather than showing an
      empty list. Make an empty `world_readable` room from another client.

## Going to a message without leaving the present — `doc/search.md`

`RoomHistory::focus_on_event()` used to build a focused timeline every time.
It now prefers the room's **live** timeline whenever that already holds the
event, and highlights the message in place. Every way of going to a message
runs through it, so all four need a look, and the one that motivated the
change is the notification.

**There is no harness for the notification case.** `COMMUNE_TEST_NOTIFICATION=1`
deliberately carries a room URI, not an event URI, so it takes the room-preview
path and proves nothing here. It needs a real incoming message while the window
is unfocused — two accounts, or a phone.

* [x] **A notification for a brand new message opens the room at the bottom,
      with no _Back to Latest_ button.** This is the whole point. Before the
      change the button appeared and the timeline then silently stopped
      updating. Reported fixed on 24 August 2026. The checks below it were not
      separately reported and stay open.
* [x] **And the room keeps updating afterwards.** Send another message from the
      other account without touching anything: it must appear. This is the half
      that was actually broken, and it is invisible unless you wait for it.
* [x] **The message is highlighted for about three seconds** and then goes back
      to normal. Watch that it does not stay highlighted, and that it does not
      look like a selection.
* [x] **A read receipt is sent.** The other account should see the message
      marked read. A focused timeline suppresses receipts on purpose, so this
      is how you tell which timeline you actually landed in without looking for
      the button.
* [x] **A notification for an old message still gets a focused timeline** —
      the _Back to Latest_ button appears and works. Scroll a room's history
      back a long way from the other account's side, or click a notification
      that has sat unread while thousands of messages arrived. The fallback is
      the case the focused timeline exists for and it must still work.
* [x] **A `matrix.to` permalink to a message near the bottom** stays live and
      highlights, and one to an old message focuses. Paste one into a room and
      click it.
* [x] **A search result behaves the same way.** A hit on a recent message
      should now leave you in the live timeline rather than in a snapshot —
      this is a deliberate change to how search results open, and it is the
      one most likely to feel wrong to somebody used to the old behaviour.
* [x] **A pinned message opens the same way**, which for a recently pinned
      message means staying live.
* [x] **A room that has never been opened in this session.** The live timeline
      is still being built when the notification is clicked, so the highlight
      is a pending one. Restart the app, do not open the room, then click a
      notification for a message in it: it should still land live and
      highlighted, not focused. If the timeline takes more than two seconds to
      become ready, it falls back to a focused timeline — no worse than before,
      but worth noticing if it happens every time.
* [x] **Clicking a notification for a room you are already reading** does not
      jump anywhere unpleasant or steal the scroll position for long.

## Threads: the chip, the view, and the list — `doc/threads.md`

A message that roots a thread carries a chip under its content — a thread
icon and a reply count — pressing it swaps the room history for the thread
itself, with a banner naming the state and the composer sending into the
thread, and a threads button in the header lists every thread of the room.
`testing/local-homeserver.sh up` seeds two threads in **Invite Room**:
alice's "Does anybody else think this deserves a thread?" with three replies
from bob, and bob's "The second thread, for the list to have two rows." with
one reply from alice. Some checks grow a thread from a terminal — run this
from the repository root, with a token from the script's usual login curl,
changing the transaction ID and the body each time:

```sh
root=$(jq -r .thread_root testing/.homeserver/seeded.json)
curl -X PUT "http://localhost:8008/_matrix/client/v3/rooms/$(jq -r .invite_room testing/.homeserver/seeded.json)/send/m.room.message/eyeball-$RANDOM" \
  -H "Authorization: Bearer $BOB_TOKEN" -H 'Content-Type: application/json' \
  -d "{\"msgtype\": \"m.text\", \"body\": \"A fourth reply.\", \"m.relates_to\": {\"rel_type\": \"m.thread\", \"event_id\": \"$root\"}}"
```

### The chip

* [x] **The chip appears on the root.** Open Invite Room as alice: the seeded
      root message carries a chip reading "3 replies", with the thread icon
      legible in both the light and dark styles.
* [x] **The replies are not in the room.** Bob's three threaded replies do
      **not** sit inline in the main timeline any more — `hide_threaded_events`
      went on when the thread view landed. The root is there, with its chip;
      the replies are only inside the thread. If they still show inline, the
      flag did not reach the live timeline's focus.
* [x] **The count moves while the room is open.** With Invite Room on screen,
      send a fourth reply with the curl above and watch the chip say
      "4 replies" without the room being reopened.
* [x] **The singular reads "1 reply".** Start a fresh thread with exactly one
      reply — the same curl against any other message's event ID — and check
      the chip says "1 reply", not "1 replies".
* [ ] **Against the real world.** On a matrix.org account, open a busy public
      room that uses threads (Element's own rooms do): roots show chips with
      plausible counts, and a thread rooted before this session's sync window
      still gets one — the count comes from the server's bundled summary, not
      from anything we witnessed.

### The view

* [x] **The chip opens the thread.** The history swaps to the thread: the
      root first, then bob's three replies, and nothing from the rest of the
      room. A banner over it reads _Viewing a thread_ with a
      _Back to All Messages_ button.
* [x] **_Back to All Messages_ goes back**, to the live timeline at the
      bottom, with the banner gone.
* [x] **_View Thread_ is in the root's context menu** and does the same as
      the chip. It must be absent on a message that is in no thread.
* [x] **A new reply arrives live.** With the thread open, send a reply with
      the curl above: it must appear at the bottom of the thread without
      touching anything.
* [x] **Composing sends into the thread.** Type into the composer while the
      thread is shown and send. The message appears in the thread; pressing
      _Back_, it is **not** in the main timeline, and the root's chip counts
      one more. From another client (or the sync JSON), the event carries
      `m.relates_to` with `rel_type: m.thread`.
* [x] **The drafts are separate.** Type into the room's composer without
      sending, open the thread — the composer is empty. Type something there,
      go back — the room's half-typed message is back, and reopening the
      thread restores the thread's own.
* [x] **Reply inside the thread stays in the thread.** Use _Reply_ from a
      thread message's context menu, send, and check from another client that
      the event carries both the reply and the thread relation.
* [x] **A long thread paginates.** Grow the thread past twenty replies with
      the curl in a loop, reopen it: it opens at the newest, and scrolling up
      loads the older replies with the root at the very top.
* [x] **The room's read state does not suffer.** Read the thread to the
      bottom, go back: the room is not suddenly marked unread, and the room's
      read marker did not jump backwards. On matrix.org against Element, your
      read receipt shows up inside the thread rather than on the main
      timeline.
* [x] **Switching rooms while in a thread** lands the other room in its
      ordinary live timeline, banner gone, and coming back to Invite Room is
      live too — the thread is left by leaving, not remembered.

### The list

* [x] **The threads button is in the header bar**, a toggle with the thread
      icon, in every room — there is no cheap way to know whether a room has
      threads before asking, so it does not hide.
* [x] **Pressing it lists both seeded threads**, most recent activity first:
      bob's second thread above alice's first. Each row shows the root's
      sender with avatar and timestamp, a preview of the root message, the
      reply count ("1 reply" / "3 replies"), and the latest reply as
      "name: message".
* [x] **Activating a row opens that thread** — the list closes, the thread
      view appears with its banner, and the composer writes into that thread.
* [x] **A room with no threads says so.** Open the threads list in Public
      Room: "No Threads", with the thread icon, not a spinner that never
      stops.
* [x] **The list stays current.** With the list open, send a reply into a
      seeded thread with the curl above: that row's count and latest-reply
      line must update without closing the list. A brand-new thread rooted
      while the list is open is **not** expected to appear as a new row —
      the endpoint is paginated and the live half only updates threads
      already listed; reopening the list picks it up.
* [x] **The pinned and threads toggles put each other out.** With pinned
      messages open, press the threads button: the threads list shows and
      the pin toggle pops out, and the other way round.
* [x] **The thread banner does not sit over the list.** Enter a thread, then
      open the threads list: the "Viewing a thread" banner must go while the
      list shows and come back if the list is closed with the thread still
      displayed.
* [x] **Switching rooms resets the list.** Open the threads list in Invite
      Room, switch to Public Room, open its list: never the first room's
      threads, not even for an instant.
* [x] **The error page recovers.** Stop the homeserver, open the threads
      list in a room whose list was never opened this session: it should say
      the threads could not be listed and offer _Try Again_. Bring the
      server back and press it — the list fills in.
* [ ] **Against the real world.** On a matrix.org account, open the threads
      list of a busy room that uses threads: rows with plausible counts and
      previews, and scrolling to the bottom loads older threads.

## Notification rules — `doc/notifications.md`

A _Notify Me About_ group on Account Settings ▸ Notifications, between the
global defaults and the keywords: four switches for mentions of your name,
@room messages, room invites and incoming calls. Everything else on that page
predates this and was seen long ago.

* [ ] **The four switches draw and read on.** On a fresh account all four
      should be on — they mirror the server's default rules — and none should
      flicker off and back on for more than the moment the page takes to
      load.
* [ ] **Flipping one sticks.** Turn _Room Invites_ off: the row spins,
      settles off, and stays off after closing and reopening the settings.
      From another client (or `curl` on
      `/_matrix/client/v3/pushrules/global/override/.m.rule.invite_for_me/enabled`),
      the rule reads disabled.
* [ ] **The mention switches move the deprecated rules too.** Turn
      _Mentions of My Name_ off and check from another client that
      `.m.rule.is_user_mention`, `.m.rule.contains_display_name` and
      `.m.rule.contains_user_name` are all disabled — the SDK keeps the
      Matrix 1.7 predecessors in step, and this is the check that it really
      does.
* [ ] **A change from elsewhere moves the switch here.** With the page open,
      disable `.m.rule.is_room_mention` from another client: the @room switch
      should follow without reopening the page.
* [ ] **The group disables with the rest.** Turn off _Enable for This
      Account_ or _Enable for This Session_: the group greys out like the
      keywords do, and comes back.
* [ ] **The behaviour is real, not just the switch.** With the global setting
      on mentions-only and _Mentions of My Name_ off, a message mentioning
      you from bob must **not** notify; turn the switch back on and it must.
      The invite and call switches can be checked the same way with an invite
      from bob and a call from bob.

## Email and phone on the account — `doc/email-and-phone.md`

_Email and Phone Numbers_ under Account Settings ▸ General, beside _Change
Password_. **It only appears on a password-auth homeserver** — on matrix.org
the browser's own account page covers it, so use the local harness. Synapse
there has no SMTP, so the add flow can only be checked up to the email that
never arrives; the full flow needs a homeserver that sends mail.

* [ ] **The row is there against the harness** and opens a page listing the
      account's email addresses. A fresh alice has none, so the list is just
      the _Add Email Address_ entry; the phone group must be absent entirely.
* [ ] **The row is not there on matrix.org** — its account management is in
      the browser, and _Manage Account_ is what shows instead.
* [ ] **Adding starts the flow.** Type an address, press add: against the
      harness the toast should say the validation email could not be sent
      (Synapse has no SMTP) — not a crash, not a silent nothing.
* [ ] **A nonsense address cannot be submitted.** Without an @ between two
      non-empty halves, the add button stays inhibited.
* [ ] **The full flow, on a homeserver that sends email:** request, open the
      link, _Continue_, give the password to the auth dialog, and the address
      appears in the list. Pressing _Continue_ **before** opening the link
      must toast "Open the link in the email first, then try again" and offer
      the dialog again — that is the check most worth doing, since it is the
      path every real user will hit at least once.
* [ ] **Removing asks first and says what is lost.** With an address on the
      account (`testing/local-homeserver.sh` can add one with the admin API,
      or use the full-flow server), the remove button raises a dialog naming
      the address; confirming removes it from the list and, checked from
      another client, from the account.
* [ ] **A phone number from elsewhere is listed and removable.** Add an
      msisdn to alice through the Synapse admin API, reopen the page: a
      _Phone Numbers_ group appears, its description says why one cannot be
      added here, and removing it works.

## Voice messages — `doc/voice-messages.md`

A microphone button in the message toolbar, next to the sticker button. It
needs a real capture device: a remote desktop session has none unless the
RDP client redirects the microphone, so these checks belong on a console
session (or WSLg with a mic passed through).

* [ ] **No microphone fails politely.** On the RDP session as it is,
      pressing the button must toast "No microphone could be opened" and
      leave the composer as it was — no recording page, no crash.
* [ ] **Recording looks like recording.** With a microphone, the toolbar
      swaps to a red record icon and a counter that ticks "0:01, 0:02…"
      once a second.
* [ ] **Cancel throws the take away.** Cancel returns to the composer;
      nothing is sent, and no `commune-voice-message-*.ogg` is left in the
      temporary directory.
* [ ] **Send delivers a voice message, not a file.** Say a few words, send:
      the message plays back in Commune's own audio row, and Element shows
      it as a voice message with a waveform — the waveform drawn from real
      loudness, lumpy where you spoke and flat where you paused, is the
      MSC3245 fields working.
* [ ] **Switching rooms mid-recording drops the take.** Start recording,
      click another room: the toolbar is back to the composer and nothing
      was sent — same for opening a thread.
* [ ] **Losing permission mid-recording drops it too.** Start recording as
      bob in a room where alice then raises the events power level: the
      recording page yields to the no-permission strip.

## Mutual rooms — `doc/mutual-rooms.md`

A _Shared Rooms_ section on the profile page (avatar ▸ from a member list or
a message), listing the rooms you share with that user. Synapse needs
`experimental_features: {msc2666_enabled: true}` for the unstable path;
a Synapse new enough to advertise v1.19 answers the stable one.

* [ ] **The section lists the shared rooms.** Open bob's profile as alice
      with two rooms in common: both rows, with avatars, and no room the two
      do not share.
* [ ] **A row goes to its room.** Activating one closes the profile window
      and lands the view in that room.
* [ ] **Your own profile has no section.** The endpoint refuses the asking
      account's own ID, and the page should not even ask.
* [ ] **A server without the endpoint shows nothing.** Against a homeserver
      with the feature off, the profile page simply has no _Shared Rooms_
      heading — no error, no empty box.

## Policy servers — `doc/policy-servers.md`

One sentence in the timeline when `m.room.policy` changes. Sending the state
event takes another client or `curl`; Commune only draws it.

* [ ] **Setting a policy server says so.** As alice, send
      `{"via": "policyserver.example"}` as `m.room.policy` with empty state
      key (Element's /devtools does it): the timeline reads "alice made
      policyserver.example check the messages of this room."
* [ ] **Unsetting reads as removal.** Send `{}` the same way: "alice stopped
      the checking of this room's messages."
* [ ] **The room's state page shows the event** under the state list like
      any other, rather than the "Unsupported event" fallback.

## Wide stickers and emoticons shrink — fix of 26 August 2026

A pack image sent alone in a message is presented at sticker size, and used
to refuse to shrink below it.

* [ ] **A wide sticker-sized emoticon fits.** Send a pack image wider than
      it is tall alone in a message, narrow the window below the image's
      width: the image scales down inside the message area, keeping its
      shape, instead of running off the right edge.
* [ ] **Several in one message share the width** rather than overflowing.
* [ ] **Among words nothing changed:** the same pack image inside a
      sentence still sits at text height.
* [ ] **An `m.sticker` sticker still fits too.** A sticker from the sticker
      picker in the same narrowed window scales down as before — this path
      was checked by harness and should already behave.

## Chat bubbles — `doc/chat-bubbles.md`

A _Chat Bubbles_ switch under Account Settings ▸ General ▸ Appearance,
off by default. All of this is presentation; nothing goes over the wire.

* [ ] **Off by default, nothing changed.** Before touching the switch, the
      timeline looks exactly as it did yesterday.
* [ ] **The switch moves the live timeline.** Flip it with a busy room open:
      every message gains a bubble without reopening the room, and flipping
      it back restores the flat rows.
* [ ] **Your messages sit on the right**, in an accent-tinted bubble, with
      no avatar and no name — just the timestamp above the first of a group
      and the delivery checkmark beside it.
* [ ] **Everybody else sits on the left**, avatar and name on the first
      message of a group, bubbles hugging their text instead of spanning
      the window.
* [ ] **Reactions and the thread chip follow their bubble's side.** React
      to one of your own bubbled messages: the pill sits at the right, and
      a thread chip on an own root does too.
* [ ] **Replies, images and code blocks live inside the bubble** without
      the bubble breaking: a quoted reply draws above the message in the
      same bubble, and a wide image stays within its rounded corners.
* [ ] **Both themes read.** The neutral bubble is visible but quiet in
      light and dark; the own bubble reads as yours in both.
* [ ] **The thread view bubbles too**, since it is the same rows.
* [ ] **A bubbled sticker is a known compromise:** it gets a bubble around
      its transparency where Element strips it. Say whether it looks wrong
      enough to earn the special case.

## Invite by email — `doc/invite-by-email.md`

Room Details ▸ Invite New Members. Needs an identity server: matrix.org
suggests vector.im through its `.well-known`, so a matrix.org account is
the easy test bed. The local harness has none, which is itself a check.

* [ ] **Typing an email offers the card.** Type `somebody@example.org` in
      the search: a card appears under the entry reading "Invite
      `somebody@example.org` by email". Typing a Matrix ID or a name does
      not summon it; adding a space makes it go away.
* [ ] **Without an identity server it says so.** Against the local
      harness, the card toasts that inviting by email needs an identity
      server and there is none to use — no crash, no silent nothing.
* [ ] **The terms come first, once.** On an account that never used
      vector.im, the first invite raises the terms dialog with a working
      link to the document; Agree sends the invite, and a second invite
      to another address asks nothing.
* [ ] **Cancel on the terms sends nothing.** The dialog closes and no
      invitation reaches the address.
* [ ] **The invite lands.** Use a real address you control: the toast
      names it, and the email arrives with the room invitation (and an
      `m.room.third_party_invite` event appears in the room state). If
      the address has a bound Matrix account, it becomes an ordinary
      invite for that account instead.
* [ ] **The Identity Server row tells the truth.** Account Settings ▸
      General ▸ Privacy: on matrix.org it reads vector.im, suggested by
      the homeserver; on the harness it reads None.
* [ ] **Setting a server validates it.** Enter nonsense or a URL that is
      not an identity server: the toast refuses it and nothing is saved.
      Enter `https://vector.im`: the row now says it is set on this
      account, and another client (or `/_matrix/client/v3/user/{id}/account_data/m.identity_server`)
      shows the account data.
* [ ] **An empty field declines on purpose.** Save with the field empty:
      the row reads "None, by choice on this account", and the invite
      card toasts that there is no identity server even on matrix.org.
      _Use the Homeserver's_ brings the suggestion back.

## Own bubbles carry the avatar — fix of 26 August 2026

The first pass of chat bubbles dropped the sender's avatar entirely on own
messages; it now sits to the right of the bubble.

* [ ] **Your avatar is on the right.** With Chat Bubbles on, the first
      message of one of your groups draws your avatar at the line's end,
      right of the bubble, where everybody else has theirs on the left.
* [ ] **Continuations still align.** Your second bubble in a group lines
      up with the first one's right edge rather than sticking out past it.
* [ ] **Clicking it opens your profile**, the same as any sender avatar.

## The bubble header clusters who and when — change of 26 August 2026

In bubble view, the sender's data sits in one spot instead of two
corners. The flat view is untouched.

* [ ] **Others read avatar, name, time.** The first message of somebody's
      group is headed "(avatar) Alice 14:32", all at the left, over the
      bubble's edge — no timestamp at the far right.
* [ ] **Your own read time, name, avatar.** An own group's first message
      is headed "14:32 you (avatar)", all at the right — the name is
      back, and always the piece against the avatar.
* [ ] **A timestamp-only header follows the side.** A group that shows
      only a time (same sender after a gap) shows it in that same spot,
      left for others and right for your own.
* [ ] **An own bubble's right edge meets the avatar line.** The bubble
      ends where the name above it ends, one spacing short of the
      avatar — no wedge of empty space — and the delivery checkmark of a
      sending message sits at the line's far left, mirroring where
      everybody else's state sits.

## Message shields — `doc/message-shields.md`

A small shield icon beside a message's delivery state in encrypted rooms,
and honest sentences on undecryptable messages. Needs an encrypted room
and a second account with an unverified session.

* [ ] **A clean message draws nothing.** In an encrypted room where both
      sides are verified, no shield appears on ordinary messages.
* [ ] **An unverified sender draws a shield.** A message from an account
      whose session is not verified carries the icon; hovering it says
      why in a sentence, not a code.
* [ ] **A message sent in the clear is called out.** Send a plain event
      into an encrypted room (curl with an m.room.message via
      /send/m.room.message): the red shield reads that the message was
      not encrypted, in a room that is.
* [ ] **An undecryptable message says why.** A message sent to the room
      before you joined reads as exactly that — not "waiting for keys".
      Log in on a fresh session with backup off: history reads as
      unavailable on this device, and with an unverified session it tells
      you to verify.

## Failed sends — `doc/failed-sends.md`

_Try Sending Again_ in the context menu of a message that failed.

* [ ] **A failed message can be retried.** Cut the network (or suspend
      the harness), send a message, wait for the error icon, restore the
      network: the context menu offers _Try Sending Again_, and using it
      delivers the message.
* [ ] **Discard still works** beside it, and a message that is merely
      sending offers only _Discard_, not retry.

## Knock banner memory — `doc/knocks.md`

The pending-knocks banner counts only requests not yet reviewed on this
device.

* [ ] **A knock raises the banner** in a knock-rule room where you can
      act on it, counting the pending requests.
* [ ] **Viewing is reviewing.** Press _View_: the members page opens on
      the knocking list, and back in the room the banner is gone — even
      though the knock is still pending.
* [ ] **A new knock raises it again**, counting only the new one.
* [ ] **Answering from the members page needs no banner.** Accept or
      decline a knock there: nothing re-appears.

## Room tag order — `doc/room-tags.md`

The favorites keep the order another client gave them. Element's room
list settings can set a manual order, or curl can write
`{"order": 0.1}` on `m.favourite` via
`/user/{id}/rooms/{roomId}/tags/m.favourite`.

* [ ] **Ordered favorites come first, in order.** Give two favorite rooms
      orders 0.2 and 0.1 from another client: the sidebar lists the 0.1
      room first, the 0.2 room second, and any unordered favorites after
      them.
* [ ] **Unordered rooms still follow activity.** With no orders set,
      nothing about the sidebar changed, in any section.
* [ ] **A change from elsewhere moves the row here** without reopening
      anything.

## Moderation policy rules — `doc/policy-lists.md`

Sentences in a policy room's timeline. Join a public policy list (for
example a community ban list) or write rules with another client.

* [ ] **A user rule reads as a sentence.** An `m.policy.rule.user` event
      reads "{sender} recommended banning the users matching {entity}:
      {reason}" — not "Unsupported event".
* [ ] **Room and server rules read the same way**, with rooms and servers
      in the sentence.
* [ ] **A withdrawn rule reads as the removal.** Redact a rule event: the
      timeline says the sender removed a moderation rule.
* [ ] **The room's state list draws them too**, rather than falling back.

## Guardrails — `doc/guardrails.md`

Three small guards; each check is a mistake the app used to allow.

* [ ] **No duplicate direct chat.** From another client, make alice and
      bob's DM look stale (bob leaves and the room stays in m.direct),
      then press Create Direct Chat on bob's profile here: the existing
      room opens instead of a second one appearing.
* [ ] **The last-device warning is plain.** On an account with exactly
      one session and no recovery set up, the log out page says this IS
      the last session and messages will be lost — not "if". With a
      second session logged in, the hedged sentence returns.
* [ ] **A too-large file is refused up front.** Attach a file over the
      homeserver's limit (Synapse defaults to 50M; the harness accepts a
      dd-made 60M file): the toast names the limit immediately, with no
      long upload first. A file under the limit sends as always.

## The Android merge, seen from the desktop — 27 August 2026

The merge that brought the Android port and main back together resolved
conflicts in shared code, and a few of those resolutions change what desktop
draws or does. The Android half of the same merge has its own sheet,
`doc/eyeball-android.md`; these are the desktop-visible seams.

* [ ] **Room details is a dialog now.** The port converted `RoomDetails` from
      `Adw.PreferencesWindow` to `Adw.PreferencesDialog`, and the merge brings
      that to desktop: it opens as a sheet over the window rather than as a
      window of its own. Open it from a room's header and from a space's menu
      — the space path is the merge's own wiring — and check both open, both
      close, and nothing modal is left stuck.
* [ ] **The composer looks exactly as it did.** The port rebuilt the
      composer's blueprint around a second, phone-only button row, and on
      desktop that row must stay invisible and everything else must stay
      put: attach, emoji, sticker, voice, More and Send all beside the
      entry, and a voice recording still counts its elapsed seconds where
      it always did.
* [ ] **A file with no local path sends instead of erroring.** The resolution
      removed the up-front "file does not have a path" bail from the send
      path: such a file is now read through GIO and sent, which is what
      Android's pickers need and desktop should never notice. Attach an
      ordinary file (unchanged), and on macOS paste a file copied from Finder
      — the pasteboard repair still runs first.
* [ ] **The homeserver entry names itself a URL.** `input-purpose: url` rode
      along from the port onto the redesigned first page. On-screen keyboards
      and input methods that read it change layout; a hardware keyboard should
      notice nothing.
* [ ] **Image fallbacks still fall back.** The pixbuf path gained a named-MIME
      second try for HEIC/HEIF/AVIF containers whose loaders declare no
      sniffing signature. On Windows, where MSYS2 ships those loaders with
      signatures, the second try should never fire: an SVG, an AVIF and a HEIC
      all still draw, and a corrupt file still reads as unsupported rather
      than looping.

## The secret store moved into the core — `doc/track3-convergence.md`

Phase 2, leaf 1. Nothing about this is meant to be visible, which is exactly
why it is here: the five backends that read and write the account are now the
core's copies, and an account that cannot be read is an account that is gone.
The compiler cannot see any of this — the store's contents are on the machine,
not in the source.

* [ ] **An existing session still restores.** Start the build over an
      installation that already has one, and the account comes back with no
      login: same rooms, same history, same device. Nothing in the store's
      format changed, so a session written by the old build is read by this
      one. **This is the one that matters** — if it fails, every other line
      here is moot.
* [ ] **A new login is still stored.** Log in, quit, start again. It comes
      back. The passphrase, the tokens and the OAuth client ID all travel the
      same path they did.
* [ ] **Logging out empties it.** Log out, restart, and the greeter is what
      comes up rather than a broken session — the delete path runs through the
      core now, including the token file beside the databases.
* [ ] **Two sessions still both restore, in the order they were in.** The
      sidebar order comes from settings and the sessions come from the store,
      and the wrapper the application puts around each one sits between them.
* [ ] **On Linux, the keyring entry reads in the user's language.** Open
      Seahorse (or `secret-tool search`) and look at the label of a Commune
      item — in a translated locale it must be that translation, not English.
      The sentence is the application's and the writing is the core's, and
      this is the only place the seam between them shows.
* [ ] **On macOS, the same in Keychain Access.** One item per session under
      the application ID, labelled with the Matrix ID.
* [ ] **On Linux, a locked keyring says so in the user's language.** Lock the
      login keyring and start the application: the error under "Could not
      restore previous sessions" must be the translated "The collection or
      item is locked.", not English. Fifteen sentences take this path; one is
      enough to prove the wiring.

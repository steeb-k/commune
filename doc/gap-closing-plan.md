# Plan: closing the feature gaps, without disappearing into one of them

This is the working plan for the round of work that started on 23 August 2026,
against the gaps `doc/client-comparison.html` and `doc/spec-gaps.html` name. It
records the intended route and the reasoning behind the ordering, and it is
updated as decisions change — the same footing as `doc/macos-plan.md`.

**Read the "Where this got to" section before starting any of this work**, rather
than re-deriving the state from the ledgers. The ledgers say what Commune can do;
only this file says why the order is what it is, and it carries five repricings
that came from reading the pinned SDK rather than the ledgers. Losing them costs
an afternoon of investigation and probably a worse order.

It lived in `~/.claude/plans/` until 24 August 2026, which is not a git
repository and not visible from another machine. The per-feature ledgers under
`doc/` remain the record of what exists; this is the record of where the work is
going.

## Context

`doc/client-comparison.html` and `doc/spec-gaps.html` name six rows where the field has
settled and Commune hasn't (spaces, signing up, pinned messages, polls, presence, threads)
and fourteen spec modules absent or partial. You want to close the big ones —
registration/reset, spaces, threads — and bank the cheap ones — pinned messages, presence,
peeking — without spending three months on one feature and shipping nothing else.

The plan below is ordered by that constraint, not by value. Every numbered item is a commit
to `main` that on its own moves a row in one of the two ledgers, so the pages stay honest
between big features rather than only after them.

Two constraints from you shape it: stay close to the real Client-Server API, accept things
that are _going_ to be spec, refuse anything experimental that cannot be a tack-on the way
the GIF picker is. And: commits land straight on `main`, no feature branch.

Three decisions taken, so they are not reopened mid-round: **the order below stands**; **polls
are out entirely** (MSC3381 is a proposal, and eight years unstandardised is not "going to be
spec"); **threads go all three slices**, since only the third flips the row and lets
`hide_threaded_events` go true.

---

## Where this got to

**Rounds 1 and 2 are done, committed and seen on screen.** Round 3 (spaces,
sliced, with peeking) is under way — **slices 1 and 2 are committed and none of
either has been seen on screen beyond the sidebar section.**

**Round 3 slice 1 (item 5) is done and committed** — `69002a14`, with the
masthead refresh in `1427c68c`. clippy, `cargo test` and `hooks/checks-bin` all
pass; the three HTML pages moved in the feature commit and are republished. On
screen the user has confirmed **only that the Spaces section appears**; the rest
of `doc/eyeball-tests.md`'s new Spaces section is unstruck. What is in it:

* `SidebarSectionName::Space`, between `ServerNotice` and `Favorite`, mapped both
  ways to `RoomCategory::Space` and displayed as "Spaces"; `into_target_room_category`
  still returns `None`, so the section takes no drops.
* `SidebarItemList`: `TOP_LEVEL_ITEMS_COUNT` 10 → 11 and the hand-written offsets
  in `section_from_room_category` shifted. `SectionsExpanded::default()` gains
  `Space`, so an **existing** session starts it collapsed and a fresh one expanded.
* `RoomCategory::Space::can_change_to(Left)` is now true, merged into the
  `ServerNotice` arm, and the sidebar row's context menu gets a `leave` action.
  A deliberate departure from the old `=> false`: otherwise a space row could
  never be got rid of.
* `ContentSpace` (`src/session_view/space.rs` + `.blp`, stack page `space`),
  because the `_ =>` arm drew an empty timeline forever. `header_bars()` 6 → 7.
* Sidebar row: `view-grid-symbolic` + a "Space" tooltip, **ahead of** the
  server-notice / call / direct branches.
* Explore: the `room_types` filter dropped; `RemoteRoom` gained `is-space` off
  `RoomSummary::room_type`; `PublicRoomRow` shows a "Space" marker. The last two
  are not in this plan — listing spaces without saying which ones they are is
  half a change.

**One bug was found only by launching it**, and it is the exact failure mode
`doc/eyeball-tests.md` exists for: `ContentSpace` did not call
`TemplateCallbacks::bind_template_callbacks`, so the two `$string_not_empty`
bindings in `space.blp` failed to resolve and the application **aborted at
startup**. clippy, the tests and `checks-bin` all passed with that in the tree.
`doc/spaces.md`'s rebase guide records it.

**Round 3 slice 2 (item 6) is done and committed** — `41136eeb`, masthead
refresh in `d47f99ed`, preceded by the harness work in `6e4079ab`. `Test Space`
had **no children** — the restricted rooms only named it in their join rule — so
`seed_space_children()` writes the `m.space.child` events for five rooms, one
per case a listing must draw. Nothing in slice 2 has been seen on screen.

**The plan was wrong about where the work goes, and the ledger records why.**
Item 6 says to lift the `limit: 1` in
`RemoteRoom::load_data_from_space_hierarchy`. That method is the fallback for
`load_data_from_summary` and only runs on a `404` from the MSC3266 summary
endpoint, which Synapse implements — so widening it would have shipped an empty
page on every homeserver worth testing against. The fallback is untouched and
the listing is a new object, `SpaceChildren`
(`src/session/remote/space_children.rs`): `/hierarchy` at `max_depth: 1`, the
space's own chunk skipped but read first for the `via` servers its
`m.space.child` events carry, ten batches of twenty and then a visible
truncation notice. Rows are `PublicRoomRow`, now `pub(super)` and reused
verbatim — the name is wrong for a space child and stays wrong, because
renaming it would put every future upstream change to that file into a path
conflict and its strings are referenced by path in thirty-odd catalogues.
`RemoteRoom` also keeps `world_readable` now, which item 7 needs.

**Item 7 (peeking) is done and committed**, the same day. It went the way the
plan's own correction 4 said it would — a model over raw events off
`/messages`, a cut-down row, a page on `RoomPreviewDialog` — plus one thing the
plan did not name: the affordance also hangs off `PublicRoomRow`, so it is in
Explore and on every space page row, which is where people decide. Two
departures worth knowing:

* **Nothing on the preview fetches media.** No avatars, no images, plain text
  only. `AvatarImageSafetySetting::MediaPreviews` exists for exactly this
  worry and takes a `Room`, which a peeked room does not have; rather than
  reimplement the setting against nothing, the preview fetches nothing.
* **Most previews fail, and that is Synapse.** It will not peek a room it does
  not already have, so a `world_readable` room on another homeserver answers
  with an error. The page says so and the failure is logged at `debug`, not
  `warn`.

Two faults came straight back from looking at it (`43d26d02`): the preview page
carried no room name, because the dialog's heading is fixed at _Join a Room_ and
only the details page draws its own; and it opened on the oldest of the twenty
messages rather than the newest. Both were invisible to every check that is not
a pair of eyes.

The harness needed one more room: `Readable Room` is `world_readable` but alice
made it, so she is in it and gets the room rather than a preview.
`seed_peekable_room()` adds bob's `Peekable Room` behind a marker of its own.

**Item 8 (spaces slice 3) is being built in two commits, not one.** The plan
names three jobs under it and the first two are separable from the third, so:

* **The picker, and authoring a restricted rule** — done. `SpacePickerDialog`
  (`src/components/dialogs/space_picker.rs`) filters the room list with the
  sidebar's own `RoomCategoryFilter`, which had to be exported. `_Who Can Join_`
  offers _Members of a Space_ to any room whose version carries the rule rather
  than only to one already restricted, and `compute_join_rule` now returns
  `Option<MatrixJoinRule>` — the plan's own warning was right, the `None` arm
  became reachable, and falling back to a rule the person did not pick is worse
  than refusing to save. This closed the restricted join rule row on its own.
* **Writing `m.space.child`** — done. `add_room_to_space`
  (`src/session/room/spaces.rs`) writes the child in the space and the parent
  in the room, and the two are not equals: the child decides whether the space
  contains the room and is allowed to fail loudly; the parent needs power in
  the **room** rather than in the space, so a failure there is a warning and
  the operation still succeeds. `canonical` is left false, because it claims to
  be the room's main space and nothing here knows whether it has one. Offered
  as _Add to Space…_ in a new _Spaces_ group on the room details general page,
  with the picker set to `SpaceRequirement::CanHoldRooms` — a different
  question from the join rule editor's, which needs no power in the space at
  all. This unblocks `image-packs.md` Phase 8.

Two things worth knowing before the second half:

* The picker answers through a `futures_channel::oneshot`, not
  `utils::OneshotNotifier`, which requires `T: Send` and so cannot carry a
  `Room`.
* Knocking over a restricted rule needs room version 10, two versions later
  than the restricted rule itself. `update_knock_sensitive` gates on
  `knock_restricted_join_rule` now.

**Round 3 is finished, and then the module was.** The plan's three slices left
three things behind — a space could not be made, a room could not be taken back
out of one, and `m.space.parent` was written and never read. On the user's
instruction those were built rather than logged, in one commit past the plan:

* **Making a space** is `creation_content: {"type": "m.space"}`, plus two
  things the spec does not ask for and every client does: the encryption switch
  hidden, and `events_default` raised to 100 so a space is not a room with an
  invisible timeline anybody can post into.
* **Taking a room out** writes `m.space.child` with nothing in it. Matrix has
  no way to delete a state event and a child with no `via` is not a child.
* **Which spaces hold a room** follows the spec's asymmetry: the space's own
  child event settles it; the room's parent claim counts only when whoever
  wrote it could have written the child, checked against that space's power
  levels. Only joined spaces can answer, which is the protocol and not a gap.
* `suggested` is read and drawn; `order` is applied by the server.

Then the last interface decision was taken rather than left: **a subspace
expands in place on the space page**, drawing the rooms inside it underneath.
Four options were put up — expand in place, a space as a sidebar filter, a
sidebar tree, or leave it — and the first was chosen: it finishes the browsing
story without reshaping what the sidebar is.

That changed the fetch. `/hierarchy` is asked without `max_depth` now, so one
walk carries the whole tree and opening a subspace costs no request. The rows
are built only once the walk finishes, because `GtkTreeListModel` asks whether
a row can be opened exactly once and remembers the answer — a subspace whose
chunk had not arrived would have been a leaf for good. Each row carries the
spaces walked through to reach it, because a Matrix hierarchy is a graph and
`A → B → A` would otherwise open forever.

Spaces is in the implemented column of `spec-gaps.html`, and what is left in
`spaces.md` under _Not done_ is two annotations the specification calls
optional and nobody has asked to write.

**The whole eyeball sheet was run on 25 August 2026**, and it is the reason the
State column below stopped saying "unseen". 154 checks that had never been
looked at were struck in one sitting, which covers everything round 3 built
plus the two rounds before it that had been left at "done, unseen". Two checks
failed, both in slice 3, and both were the same kind of fault as the ones the
sheet caught the day before — invisible to every automated check, obvious the
moment somebody used it:

* **The encryption switch stayed on screen for a private space.** The condition
  is "private and not a space", and it had been written in two places: the
  visibility half as a `visible` binding in `create_room_dialog.blp`, the kind
  half as a `set_visible` in `update_kind`. A binding re-asserts itself from
  its own sources, so the Rust half was overwritten and the switch only went
  when _Public_ was picked. Both halves are in the binding now, which is the
  general lesson: **a property with a binding on it has one author, and it is
  the template.**
* **A room removed from a space kept its row for half a minute.** The
  _Spaces_ group re-read the whole list after the change, and `parent_spaces`
  answers from the local state store, which does not carry the write until it
  comes back down the sync. Against a slow homeserver the re-read answered with
  the state as it was before the button was pressed, so the row sat there next
  to a toast saying it had gone. Adding had the same fault and nobody had
  noticed, because the check for it says to reopen the page. Both are corrected
  in the list directly now, on the strength of the homeserver having accepted
  the write.

The tear-down checks under the **Destructive** lines were done too — leaving a
space, dragging one to _Historical_, taking a room back out and putting it
back. Both faults were fixed the same day and a second pass over the sheet
found all 200 checks good, so **`doc/eyeball-tests.md` has nothing unstruck in
it for the first time**. Rounds 1 to 3 are seen, and the next thing that draws
will be the only unseen line in that file.

One note for the next run: the sheet's report counts button presses rather than
checks, so a check done and not clicked reads as "not looked at". That produced
a phantom "eleven not looked at" on the first pass, and the number should not
be read as a list of what was skipped.

**Round 4 slice 1 (item 9) is built — a thread announces itself.** The
"N replies" chip sits under a thread root's content in every timeline the
ordinary message row draws, fed by `Event::thread_summary()` reading the
`ThreadSummary` the SDK already puts on every message item. Three things worth
knowing beyond what `doc/threads.md` records:

* **No request is involved anywhere.** The summary comes from the bundled
  `m.thread` aggregation under `unsigned.m.relations`, extracted in
  `TimelineEvent::new` on every event built from raw — so the chip works on
  old roots the moment they scroll into view, and updates live through
  `Event`'s unconditional `item-changed` emission when the event cache
  recomputes the count.
* **The chip is a passive `Gtk.Box` on purpose.** Slice 2 builds the thread
  view; a button that does nothing until then would read as broken. Two
  visibility rules: `num_replies == 0` (an all-redacted thread) hides it, and
  the compact content formats hide it under exactly the condition that hides
  the reaction list.
* **`thread-symbolic.svg` is hand-drawn** — nothing shippable across the three
  platforms carries a thread glyph. `seed_thread()` in
  `testing/local-homeserver.sh` gives alice a three-reply thread in Invite
  Room behind its own `thread_root` marker.

The pages moved in the commit: threads left the Absent column of
`spec-gaps.html` for Partial (six absent, three partial now), and the
comparison page's grade deliberately stays at 0 — a chip satisfies none of
the four things that row measures — with only its note updated. `AGENTS.md`'s
ledger list turned out never to have picked up round 3 (`spaces.md`,
`peeking.md`); both were added alongside `threads.md`.

**Round 4 slice 2 (item 10) is built — a thread can be read and written.**
The plan said "a side sheet or subpage"; the code says neither, and the code
is right: the room history already swaps its displayed timeline for a focused
one and back, so the thread view is `Timeline::new_threaded` set as the
displayed timeline, with an `Adw.Banner` (_Viewing a thread_ /
_Back to All Messages_) naming the state. Every row is the real `EventRow` —
context menus, reactions, editing all work in a thread for free. Beyond the
plan's own list:

* **Composer routing is one closure**: `$compose_timeline` in the room
  history's template picks the displayed timeline when it is a thread, the
  room's live timeline otherwise. A thread timeline receives its own local
  echoes, which is what makes composing into it directly possible at all.
* **Receipts travel through the timeline that shows them.**
  `Room::send_receipt`'s body moved to `Timeline::send_receipt`; the room
  history sends `Read` receipts through the displayed timeline, so the SDK
  stamps thread receipts in a thread. `FullyRead` is never sent from a
  thread — it is the room's marker and a thread event may sit far back in
  room order.
* **Drafts are per-thread and server-side** — the SDK's composer-draft API
  takes a thread root, so `ComposerState` carries one and the toolbar's
  states map is keyed by `(room, thread root)`. A draft typed for the room
  can no longer be sent into a thread by accident.
* **`hide_threaded_events` is on** for the live focus and the focused
  timeline's `Automatic` mode, in the same commit — the plan's ordering rule
  held. Threaded replies left the main timeline the moment they had somewhere
  to be read.
* The chip became a button (`room-history.show-thread` with the root as
  target) and the context menu gained _View Thread_.

**Round 4 slice 3 (item 11) is built — the thread list, and the row flips.**
It went exactly the way item 11 said: `ThreadList`
(`src/session/room/thread_list.rs`) wraps the SDK's `ThreadListService` the
way `RoomSearch` wraps its endpoint — a `gio::ListStore` mirroring the
service's `VectorDiff`s, `load_more()` paginating `/threads`, live updates
arriving as `Set` diffs that replace a row wholesale. The view is the search
page's shape (stack, cut-down rows, paginate near the bottom), reached from
an always-visible header toggle; activating a row closes the list and opens
the thread view from slice 2, so the list finds and the view reads. The
pinned and threads toggles put each other out, and the slice-2 banner is not
revealed over the list. Thread subscriptions (MSC4306) stayed out, as
decided. `seed_thread()` seeds a second thread so the list has two rows in a
known order. One SDK wrinkle: `ThreadListService` does not implement `Debug`,
so the model's imp writes it by hand — the `VisualMediaRowModel` precedent.

**Round 4 is finished.** Threads moved into the implemented column of
`spec-gaps.html` and to a full mark on the comparison page; with polls out by
decision, "Everyone but us" is down to that single off-spec row.

**Next: round 5 — push rules and email on the account.** Before scoping it,
re-read its entry in the decided rounds below and `doc/sdk-unused.md`: much
of `NotificationSettings` already has call sites, so check what the settings
UI actually draws first — the round may be smaller than a round.

**Decided 26 August 2026 — what follows round 4.** The rest of the board was
walked and the next rounds settled, so they are not re-derived later:

* **Round 5 — push rules and email on the account.** Editable push rules are
  stable spec and cheaper than they look: the pinned SDK carries a
  `NotificationSettings` API (per-room modes, keyword rules, mention toggles),
  so the work is settings UI. **Repriced again by the 26 August SDK audit
  (`doc/sdk-unused.md`): much of `NotificationSettings` — keywords included —
  already has call sites.** Before scoping, check what the settings UI
  actually draws; the round may be smaller than a round. Third-party
  identifiers are **email only** — MSISDN verification needs SMS
  infrastructure almost no homeserver has — and
  reuse the request-token half password reset built plus `AuthDialog`'s UIAA
  for `/account/3pid/add`. Know before starting: on an OAuth homeserver
  (including matrix.org, the default) 3PIDs are managed in the browser page we
  already link to, so the in-app UI only ever appears on password-auth
  servers, the same reachability as the _Forgot Password?_ link.
* **Round 6 — voice message recording (MSC3245), mutual rooms, policy
  servers.** Voice recording is the one MSC admitted past the stance: the
  playback half already ships, every maintained peer has the other half, and
  recording is a genuine tack-on — a composer control and the extra fields on
  an `m.audio` send. Mutual rooms is one endpoint
  (`GET /_matrix/client/v1/mutual_rooms`) and a profile-dialog section that
  **no client on the comparison page has**. Policy servers was standardised in
  v1.16 and nobody has picked it up either — read `m.room.policy`, mark
  spam-checked events.
* **Round 7 — invite by email.** The highest-value gap left anywhere on the
  board (`spec-gaps.html` ranks it second after threads): the invite subpage
  takes Matrix IDs only, so the person deciding whether to try Matrix at all
  is the one person unreachable. It costs an identity-server flow, a small
  module of its own, which is why it sits after the cheap wins rather than
  before them. _Repriced by the SDK audit: `Room::invite_user_by_3pid()` is
  ready-made, so the module of its own is identity-server configuration UI,
  not protocol work._
* **Round 8 — finish what we already claim.** From the 26 August SDK audit
  (`doc/sdk-unused.md`), three things where the feature exists and the SDK
  computed the missing half all along: **per-message encryption shields**
  (`EventTimelineItem::get_shield()` — no per-message trust indicator exists
  today — plus reading `UtdCause` instead of a generic placeholder);
  **retry/discard for failed sends** (`SendHandle::{unwedge, abort}` off the
  send queue that is already on); and **knock notifications** — answering a
  knock works today through the members page, standard membership calls, but
  nothing surfaces that a knock is waiting
  (`Room::subscribe_to_knock_requests`, `mark_as_seen`). Attachment captions
  and upload progress ride along if the round has room.
* **Round 9, optional — tag order, moderation policy lists.** Arbitrary tags
  and `order` close a partial row every client on the page sits at ◐ on;
  policy lists pair with policy servers as the moderation story and matter to
  anyone running a public space. The audit's smaller wins —
  `is_last_device()` before logout, `get_dm_room()` before creating a DM, a
  storage settings page — are the same size and slot in wherever a round
  runs short.
* **2.0 — sliding sync, and only sliding sync.** MSC4186 was **accepted into
  the spec on 3 July 2026**, so the "off spec" rejection below is out of date;
  what stands is the size. It is not a tack-on: the SDK's sliding-sync path is
  the `SyncService`/`RoomListService` stack, a different architecture from the
  classic-sync session core, so migrating reshapes the room list, the sidebar's
  data source and the sync lifecycle. Two things to decide deliberately when
  the round is planned: **presence regresses** — simplified sliding sync
  carries no presence, so round 1's feature goes dark until the spec grows an
  extension — and fork divergence is **not** a reason to wait: `doc/fork.md`
  says this is a permanent fork and feature parity with upstream is not a
  goal. In sliding sync's favour, the SDK's maintained, exercised path _is_
  that stack; classic sync is its legacy path.
* **Watch list, not 2.0 — QR sign-in (MSC4108) and MatrixRTC (MSC4143).**
  Both verified 26 August 2026: MSC4108 is marked "rework in progress" with a
  competing rendezvous transport (MSC4388) unsettled, and only works against
  OAuth homeservers running MAS; MSC4143 cannot enter FCP for want of a
  qualifying implementation, and its transport rides MSC4195, a LiveKit
  backend that is Element infrastructure in all but name. Neither is "going
  to be spec" on any near horizon. Revisit if either reaches FCP.

The three HTML ledgers did not move with round 1 and were caught up afterwards —
pinned messages and presence marked as shipped in `client-comparison.html`, both
modules moved into the implemented column of `spec-gaps.html` (Pinned Events had
never been counted on that page at all), and the stale commit counts in
`upstream-defects.html` refreshed. `AGENTS.md` now carries the rule, and round 2
moved all three pages in the feature commits themselves. **Do not repeat the
drift: they go in the feature's own commit.**

| | Commit | State |
| --- | --- | --- |
| Ledger corrections | `5bc1b156` | done |
| 1. Pinned messages | `730310e3`, `e93670d4` | done, seen |
| 2. Presence | `052e5f37`, `b6570c23` | done, seen |
| Ledger catch-up | `44a0aedb`, `54563b65` | done |
| 3. Decouple `AuthDialog` | `59dcc516` | done |
| 3. Register | `73294320` | done, seen |
| 3. Token and terms stages | `0bddedb0` | done, seen |
| 4. Password reset | `b5d4ae74` | done, seen as far as a server without SMTP allows |
| Homeserver default | `5d24e432` | done, seen |
| 5. Spaces, slice 1 | `69002a14`, `1427c68c` | done, seen |
| Space children in the harness | `6e4079ab` | done |
| 6. Spaces, slice 2 | `41136eeb`, `d47f99ed` | done, seen |
| 7. Peeking | `ee2d0272`, `48e21dc1`, `43d26d02`, `3916a716` | done, seen, negatives included |
| 8a. Space picker, restricted rule | `b60baf66`, `640b1576` | done, seen |
| 8b. `m.space.child` | `37d318aa`, `9db20bfa` | done, seen; two faults, fixed and re-seen |
| Finishing the module | `b3deab0d`, `aec0710a` | done, seen; two faults, fixed and re-seen |
| Subspaces expand in place | `ce984576`, `21f89afe` | done, seen |
| 9. Threads, slice 1 | `fabc003f` | done, not yet seen |
| 10. Threads, slice 2 | `6a2699d9` | done, not yet seen |
| 11. Threads, slice 3 | `9c85de03` | done, not yet seen |

**Round 2 came out slightly differently from the plan, and the code is right:**

* The greeter's action is `login.create-account`, a widget action on `Login`,
  not the `app.create-account` the dead button named. Everything it touches is
  inside that widget.
* `m.login.registration_token` and `m.login.terms` got native pages, which the
  plan left on the fallback page. `AuthDialog::page()` takes the whole
  `UiaaInfo` now, because the terms policies are in the flow's params.
* **Reset has no OAuth branch and does not need one.** The plan said to mirror
  the native/OAuth split from `deactivate_account_subpage.rs`. The
  _Forgot Password?_ link lives on the password login page, which a homeserver
  with the OAuth 2.0 API never shows — the branch is unreachable, not missing.
* `utils::password` came out of it: three pages drew the same password meter,
  and the change-password page now shares the new one. Watch that page when
  checking round 2.
* `testing/local-homeserver.sh signup open|token|off` is new; `signup token` is
  the only way to see the token page locally. The terms page has **no harness at
  all** and has never been drawn.

**After round 2, on the user's request (`5d24e432`): the homeserver page offers
`matrix.org` by default**, with the old entry behind an "Another Homeserver"
row. Two consequences to know before touching login again, both in
`doc/registration.md`: the local harness is now reached through the second row,
and `matrix.org` delegates authentication (`auth_metadata` answers 200), so on
the default homeserver the password login page — and with it the
_Forgot Password?_ link — never appears. That page's eight eyeball checks are
the only unseen ones left from all of this.

**Two things below were built differently from what this plan says. The plan is
wrong, not the code — do not "fix" them back.**

* **Pinned messages is a header toggle, not a banner.** Four banners already
  stack over the timeline, and a banner's button is one-way where the pinned
  list takes over the whole stack and needs a way back. `doc/pinned-messages.md`
  has the argument.
* **The presence sharing switch defaults ON, not off.** This plan assumed
  publishing was "a wasted request with a privacy cost". It is not a request —
  `set_presence` is a query parameter on `/sync` that defaults to `online` when
  omitted, so this fork has published presence on every sync it has ever made.
  Defaulting the switch off would have silently changed what other people see.
  `doc/presence.md` has the argument.

A third thing turned up on the way: `m.room.server_acl` was never in
`show_in_timeline`'s state allow-list, so the sentence the ACL editor wrote for
it had never once been drawn. Fixed alongside pinned messages; `doc/server-acls.md`
records it.

`doc/eyeball-tests.md` is the running list of what has been built and not yet
looked at. Add to it in the same commit as the feature; strike an entry only
when the user reports what they saw.

---

## Five things the ledgers get wrong

These came out of reading the pinned SDK (`matrix-sdk` rev `db02d6c`, `Cargo.toml:82-110`)
against the claims. They reprice most of the plan, in both directions.

**1. Threads are UI work, not protocol work.** The pinned `matrix-sdk-ui` already carries the
whole module: `TimelineFocus::Thread { root_event_id }` (`timeline/mod.rs:143`), a
`ThreadListService` with live updates and pagination (`timeline/thread_list_service.rs`),
`ThreadSummary` with `num_replies` and per-thread read receipts on every message item
(`event_item/content/msg_like.rs:51-84`), `Room::list_threads` for the `/threads` endpoint
(`matrix-sdk/src/room/mod.rs:4252`), and `hide_threaded_events` on the live focus. Per-thread
receipts are inferred from the focus (`infer_thread_for_read_receipt`, `timeline/mod.rs:768`).
Threading is a _stable_ CS-API module, not an MSC. The `XL` was measured before all this
landed; it is an `L` now, and it slices cleanly.

**2. We are not corrupting anyone's threads today.** Both pages say "a reply into a thread is
sent as a plain rich reply". Not true on this SDK pin: `Timeline::send_reply` calls
`infer_reply` (`timeline/mod.rs:434-480`), which passes `EnforceThread::MaybeThreaded` outside
a threaded focus, and `MaybeThreaded` _forwards_ an existing thread relation
(`matrix-sdk/src/room/reply.rs:71-73`). Replying to a threaded message already sends
`m.thread`. The threads gap is entirely presentational — which removes the one urgent reason
to start there, and is a correction both pages need now.

**3. Registration is much cheaper than `L`.** The hard part of registration is UIAA, and UIAA
is already built and shipping: `src/components/dialogs/auth/mod.rs` runs the full stage loop,
handles `Password` and `Dummy` natively, and routes _every other stage_ to the spec's own
`GET /_matrix/client/v3/auth/{authType}/fallback/web` page (`fallback_url()`, `:387-432`).
That is the standard's answer to recaptcha, terms and email verification — no MSC, no invented
UI. `MatrixAuth::register()` even sets the session from the response
(`matrix-sdk/src/authentication/matrix/mod.rs:604`), so `Login::create_session()` works
unchanged. A dead `_Create Account` button is already sitting in `src/login/greeter.blp:115-125`,
wired to an `app.create-account` action that exists nowhere. One real blocker: `AuthDialog` is
`construct_only`-bound to a logged-in `Session`.

**4. Peeking is not an easy win — it is the most expensive thing on your easy list.** Every
`Timeline` in the tree is built from a `Room` GObject built from a `matrix_sdk::Room`, and
`matrix_sdk::Room` only exists for rooms sync knows about (`src/session/room_list/mod.rs:224-280`).
For a room we have not joined there is no `Room`, so the whole `Timeline` / `Event` /
`MessageRow` stack is unavailable and a peeked timeline needs a bespoke model over
`Raw<AnyTimelineEvent>` plus a cut-down row widget. There is no `Client::peek` in the SDK.
`spec-gaps.html` already grades this `M`; the comparison page's framing as a near-miss is
what misleads. It is real work — worth doing, but not a palate cleanser.

**5. Presence is cheaper than `M`, and lower value than you think.** Cheaper because every
avatar in the app binds through one `AvatarData` (`src/components/avatar/data.rs:17-24`) and
one `Avatar` widget (`src/components/avatar/mod.blp:4-11`), so an overlay badge is _one_
widget change reaching all twenty-odd sites. Lower value because `spec-gaps.html` grades it
Low itself and most homeservers keep it off. One snag: the pinned SDK loads a `PresenceEvent`
into every `RoomMember` and then gives no public accessor
(`matrix-sdk-base/src/room/members.rs:253` — the field is `pub(crate)`), so read it from
`client.state_store().get_presence_event()` or an `add_event_handler` on `PresenceEvent`
instead.

Not a correction but worth recording: **polls are dropped by our own filter, not by the SDK.**
`show_in_timeline` (`src/session/room/timeline/mod.rs:1344-1394`) is an allow-list ending in
`_ => false`, so `m.poll.start` never reaches the timeline, while the SDK already parses it
into `MsgLikeKind::Poll(PollState)` with a `results()` accessor
(`event_item/content/polls.rs:37-190`). MSC3381 is off spec and stays out on your stance; the
cost is recorded at the end in case you ever want it.

---

## The shape: eleven commits, four rounds, never two big things in a row

| Round | Items | Rows moved |
| --- | --- | --- |
| 1 — easy wins | Pinned messages, Presence | 2 |
| 2 — highest value | Registration, Password reset | 1 (the largest peer gap in the table) |
| 3 — spaces, sliced | Show, Browse, Peek, Picker | 3 (spaces, restricted rules, peeking) + image-pack Phase 8 unblocked |
| 4 — threads, sliced | Summary, View, List | 1 |

After round 3, "Everyone but us" is down from six rows to one (threads, plus off-spec polls).
Peeking rides inside round 3 because it and space-browsing share `RemoteRoom` and the same
discarded `world_readable` flag — doing them adjacent is cheaper than doing them apart.

---

## Round 1 — the two genuinely cheap wins

### 1. Pinned messages — `doc/pinned-messages.md`

8 of 9 peers, and the only unambiguous easy win in the table. The plumbing exists and is
pointed at one room, exactly as the page says.

* **Read precedent:** `Room::imp::update_active_server_notice`
  (`src/session/room/mod.rs:634-697`) reads `matrix_room.pinned_event_ids()` (synchronous, off
  cached `RoomInfo`), memoises against `server_notice_pinned_ids`, walks newest-first with
  `load_or_fetch_event`, and is driven from `update_with_room_info()` (`:1656-1663`). Copy that
  shape into a sibling reader — do not generalise the existing one: it collapses the list to
  one string by msgtype, and the spec requires notices keep their own UI
  (`doc/server-notices.md:88-93`).
* **Writes are free:** `Room::pin_event` / `unpin_event` / `load_pinned_events`
  (`matrix-sdk/src/room/mod.rs:4529,4555,4085`).
* **The pinned view is free:** `timeline_builder().with_focus(TimelineFocus::PinnedEvents)`
  reuses the entire existing `Timeline` / `Event` / `MessageRow` stack, exactly as
  `Timeline::new_focused` already builds a second timeline
  (`src/session/room/timeline/mod.rs:1152-1187`). Note the focus is now a **unit variant**
  (`timeline/mod.rs:146`) — there is no `with_pinned_events` builder and `PinnedEventsRoom`
  is gone. Pagination is refused on it (`timeline/pagination.rs:63`), which is fine.
* **New work:** a permission helper for `StateEventType::RoomPinnedEvents` in
  `src/session/room/permissions.rs` (none exists; follow the per-type helpers at `:377-618`);
  a Pin/Unpin action in `event_actions/context_menu.blp` + `group.rs:139`; a banner (a fourth
  `Adw.Banner` alongside `room_history/mod.blp:120,125,129,145`) with a counter and
  click-to-jump via the existing `RoomHistory::focus_on_event()` (`mod.rs:1622-1636`); and a
  pinned-list stack page modelled on the search page (`room_history/search/`).
* **Also:** allow `m.room.pinned_events` through `show_in_timeline`'s state allow-list
  (`timeline/mod.rs:1411-1417`) and render it in
  `room_history/state/content.rs:97-130` — the SDK exposes `RoomPinnedEventsChange`
  (`event_item/mod.rs:49`), so "X pinned a message" becomes a sentence instead of nothing.

### 2. Presence — `doc/presence.md`

7 of 9 peers. Small because of the avatar chokepoint; do the read side and stop.

* Read from `client.state_store().get_presence_event(user_id)`, or an `add_event_handler`
  on `PresenceEvent` (presence is a top-level `/sync` field and the app is on classic sync,
  `src/session/mod.rs:486-540`, so it arrives). Do **not** wait on `RoomMember::presence()` —
  it is `pub(crate)` upstream.
* Hang `presence`, `last_active_ago`, `currently_active`, `status_msg` on `PillSource`
  (`src/components/pill/source.rs:33-48`) so `User`, `Member` and every avatar get them at once.
  `Member` already has `latest-activity` (`src/session/room/member.rs:74-93`) as the analogue.
* One overlay badge in `src/components/avatar/mod.blp` reaches the message row, member list,
  user profile, sidebar, pills and the pickers. Exception: `OverlappingAvatars`
  (`src/components/avatar/overlapping.rs:230`) crops its children — leave the badge off there.
* Push updates through `MemberList::update_member()` (`src/session/room/member_list.rs:308`).
* Degrade to _nothing_, not to "offline", when the server publishes nothing — that is the
  behaviour the comparison page credits the field with.
* **Assumption, stated rather than asked:** publishing our own presence
  (`Client::set_presence`, `client/mod.rs:828`) goes behind a setting defaulting to **off**.
  It is the half with a privacy cost and it is a wasted request on servers with presence
  disabled. Say so in the ledger.

---

## Round 2 — signing up and resetting a password

`doc/registration.md`. 8 of 9 peers, and the only gap that costs a person before they have
seen a timeline.

### 3. Decouple `AuthDialog`, then register

* **Refactor first, its own commit.** In `src/components/dialogs/auth/mod.rs`, add a
  construction path taking a `matrix_sdk::Client` plus `Option<OwnedUserId>`, and make
  `new(session)` a thin wrapper. Three places reach through the session: `authenticate` `:124`,
  `current_stage_auth_data` `:456-461`, `fallback_url` `:388,397`. `perform_uiaa`, `AuthState`,
  `ExtractUiaa`, the `OneshotNotifier` plumbing and the fallback page carry over untouched.
* Teach `AuthState::next` (`:558-588`) to prefer `RegistrationToken` and `Terms` alongside
  `Password | Sso | Dummy`, with native pages for those two (a text entry; a checkbox list of
  the policies the server returns). Recaptcha and email keep going to the spec's fallback page
  — that is the correct standard answer, not a shortcut.
* New `src/login/register_page.rs` + `.blp`, pushed into the existing `Adw.NavigationView`
  (`src/login/mod.rs:100`, tags at `:43-86`), reached by implementing `app.create-account` and
  unhiding `greeter.blp:115-125`. Reuse `LoginHomeserverPage` for server selection; reuse
  `validate_password` (`src/utils/matrix/mod.rs:79`) and the LevelBar/Revealer UI at
  `change_password_subpage.rs:70-151` verbatim.
* Debounced username availability via ruma's `get_username_availability`.
* Drive `client.matrix_auth().register(request)` through the dialog; on success call
  `Login::create_session()` unchanged (`src/login/mod.rs:425-471`).
* Extend `src/user_facing_error.rs` with `UserInUse`, `InvalidUsername`, `Exclusive`,
  `WeakPassword` — today `WeakPassword` is handled at one call site
  (`change_password_subpage.rs:199-207`) and the rest are unmapped.

### 4. Password reset

* Mirror the native/OAuth split `deactivate_account_subpage.rs` already establishes
  (`:143-180`): on an OAuth server send the user to `account_management_url_with_action(...)`;
  on a native server do it in-app.
* In-app: ruma's `request_password_change_token_via_email`, then `change_password` with
  `AuthData::ThirdPartyIdentifier`. A "Forgot password?" link on `LoginMethodPage`.
* **Scope note:** this is the one place needing something that does not exist at all — no 3PID
  layer anywhere in `src/`. Reset needs only the request-token half, not the whole "Email and
  phone on the account" row. Build the half; leave that row for later.

---

## Round 3 — spaces in three slices, with peeking folded in

`doc/spaces.md`, `doc/peeking.md`. The row that reads as one huge feature is actually three,
each flipping something.

### 5. Slice 1 — stop hiding them

The entire current behaviour is one `return None`. `RoomCategory::Space` exists
(`src/session/room/category.rs:11-39`, produced at `src/session/room/mod.rs:717-720`) and
`SidebarSectionName::from_room_category` drops it
(`src/session/sidebar_data/section/name.rs:46`).

* Add `Space` to `SidebarSectionName` and a section to `SidebarItemList`
  (`src/session/sidebar_data/item_list.rs:53-81`). **Watch the hand-written indices** —
  `TOP_LEVEL_ITEMS_COUNT = 10` (`:11`) and the offsets at `:188-199` are literals.
  `SidebarSectionName` is serde-persisted in session settings, so keep the kebab-case names stable.
* `Display for RoomCategory` currently `unimplemented!()`s on `Space`
  (`category.rs:130-138`); that panic goes away with the section name.
* A space icon in `SidebarRoomRow::update_room_icon` (`sidebar/room_row.rs:251-281`), which
  already does this for server notices, calls and DMs.
* A `ContentPage::Space` arm in `src/session_view/content.rs:243-274` — today the `_ =>` arm at
  `:259` drops a selected space into `RoomHistory`, which is the visible bug. `header_bars()`
  (`:284-293`) returns a fixed-arity array that has to grow.
* Drop `RoomTypeFilter::Default` from the explore search (`explore/search.rs:278`) so the
  public directory stops hiding spaces too.

This alone takes the row from `·` to `◐` and is the smallest honest thing.

### 6. Slice 2 — browse and join a space

* The hierarchy call is written already: `load_data_from_space_hierarchy`
  (`src/session/remote/room.rs:357-424`) calls `get_hierarchy::v1` with `limit: 1` and throws
  the children away. Lift the limit, keep the chunks.
* Each chunk carries `summary: RoomSummary`, and `RemoteRoom::with_data` (`remote/room.rs:454`)
  takes exactly that — every child row is a `RemoteRoom` for free.
* **Also stop discarding `world_readable`** in `set_data()` (`remote/room.rs:273-287`). Ruma's
  `RoomSummary` carries it (`ruma-common/src/room.rs:398`); item 7 needs it and it is one line here.
* `RoomListRoomInfo` (`src/session/room_list/room_info.rs`) already answers "have I joined /
  am I joining this remote room", which is what a child row's View-vs-Join button needs. Join
  through `RoomList::join_by_id_or_alias` / `knock` (`room_list/mod.rs:543,552`), the same way
  `room_preview.rs:393-428` does.
* **One level of nesting only.** There is no `GtkTreeListModel` anywhere in the tree; draw it
  with the existing section expander (`sidebar_data/item.rs:77-114`). Arbitrary nesting means
  adopting `GtkTreeListModel` plus a `GtkTreeExpander` row type in `sidebar/row.rs:150-220` —
  defer that, and say in the ledger that it is deferred.
  _(Deferred at the time, built afterwards: subspaces expand in place on the
  space page. `GtkTreeListModel` came in there rather than in the sidebar,
  which is untouched.)_

### 7. Peek a `world_readable` room

Rides on slice 2's `world_readable` flag; both features are "look at a room you have not joined".

* Offer the peek only when `world_readable` is true, and fall back to today's behaviour
  verbatim otherwise — `RoomPreviewDialog` already has the "cannot be previewed… you can still
  try to join it" state (`room_preview.rs:299-351`).
* **The events cannot come through the normal stack** (correction 4). Send
  `get_message_events` (`/rooms/{id}/messages`) raw through `client.send(request)`, the pattern
  already used for `get_summary::v1` at `remote/room.rs:325-332`. Prefer it over
  `peeking::get_current_state::v3` (`/initialSync`), which ruma has but which is the legacy
  endpoint.
* Render into a new read-only page on `room_preview.blp` with a cut-down row widget. The only
  precedent for a timeline-ish list not backed by `matrix-sdk-ui` is `HistoryViewerTimeline`
  (`room_details/history_viewer/timeline.rs:175-201`) — same idea, but it still needs a `Room`,
  so this is a new lightweight model over `Raw<AnyTimelineEvent>`.
* Consider also hanging the peek affordance off the explore rows
  (`explore/public_room_row.rs:170-240`), which is the other place a person decides whether to join.

### 8. Slice 3 — a space picker, and `m.space.child`

* The picker is structurally `invite_subpage/` (`src/session_view/room_details/invite_subpage/`)
  with `Room`s filtered on `category == RoomCategory::Space` instead of users;
  `RoomCategoryFilter` already does that filtering shape.
* Wire it into the restricted join rule editor. Per `doc/join-rules.md` this needs
  `membership_row.set_visible(true)` unconditionally (subject to the existing power-level gate
  at `join_rule_subpage.rs:221-232`), and `compute_join_rule` (`:352-382`) taking the allow list
  the _page_ holds rather than `current_restricted` read off the saved rule (`:243-249`). The
  ledger's own rebase note warns this makes the `None` arm reachable and wrong — handle it, and
  extend the unit tests at `:398-506`.
* Write `m.space.child` / `m.space.parent` to add a room to a space. Nothing in the tree reads
  or writes either event today.
* This also unblocks `doc/image-packs.md` Phase 8 (space pack inheritance, `:270`), which can
  follow as a separate small commit.

---

## Round 4 — threads in three slices

`doc/threads.md`. Last, because it is the largest UI job and — per correction 2 — the least
urgent: nothing is going out wrong today.

### 9. Slice 1 — see that a thread exists

Render the `ThreadSummary` already on every message item
(`event_item/content/msg_like.rs:51-69`) as an "N replies" chip. The row shell is a grid
(`message_row/mod.blp`) with avatar col 0, content col 1 row 1, reactions row 2 — the chip goes
at row 3. `thread_root` and `thread_summary` are reachable today from `Event::item().content()`
and are simply never read (`session/room/timeline/event/mod.rs:754-772` reads only `in_reply_to`).

### 10. Slice 2 — read a thread, and write into it

* Add `Timeline::new_threaded(room, root_event_id)` beside `new` and `new_focused`
  (`src/session/room/timeline/mod.rs:1152-1187`), mapping to
  `TimelineFocus::Thread { root_event_id }`. Both existing constructors already funnel through
  one `construct()`, so this is near-mechanical.
* Render it in a side sheet or subpage with the existing message rows.
* **Sending comes free.** `Timeline::send_reply` infers `EnforceThread::Threaded` from the focus
  (`timeline/mod.rs:434-450`) and `send()` picks up the thread relation too (`:367-378`), so the
  composer needs _routing_ — point `MessageToolbar` at the thread `Timeline`
  (`message_toolbar/mod.rs:769-830`) — not new send code. Per-thread read receipts likewise come
  free via `infer_thread_for_read_receipt` (`:768-800`).
* Only once this exists, flip `hide_threaded_events` to `true` on the live focus. Not before —
  hiding threaded events with nowhere to read them is strictly worse than inlining them.

### 11. Slice 3 — the thread list

Wrap `ThreadListService` (`timeline/thread_list_service.rs:148-314`) in a `gio::ListModel` the
way `RoomList` wraps its `IndexMap`. It yields `ThreadListItem` with sender profiles resolved
and content already parsed, updated live, paginated by `paginate()`. Thread subscriptions
(`Room::subscribe_thread`, `room/mod.rs:4316`) are MSC4306-era — leave them out.

---

## Off the plan, on purpose

* **Polls (MSC3381).** Off spec; out on your stance. Recording the cost since the page
  overstates it: one arm in `show_in_timeline` (`timeline/mod.rs:1373`) and one arm in
  `message_row/content.rs:295-338` rendering `MsgLikeKind::Poll(PollState)` via
  `PollState::results()`. Render-only puts nothing non-standard on the wire; voting would.
* **MatrixRTC (MSC4143), QR sign-in (MSC4108), widgets, thread subscriptions
  (MSC4306).** Off spec and none is a tack-on. Out — on the watch list above,
  revisit at FCP. _(This bullet originally also named voice recording and
  sliding sync; both were repriced on 26 August 2026 — see the decided rounds
  above. Sliding sync is accepted spec now and is the 2.0 feature; voice
  recording is round 6.)_
* **Guest access, OpenID.** On spec, and out anyway: guest access is an L for
  the worst value ratio on the board, and OpenID only proves identity to
  widgets and integration managers, which Commune does not host.
* ~~**Email and phone on the account, third-party invites, mutual rooms, policy
  servers, tag order, editable push rules.** On spec, low value; the natural
  next tier after round 4.~~ _All six are scheduled now, rounds 5 through 8
  above._

---

## Doc obligations

`AGENTS.md` requires a ledger under `doc/` per feature, kept current with each change:
`doc/pinned-messages.md`, `doc/presence.md`, `doc/registration.md`, `doc/spaces.md`,
`doc/peeking.md`, `doc/threads.md`. Each carries design decisions, integration points and a
rebase guide, like `doc/calls.md` — including what is _unwatched_, which is the convention
`doc/calls.md` set.

**All three HTML ledgers — `doc/client-comparison.html`, `doc/spec-gaps.html` and
`doc/upstream-defects.html` — get updated in the same commit as the feature that moves a row in
them, not at the end of a round.** This plan said "at the end of each round" and round 1 shipped
without them; they were brought up to date on 23 August 2026 in a separate commit, after the user
pointed it out. `AGENTS.md` now carries the rule, since a plan is not read by the next session and
`AGENTS.md` is. Corrections 2, 4 and the poll note went in before any code, in `5bc1b156`.

---

## Verification

Per commit, before saying anything works:

```sh
meson install -C _build                                    # builds and installs to ~/.local
CARGO_TARGET_DIR=~/.cache/fractal-target cargo clippy --all-targets
CARGO_TARGET_DIR=~/.cache/fractal-target cargo test -j 2 --bin commune
hooks/checks-bin                                           # the 15-check conformity suite
```

Give clippy and the tests their own `CARGO_TARGET_DIR` — pointing them at `_build/cargo-target`
makes the next `meson install` rebuild matrix-sdk-ui for nothing. New `.rs` files calling
`gettext`/`gettext_f` go into `po/POTFILES.in` alphabetically; new `.blp` files into
`src/ui-blueprint-resources.in`, also alphabetically.

**Nothing that draws is verified by any of that.** Every item ends with the app launched and
the window looked at. Building and installing is mine; launching and exercising is yours — I
hand over exact steps and wait rather than driving your live session.

Against a homeserver, per `doc/testing.md`:

* **Registration** must go to `testing/local-homeserver.sh` — it needs open registration and a
  registration token, and trying variants on a public server makes junk accounts. That is a
  small addition to the script.
* **Spaces** — the script already seeds `Test Space` plus `restricted` and `knock_restricted`
  rooms pointing at it (`:378-420`), which is exactly slice 3's case.
* **Presence** — Synapse ships it enabled, matrix.org has it off. Test locally, then confirm the
  degrade-to-nothing path against matrix.org.
* **Pinned messages, peeking, threads** — a matrix.org account cross-checked against Element,
  the way the calls work was done.

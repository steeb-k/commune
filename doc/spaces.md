# Spaces — downstream implementation notes

This file is the ledger for the Spaces module (`m.space`, and the
`/hierarchy` endpoint that goes with it): what the fork added, the decisions
behind it, and what to check when rebasing onto a new Fractal release. See
`fork.md` for why none of this goes upstream.

**All three slices have landed.** The module is still graded `◐` on
`spec-gaps.html` and that is deliberate: `m.space.parent` is written and never
read, and a room that has been put into a space cannot be taken out of one from
here. See _Not done_.

## Scope of slice 1 — stop hiding them

Upstream knows what a space is and then drops it on the floor. A joined space
is a `Room` in the room list like any other, gets `RoomCategory::Space`, and
then:

* `SidebarSectionName::from_room_category` returned `None` for it, so it
  belonged to no sidebar section and was never drawn;
* selecting one anyway — through a `matrix:` link, or the room search — fell
  through the `_ =>` arm of `Content::update_visible_child` into `RoomHistory`,
  which built a live timeline for a room that has no messages and drew an empty
  one forever;
* the public room directory was queried with `room_types: [Default]`, which is
  the filter's way of saying "rooms that are not spaces", so Explore could not
  find one either.

Slice 1 undoes those three. It moves the row from "absent" to "present and
inert": you can see your spaces, open them, leave them, and find new ones. It
cannot yet tell you what is _inside_ one.

## A space is a room, except for the four things it is not

`RoomCategory::Space` sits in the same enum as `Favorite` and `LowPriority`,
which are tags, and that is the trap. A space is not tagged; it is a room whose
`m.room.create` carries `type: m.space`. So:

* **It cannot be re-categorised.** `can_change_to` refuses every target but
  one. Dragging a space onto Favorites would try to write `m.favourite` onto
  it, which the server would accept and which would then fight with
  `is_space()` on every sync.
* **It can still be left.** `can_change_to(Left)` is now `true`, where the old
  code refused everything. This is a departure from how `Space` was written and
  it is deliberate: leaving is a membership change, not a tag, and a space that
  could not be left would sit in the sidebar with no way to remove it — the
  context menu would have offered nothing but _Report_. `TargetRoomCategory::Left`
  ends at `matrix_room.leave()`, which is correct for a space.
* **It is never a drop target.** `SidebarSectionName::Space::into_target_room_category`
  returns `None`, so the Spaces section takes no drags even though rooms can be
  dragged out of it — there is no "move this room into that space" here, and
  there will not be until `m.space.child` is written in slice 3.
* **It has no timeline worth drawing.** Hence the new page.

## The sidebar section, and the literals around it

`SidebarItemList` holds a fixed-length array and a hand-written index map, and
both had to be counted again by hand:

* `TOP_LEVEL_ITEMS_COUNT` went 10 → 11.
* `section_from_room_category` computes indices as `FIRST_ROOM_SECTION_INDEX + n`
  with `n` written out; everything after Server Notices shifted by one.

Neither is derived from the array, so a future section has to do the same
walk. The `n_items()` of the model is the constant, so getting it wrong is not
a compile error — it is a silently truncated or over-long sidebar.

The section sits between **Server Notices** and **Favorites**: after the two
things that demand attention (invites, the homeserver talking to you), before
the ordinary rooms. `SidebarSectionName` is serde-persisted in the session
settings under kebab-case names, so `Space` serialises as `space` and the
variant name must not be renamed casually. It was also added to
`SectionsExpanded::default()`, so an existing session — whose stored set has no
`space` in it — starts with the section **collapsed**, while a fresh session
starts expanded. That asymmetry is inherent to storing a set of what is open.

A knock-on: `Display for RoomCategory` goes through `from_room_category` and
`unimplemented!()`s when there is no section. `Space` used to hit that panic.
Only `Outdated` and `Ignored` can now, and nothing displays either.

## The space page

`ContentSpace` (`src/session_view/space.rs`) is a new stack page in `Content`,
modelled on `Invite`: header bar, avatar, name, canonical alias, topic. It
exists because the alternative was leaving the `_ =>` arm in place, and an
empty `RoomHistory` is worse than a page that says what it is.

Slice 1 carried one sentence admitting it could not list the rooms inside.
Slice 2 replaced that sentence with the list.

`Content::header_bars()` returns a fixed-arity array which grew 6 → 7. It feeds
one `GtkSizeGroup` so every header bar on screen is the same height; a page
left out of it is a page whose header bar jumps when the text scaling changes.

## Scope of slice 2 — look inside one

The space page now lists the rooms that are in the space, each with the same
row the public directory uses: _View_ if it has been joined, _Join_ if it has
not, _Request an Invite_ if it only takes knocks.

### The plan said to widen the call that was already there. That was wrong

`RemoteRoom::load_data_from_space_hierarchy` does call `get_hierarchy::v1`, and
it does ask for `limit: 1` and throw the children away, so widening it looks
like the whole job. It is not, because of _when_ it runs: it is the fallback
for `load_data_from_summary`, and it is only reached when the summary endpoint
answers `404`. Synapse implements MSC3266, so on any homeserver worth testing
against that method never executes. A space page hanging off it would have
shown nothing, on the servers people actually use, with no error.

So the fallback is left exactly as it is — `limit: 1` is correct for what it
does, which is fetch one room's summary — and the listing is a separate object,
`SpaceChildren` (`src/session/remote/space_children.rs`), which calls the same
endpoint on purpose.

### What `SpaceChildren` is

A `GListStore` of `RemoteRoom` plus a loading state, modelled on
`ExploreSearch`, which is the tree's existing answer to "a paginated list of
rooms nobody has joined". Every chunk of the response carries a
`summary: RoomSummary`, and `RemoteRoom::with_data` takes exactly that, so each
row is a `RemoteRoom` with no conversion in between.

Three things in it are decisions rather than mechanics:

* **`max_depth: 1`.** The endpoint walks the tree depth-first and will happily
  return grandchildren. There is nowhere to put them — see _one level only_
  below — so asking for them would mean drawing rooms in a flat list that are
  not in this space at all.
* **The space's own chunk is skipped, but read first.** The first room in the
  response is the space itself, and its `children_state` is the only place in
  the response that carries the `via` servers from each `m.space.child` event.
  Without them a room on another homeserver is unjoinable, so the events are
  read into a map before the rows that need them are built. The space's chunk
  only appears in the first batch, which is why the map is a field and not a
  local.
* **Ten batches of twenty, and then it stops.** A space can hold thousands of
  rooms and the endpoint paginates; something has to end the loop. When the
  cap is hit the list says so — `truncated_label`, _"This space holds more
  rooms than are listed here."_ — rather than quietly looking complete.

### The rows are `PublicRoomRow`, and the name stays wrong

A space child needs an avatar, a name, a topic, an alias, a member count, a
_Space_ marker and a View/Join button off `RoomListRoomInfo`. That is
`PublicRoomRow` down to the last widget, so the space page uses it, and
`explore/public_room_row.rs` is now `pub(super)`.

The rooms in a space are not necessarily public, so the name is now a lie, and
it stays a lie deliberately. Renaming the file would put every future upstream
change to it into a conflict git cannot resolve by path, and its strings are
referenced by path in thirty-odd `po/*.po` files. A wrong name is cheaper than
that, in a fork that rebases.

The `.explore .padded-button` and `.public-rooms row` rules are scoped to the
Explore page, so `.space-children` in `_session_view.scss` repeats them. If the
row ever grows a third home, that is the point to hoist them.

### One level only

A subspace is drawn as an ordinary row. Joined, its button says _View_, and
viewing it selects it in the sidebar, which lands on _its_ space page with
_its_ children. So the tree is walkable, one page at a time, without a tree
widget.

Real nesting — an expander per subspace, children drawn underneath — needs
`GtkTreeListModel` and a `GtkTreeExpander` row type, neither of which exists
anywhere in the tree. That is deferred, and it is deferred rather than
forgotten.

### `world_readable` is kept now

`RemoteRoom::set_data` used to drop it. It is one line and one property, and
peeking is what reads it — see `peeking.md`, which was built on top of it the
same day, and which also added `is-encrypted` from the same summary.

## Slice 3, first half — a space picker

`SpacePickerDialog` (`src/components/dialogs/space_picker.rs`) asks the person
which of their spaces they mean. It is a dialog rather than a subpage because
it has two callers already and neither shares a navigation stack with the
other: the restricted join rule editor (`doc/join-rules.md`) and — next — the
action that puts a room into a space.

* The model is the session's `RoomList` behind three filters:
  `RoomCategoryFilter` on `RoomCategory::Space`, a `GtkStringFilter` for the
  search box, and a `GtkCustomFilter` that leaves out one room the caller
  names. `RoomCategoryFilter` is the sidebar's own filter, which had to be
  exported from `session::sidebar_data`; it was written for exactly this shape
  of question and there was no reason to write a second one.
* Rows are plain `AdwActionRow`s with an `Avatar` prefix, bound with
  `bind_model`. No new row widget: a space has a name, an alias and a picture,
  and `AdwActionRow` draws all three.
* **The exclusion matters.** A room restricted to itself admits nobody new, and
  a space cannot be put inside itself. The caller passes the room and the
  picker drops it.
* The answer comes back through a `futures_channel::oneshot`, resolved either
  by activating a row or by `AdwDialogImpl::closed` — so dismissing the dialog
  releases the caller with `None` rather than leaving a future hanging.
  `utils::OneshotNotifier` could not be used: it requires `T: Send`, and a
  `Room` is a GObject.

## Slice 3, second half — putting a room into a space

`add_room_to_space` (`src/session/room/spaces.rs`) writes both halves of the
relationship, and they are not equally important.

* **`m.space.child`, in the space**, is the one that counts. `/hierarchy` is
  built from it; a space carrying no child event for a room does not contain
  that room, whatever the room claims. This one is allowed to fail loudly.
* **`m.space.parent`, in the room**, is how the room claims which space it
  belongs to. It needs power **in the room** rather than in the space — a
  different permission, often a different person — so a failure is a line in
  the log and nothing more. The room is in the space either way.

`canonical` is left `false` on the parent event. It means "this is the room's
main space", and nothing here knows whether the room already has one; claiming
it would be a guess with consequences for other clients.

The `via` lists are not interchangeable: the child's names servers that can
reach the **room**, the parent's names servers that can reach the **space**.
Both come from `matrix_sdk::Room::route()`, which already leaves out the
servers a room's ACL excludes.

### Where it is offered

A _Spaces_ group on the room details general page, with one row,
_Add to Space…_. Hidden for a direct chat, like the rest of that half of the
page.

The picker filters to spaces this account may write state in —
`SpaceRequirement::CanHoldRooms`, which asks the room's permissions for
`SendState(SpaceChild)`. That is the difference between this caller and the
join rule editor: **pointing a join rule at a space needs no power in the
space**, because the rule is state in the room being restricted. Offering a
space that would refuse the write is the kind of thing that produces a toast
instead of an answer.

## Explore stops filtering them out

`ExploreSearchData::as_request` sent `room_types: vec![RoomTypeFilter::Default]`.
Per the spec an **empty** list means no filtering, and `Default` means "rooms
with no `type`" — i.e. everything except spaces. Dropping the field is the whole
change.

That alone would list spaces indistinguishably from rooms, so `RemoteRoom`
gained an `is-space` property read from `RoomSummary::room_type`, and
`PublicRoomRow` shows a dimmed grid icon and the word _Space_ beside the member
count. Not in the plan for this slice; added because listing spaces without
saying which ones they are is a half-change, and the summary already carried
the answer.

`RemoteRoom::set_data` also gained `is-world-readable` in slice 2; see above.

## Files

| File | What |
| --- | --- |
| `src/session/sidebar_data/section/name.rs` | The `Space` section name and its two mappings |
| `src/session/sidebar_data/item_list.rs` | The section itself, `TOP_LEVEL_ITEMS_COUNT`, the index map |
| `src/session/session_settings.rs` | `Space` in the default expanded set |
| `src/session/room/category.rs` | `can_change_to` letting a space be left |
| `src/session_view/space.rs`, `space.blp` | The page |
| `src/session_view/content.rs`, `content.blp` | The `space` stack page and the routing arm |
| `src/session_view/sidebar/room_row.rs` | The `view-grid-symbolic` row icon |
| `src/session_view/sidebar/row.rs` | The `leave` action for a space |
| `src/session_view/explore/search.rs` | Dropping the `room_types` filter |
| `src/session/remote/room.rs` | `is-space` off `RoomSummary::room_type` |
| `src/session_view/explore/public_room_row.rs`, `.blp` | The _Space_ marker; made reusable in slice 2 |
| `src/session/remote/space_children.rs` | Slice 2: the `/hierarchy` listing |
| `data/resources/stylesheet/_session_view.scss` | Slice 2: `.space-children` |
| `src/components/dialogs/space_picker.rs`, `.blp` | Slice 3: the picker |
| `src/session/sidebar_data/section/mod.rs`, `sidebar_data/mod.rs` | Slice 3: exporting `RoomCategoryFilter` |
| `src/session/room/spaces.rs` | Slice 3: `m.space.child` and `m.space.parent` |
| `src/session_view/room_details/general_page.rs`, `.blp` | Slice 3: the _Spaces_ group |

## Rebase guide

1. **`TOP_LEVEL_ITEMS_COUNT` and the `FIRST_ROOM_SECTION_INDEX + n` map are the
   fragile pair.** If upstream adds or moves a sidebar section, both need
   counting again, and neither will fail to compile.
2. `RoomCategory` and `TargetRoomCategory` have several exhaustive `match`es
   across the tree. An upstream change that adds a variant will point at all of
   them; an upstream change that _reorders_ them will not, and `SidebarSectionName`
   derives `Ord` from declaration order for the serialised `BTreeSet`.
3. If upstream ever grows its own spaces support, the `_ =>` arm in
   `Content::update_visible_child` is where the two will collide.
4. `RoomTypeFilter` is no longer imported by `explore/search.rs`. If a merge
   brings the filter back, spaces vanish from Explore with no other symptom.
5. `TemplateCallbacks::bind_template_callbacks` must stay in `ContentSpace`'s
   `class_init` — `space.blp` uses `$string_not_empty` twice, and without the
   binding the template fails to build and the application aborts at startup.
   This is not caught by anything but launching it, and it did happen.
6. `explore/public_room_row.rs` is `pub(super)` and has a second caller now. An
   upstream change to what `set_room` expects breaks the space page too, and
   the compiler will only point at Explore's copy of the call.
7. `SpaceChildren` skips the first room in the `/hierarchy` response by
   comparing room IDs, not by position. If a server ever omits the space's own
   chunk the list still works; it just loses the `via` servers with it.
8. `add_room_to_space` treats a failed `m.space.parent` as a warning. If a
   merge makes it an error, adding a room to a space stops working for anybody
   who is not also an administrator of the room.

## Not done

* **Nesting deeper than one level.** A subspace is a row that opens its own
  page. Drawing its rooms underneath it needs `GtkTreeListModel` and
  `GtkTreeExpander`, which nothing in the tree uses yet.
* ~~Peeking a `world_readable` room.~~ Built on top of slice 2's flag; see
  `peeking.md`. `PublicRoomRow` carries the _Preview_ button, so it is on every
  space page row as well as in Explore.
* **The listing is not live.** `SpaceChildren` asks once, when the page is
  first given the space. A room added to the space while it is on screen does
  not appear, because `m.space.child` is a state event in a room whose timeline
  is not being watched. Reopening the space asks again — and so does _Try
  Again_ after a failure.
* **`m.space.parent` is written and never read.** A room does not say which
  spaces it is in anywhere in this client; the only way to see the
  relationship is to open the space and look at its list. That is the main
  reason the module is still graded partial.
* **A room cannot be taken out of a space.** Removing means redacting the
  `m.space.child`, or writing it with no `via`, and there is nowhere to ask
  for it: the space page's rows are `PublicRoomRow`, shared with Explore,
  where such a control would make no sense.
* **A space cannot be created.** Room creation does not offer
  `creation_content: {"type": "m.space"}`, so every space this client shows
  was made somewhere else.
* **The picker replaces a whole allow list.** A room restricted to several
  spaces keeps all of them until somebody picks a space, and then keeps one.
  Expressing "these three and not that one" needs a multi-select picker, and
  nothing has asked for it.
* **`image-packs.md` Phase 8 is unblocked now** — space pack inheritance was
  waiting on nothing but a way to know a room is in a space.
* **Space invites are ordinary invites.** An invite to a space gets
  `RoomCategory::Invited` and the ordinary `Invite` page, which says nothing
  about it being a space. Correct as far as it goes — accepting it lands the
  space in the Spaces section — but the page could say what it is.
* **No space ordering of our own.** `m.space.child` carries an `order` field.
  The listing takes the server's order, which the spec already defines as
  `order`, then `origin_server_ts`, then room ID — so this is right by
  accident until slice 3 writes the events.

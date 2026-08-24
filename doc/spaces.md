# Spaces — downstream implementation notes

This file is the ledger for the Spaces module (`m.space`, and the
`/hierarchy` endpoint that goes with it): what the fork added, the decisions
behind it, and what to check when rebasing onto a new Fractal release. See
`fork.md` for why none of this goes upstream.

**This feature is being built in three slices and only the first has landed.**
The second and third are named at the bottom under _Not done_, and the HTML
ledgers grade the row `◐`, not `●`, on purpose.

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

For slice 1 it also carries one sentence admitting it cannot list the rooms
inside. That sentence is what slice 2 replaces; it is there rather than absent
because a page with an avatar and nothing else reads as broken.

`Content::header_bars()` returns a fixed-arity array which grew 6 → 7. It feeds
one `GtkSizeGroup` so every header bar on screen is the same height; a page
left out of it is a page whose header bar jumps when the text scaling changes.

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

`RemoteRoom::set_data` still throws away `world_readable`. That is slice 2's
line to add, and peeking is what needs it.

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
| `src/session_view/explore/public_room_row.rs`, `.blp` | The _Space_ marker |

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

## Not done

* **Slice 2 — browsing a space.** The harness is ready for it:
  `seed_space_children()` puts five rooms in `Test Space` — one to view, one to
  join, a subspace and a `world_readable` room — since naming a space in a join
  rule is not the same as being a child of it, and nothing wrote
  `m.space.child` until then. `RemoteRoom::load_data_from_space_hierarchy`
  already calls `get_hierarchy::v1` with `limit: 1` and throws the children
  away; lifting the limit and keeping the chunks is the work, plus a child row
  with a View/Join button off `RoomListRoomInfo`. One level of nesting only —
  arbitrary depth means adopting `GtkTreeListModel` in the sidebar.
* **Peeking a `world_readable` room**, which rides on slice 2 keeping the
  `world_readable` flag `set_data` currently drops. See `peeking.md` when it
  exists.
* **Slice 3 — a space picker and `m.space.child`.** Nothing in the tree reads
  or writes `m.space.child` or `m.space.parent` today, so no room can be put
  into a space from here, and the restricted join rule editor still cannot name
  a space. That is also what unblocks `image-packs.md` Phase 8.
* **Space invites are ordinary invites.** An invite to a space gets
  `RoomCategory::Invited` and the ordinary `Invite` page, which says nothing
  about it being a space. Correct as far as it goes — accepting it lands the
  space in the Spaces section — but the page could say what it is.
* **No space ordering.** `m.space.child` carries an `order` field; without
  slice 3 there is nothing to order.

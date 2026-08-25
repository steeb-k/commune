# Join rules — downstream implementation notes

This file is the ledger for making a restricted room's join rule editable. See
`fork.md` for why none of this goes upstream.

## What was already there

Contrary to a first reading of the code, upstream **can** set a room to knock.
`join_rule_subpage.blp` has an "Allow Invite Requests" switch, and
`new_join_rule()` turns "Only Invited Users" plus that switch into
`MatrixJoinRule::Knock`. Knocking works in both directions and always did.

What could not be set was anything restricted:

```rust
// Before.
pub(crate) fn can_be_edited(self) -> bool {
    matches!(self, Self::Invite | Self::Public)
}
```

`JoinRuleValue::RoomMembership` was absent, so for a room restricted to a space
the Save button never enabled and `new_join_rule()` would have hit its
`unimplemented!()` arm if it ever ran. `KnockRestricted` was unreachable.

Worse than unreachable: `set_local_value()` made the knock switch _sensitive_
for `RoomMembership`. On a restricted room the switch moved when clicked and
nothing could ever be saved — a control that lies about what it does.

## What changed

`JoinRuleValue::RoomMembership` joins `can_be_edited()`, and the subpage gained
a third choice row, `membership_row`, titled "Members of {room}".

At first the row was **only shown when the room already had a restricted
rule**, because a restricted rule needs an allow list and building one means
picking a space, which this client could not do. The allow list could be
carried over but never authored.

That constraint is gone — see _Authoring one_ below.

## Authoring one

Round 3 slice 3 gave the client a space picker, so the row is offered to any
room whose version supports the rule at all, and a space can be chosen for it.

* `membership_row` is visible when `authorization.restricted_join_rule` is
  true, which is room version 8 and up. It no longer waits for the room to
  already be restricted, and its title is the static _Members of a Space_
  rather than the name of one.
* A second list box, `space_row`, appears under the choices when the membership
  rule is the selected one. Its subtitle is the space, and activating it opens
  `SpacePickerDialog`. Without permission to change the rule it still says
  which space, and stops pretending it can be changed — no arrow, not
  activatable.
* The room itself is excluded from the picker. A room restricted to itself
  admits nobody new.

### The allow list the page holds, not the one the room has

`compute_join_rule`'s third argument used to be `current_restricted`, read off
the room's saved rule. It is now `restricted`, which is:

* the space the user picked, as a one-entry allow list, **replacing** whatever
  was there; or
* the saved list verbatim, if they have not picked anything.

The second half matters more than it looks. A room restricted to three spaces
must come back from a knock-switch toggle with all three — dropping one would
quietly lock people out — so a list left alone is never rebuilt. The first half
is a deliberate loss: picking a space replaces the whole list, because a picker
that chooses one space cannot express "and also keep these two". A room with
several allowed spaces can still be left alone, and the ledger says so rather
than the interface pretending otherwise.

### `None` is no longer unreachable, so it is no longer a fallback

```rust
fn compute_join_rule(
    value: JoinRuleValue,
    can_knock: bool,
    restricted: Option<Restricted>,
) -> Option<MatrixJoinRule>
```

The old signature returned `MatrixJoinRule` and fell back to `Invite`/`Knock`
for the membership rule with no allow list. That was safe only because the UI
could not reach the state. It can now — select the rule, pick nothing — and
falling back would send a rule the person did not choose while the screen said
otherwise.

So it returns `None` there: not a rule. `update_changed` reads that as "no
change", which keeps _Save_ disabled until a space is chosen, and `save`
returns early. What must never happen is `Restricted` with an empty allow list:
a room nobody can join, which this page could not undo.

### Knocking arrived later than restricting

`knock_restricted` is room version 10; `restricted` is room version 8. A room
can be old enough for one and not the other, so `update_knock_sensitive` gates
the switch on `knock_restricted_join_rule` when the membership rule is
selected, and on nothing extra for the invite rule. The version notice at the
top of the page now shows for either shortfall.

## One word for it, and that word is access

Knocking had two vocabularies. The asking side said **invite** — _Request an
Invite_, _Invite Requests_, _Allow Invite Requests_, _Requested an Invite_ —
and the refusing side said **access**: _Deny Access to {user}?_, _Deny Access_,
_Could not deny access to user_. Both were upstream's, in the same feature, on
adjacent buttons.

Worse, the button that accepted a request said **_Invite_**, full stop, beside
one that said _Deny Request_. It is only ever shown for somebody who has
knocked, so "invite" was never the right word there: it named what the protocol
does — a knock is answered with an `m.room.member` invite — rather than what
the person is doing, which is answering a request.

It is all _access_ now, because that is what the person on either end is
thinking about, and because the deny side was already using it:

| Was | Is |
| --- | --- |
| Request an Invite | **Request Access** |
| Invite | **Accept Request** |
| Deny Request | Deny Request |
| Requested an Invite | **Requested Access** |
| Invite Request(s) | **Access Request(s)** |
| Allow Invite Requests | **Allow Access Requests** |

`SidebarSectionName::InviteRequest` keeps its name: the variant is serialised
into the session settings, and only what it displays changed.

The same words serve both directions on purpose. The sidebar section holds
rooms **you** have knocked on; the members page holds people who have knocked
on **yours**. "Access Requests" reads correctly either way, which "Invite
Requests" also did — that part was never the problem.

## The dead switch

`update_knock_sensitive()` replaces the inline sensitivity in
`set_local_value()`, and now gates on permission as well as on the rule:

```rust
self.knock_box.set_sensitive(rule_supports_knocking && self.can_change());
```

It is called from `set_local_value()` and from `update()`, because the first
returns early when the value has not changed and so would miss a permission
change arriving on its own.

## Integration points

* `session/room/join_rule.rs` — `can_be_edited()` gains `RoomMembership`.
  Nothing else in that file changed; `From<&MatrixJoinRule>` already folded
  both restricted variants onto `RoomMembership`, and `can_knock` already
  covered `KnockRestricted`.
* `session_view/room_details/join_rule_subpage.blp` — the `membership_row`
  choice, `visible: false` until the room turns out to be restricted.
* `session_view/room_details/join_rule_subpage.rs` — `compute_join_rule()` as a
  free function at module level, `update_space_row()`,
  `update_knock_sensitive()`, and a handler on the join rule's
  `display-name` notify, because the name of the space a restricted rule points
  at resolves asynchronously and the row has to follow it.
* `components/dialogs/space_picker.rs` — the picker itself. See `spaces.md`.
* `session/sidebar_data/section/room_category_filter.rs` — unchanged, but
  exported now, since the picker filters the room list the same way the sidebar
  does.

## Testing

`compute_join_rule()` is a free function precisely so it can be unit-tested
without a room, a session or a widget; the tests at the bottom of
`join_rule_subpage.rs` cover each rule, both switch positions, the round trip
through knocking, and the empty-allow-list fallback.

What they cannot cover is whether a homeserver accepts the result, and making a
restricted room by hand needs a space. `testing/local-homeserver.sh` seeds one
of each; see `testing.md`.

Two tests were added with the picker: one for an allow list of several spaces
surviving a knock-switch toggle whole, and one for the membership rule with no
space producing no rule at all.

## Rebasing

1. If upstream adds a space picker of its own, take theirs and delete
   `components/dialogs/space_picker.rs` — but keep `compute_join_rule`'s
   `Option` return, which is about what may be _sent_ rather than about how a
   space is chosen.
2. `compute_join_rule` returns `Option<MatrixJoinRule>`. A merge that restores
   the old signature reintroduces a fallback that sends a rule the person did
   not pick.
3. `RoomCategoryFilter` is exported from `session::sidebar_data` now. Upstream
   keeps it private to the sidebar; a merge that re-privatises it breaks the
   picker with a visibility error, which at least fails loudly.
4. **Fifteen strings about knocking were rewritten**, across seven files, from
   _invite_ to _access_. A merge takes upstream's wording back without
   complaint, since nothing about it fails to compile — the table above is the
   record of what they should say.

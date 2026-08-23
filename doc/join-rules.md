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

The row is **only shown when the room already has a restricted rule**. This is
the load-bearing constraint: a restricted rule needs an allow list, and building
one means picking a space, which means a space picker, which this client does
not have — spaces are recognised and then hidden (see the spec gap page). So the
allow list can be carried over but never authored. A room with no restriction
cannot be given one here.

That is why the computation is written as it is:

```rust
fn compute_join_rule(
    value: JoinRuleValue,
    can_knock: bool,
    current_restricted: Option<Restricted>,
) -> MatrixJoinRule
```

`current_restricted` is the allow list read back off the room's _saved_ rule,
not off anything the page holds. Turning the switch on rebuilds the rule as
`KnockRestricted` with that same list; turning it off rebuilds it as
`Restricted` with that same list. Nothing about who is allowed in changes when
the user is only changing whether people may knock.

The `None` arm exists for a state the UI does not reach, and matters anyway: it
falls back to `Invite`/`Knock` rather than emitting `Restricted` with an empty
allow list, which is a rule that admits nobody and cannot be undone from this
page.

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
  free function at module level, `update_membership_row()`,
  `update_knock_sensitive()`, and a handler on the join rule's
  `display-name` notify, because the name of the space a restricted rule points
  at resolves asynchronously and the row title has to follow it.

## Testing

`compute_join_rule()` is a free function precisely so it can be unit-tested
without a room, a session or a widget; the tests at the bottom of
`join_rule_subpage.rs` cover each rule, both switch positions, the round trip
through knocking, and the empty-allow-list fallback.

What they cannot cover is whether a homeserver accepts the result, and making a
restricted room by hand needs a space. `testing/local-homeserver.sh` seeds one
of each; see `testing.md`.

## Rebasing

If upstream adds a space picker, this whole approach is superseded: the row
stops needing to hide itself, and `compute_join_rule()`'s `None` arm stops being
unreachable-in-practice and becomes wrong. Take upstream's version whole.

# Notification rules — downstream implementation notes

Upstream Fractal already carried most of the editable push rules story, which
round 5 of `doc/gap-closing-plan.md` discovered before building anything: the
account and session switches, the three global defaults, keyword rules with
add and remove, and per-room modes on the room details page all predate the
fork's work here. What this ledger covers is the part that did not exist —
the predefined rules that apply across every room — and the reasoning that
keeps the rest as it is.

## Scope

A _Notify Me About_ group on Account Settings ▸ Notifications, between the
global defaults and the keywords, with four switches:

* **Mentions of My Name** — `.m.rule.is_user_mention`, the Matrix 1.7 rule.
* **Messages Addressing the Whole Room (@room)** — `.m.rule.is_room_mention`.
* **Room Invites** — `.m.rule.invite_for_me`.
* **Incoming Calls** — `.m.rule.call`, the one underride among overrides.

Together with what already shipped, this is the full surface every peer
offers: global defaults, keywords, per-room modes, and the per-category
rules. Raw rule editing (custom conditions, tweak actions, sounds) is not
offered anywhere in the field as UI and not here either.

## The SDK keeps the deprecated rules in step

`.m.rule.is_user_mention` and `.m.rule.is_room_mention` replaced
`.m.rule.contains_display_name`, `.m.rule.contains_user_name` and
`.m.rule.roomnotif` in Matrix 1.7, but older clients still read the old ones.
The SDK's `set_push_rule_enabled` special-cases the two new IDs and writes
the deprecated rules alongside them, so a toggle flipped here reads the same
from Element Classic. That is the reason the rules are set through
`NotificationSettings` rather than raw ruleset requests, and the reason this
fork did not have to know the deprecated IDs exist.

## Integration

`NotificationsSpecialRule` (an enum of the four) lives in
`src/session/notifications/notifications_settings.rs` beside the existing
global and per-room types, mapping each rule to its `RuleKind` and ID. The
`NotificationsSettings` object gains four read-only boolean properties,
refreshed by the same `subscribe_to_changes` stream that keeps the rest of
the page current — so a rule flipped from another client moves the switch
here. The page rows are `SwitchLoadingRow`s following the account switch's
shape: optimistic UI is avoided, the row goes insensitive with a spinner
while the request is out, and a failure toasts and snaps the switch back.

The group is only sensitive while account and session notifications are both
enabled, like the keywords group.

## Rebase guide

* Upstream owns `notifications_page.{rs,blp}` and
  `notifications_settings.rs`; the special-rules group and enum are additive
  blocks in each. A conflict is most likely in the page's
  `set_notifications_settings`, where the handler list grew by four.
* If upstream ever adds its own predefined-rule toggles, prefer theirs and
  fold `NotificationsSpecialRule` away.

## Not done

* Sounds and tweak actions on rules — no peer offers this as UI either.
* `.m.rule.suppress_notices` and `.m.rule.tombstone` toggles — nothing in
  the field exposes them, and a switch nobody understands is noise.
* Per-room keyword rules — the spec allows `room`-kind content rules; nobody
  ships UI for them.

# Knock requests — downstream implementation notes

Round 8 of `doc/gap-closing-plan.md`, third item, built 26 August 2026:
the banner over the room history that announces pending knocks now has a
memory. Upstream drew the banner off the member list — it revealed
whenever anybody's membership was `knock` and the account could act on it
— which meant it stood forever until every knock was answered, nagging
about requests already reviewed and deliberately left pending.

## The subscription, and being seen

The banner now listens to `Room::subscribe_to_knock_requests()`, the
SDK's stream that folds the member events and the seen-request list into
one answer, and counts only the requests not yet reviewed. Pressing
_View_ does two things: opens the members page on the knocking list, the
way it always did, and marks every counted request as seen
(`KnockRequest::mark_as_seen`) — viewing them is reviewing them. The
banner stands down until somebody new knocks; the requests themselves
stay on the members page, where accepting and declining always lived.

**The seen list is local to this device** — the SDK keeps it in its own
store, not in account data — so another client, or this one after a wiped
store, announces the same knocks again. That is the SDK's contract, and
it errs on the side of not missing a request.

The subscription is per displayed room, torn down and rebuilt as rooms
change; the SDK hands back a cleanup task with the stream, and both are
aborted together.

## Rebase guide

The whole feature is in `room_history/mod.rs`: `watch_knock_requests`,
the reveal logic in `update_pending_knocks`, and the marking in
`view_pending_knocks`. The banner widget and the members page are
upstream's. If upstream ever adopts the subscription itself, prefer
theirs and keep the marking-on-view behavior.

# Moderation policy lists — downstream implementation notes

Round 9 of `doc/gap-closing-plan.md`, second item, built 26 August 2026:
the `m.policy.rule.*` events are read and said. A policy room — the kind
Mjolnir and its successors write — used to render every rule as
"Unsupported event"; its history is now sentences.

## What is drawn

The three rule types joined the timeline's state allow-list beside
`m.room.policy`, and the SDK's state-change enum carries all three, so
the rendering is typed (`policy_rule_message` in
`room_history/state/content.rs`): who recommended banning the users,
rooms or servers matching which entity, and for what reason. A
recommendation other than `m.ban` — the only one the specification
defines — reads as "set a moderation rule" without claiming to know what
it asks for, and a redacted rule, which is how a rule is withdrawn, reads
as the removal.

## What stays out, and why

Subscribing to a list and applying it — hiding or ignoring the matched
users client-side — is deliberately not built. Applying a policy list is
a moderation action with teeth, the field does it through server-side
tooling (Mjolnir, Draupnir, policy servers), and a client quietly hiding
messages on a glob match is a decision this fork does not want to make
half-way. Authoring rules from a UI is the same story. What a client
owes its user is legibility — being able to read what a policy room
decided and who decided it — and that is what this ships. The row moves
from absent to partial, not to done, and honestly so.

## Rebase guide

One allow-list block in `timeline/mod.rs`, three match arms and two
functions in `state/content.rs`. If a later SDK reduces the rule change
to something richer, the arms follow; the sentences stay.

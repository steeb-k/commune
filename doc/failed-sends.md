# Failed sends — downstream implementation notes

Round 8 of `doc/gap-closing-plan.md`, second item, built 26 August 2026:
a message that failed to send can be sent again. Upstream carried the
send queue and drew the failure — the error icon on the row, and a
_Discard_ in the context menu that redacts the local echo — but a failed
message could only be thrown away, never retried, even though
`SendHandle::unwedge()` sat on every local echo.

## What was added

One context-menu entry, _Try Sending Again_, on a local echo in either
error state (`MessageState::RecoverableError` and `PermanentError` both:
retrying something the server will refuse again is harmless and the
person may have fixed the reason). It takes
`EventTimelineItem::local_echo_send_handle()` and calls `unwedge()`,
which puts the transaction back in the queue; the row's state stack then
tells the rest of the story the way it already did. _Discard_ is
untouched.

## What was left

`SendQueue::local_echoes()` — an outbox listing across rooms — stays
unused: the failed message is visible in the room it failed in, and a
global outbox page is a feature of its own for a round that wants it.
Upload progress and attachment captions, the audit's riders, also stay
for later.

## Rebase guide

The action is `event.retry-send` in `event_actions/group.rs` beside
`event.cancel-send`, and one menu item in `context_menu.blp`. If upstream
reshapes the actions group, those two are the whole feature.

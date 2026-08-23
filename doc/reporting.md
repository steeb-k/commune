# Reporting content — downstream implementation notes

This file is the ledger for reporting a room and reporting a user: design
decisions, every integration point into existing code, and what to check when
rebasing onto a new Fractal release. See `fork.md` for why none of this goes
upstream.

## Scope

The Client-Server API's Reporting Content module defines three endpoints.
Upstream implements one of them:

| | Endpoint | Before | Now |
| --- | --- | --- | --- |
| Event | `POST /rooms/{roomId}/report/{eventId}` | yes | yes, unchanged |
| Room | `POST /rooms/{roomId}/report` | no | yes |
| User | `POST /users/{userId}/report` | no | yes |

Reporting an event was already there, reached from a message's context menu.
The other two had no call site and no menu entry, so the safety surface of the
client stopped at a single message: there was no way to say "this whole room is
a spam room" or "this account is the problem", which is what someone reaching
for a report usually means.

## Where a report goes, and why the wording matters

A report is sent to the administrator of **our own** homeserver, not to the
server hosting the room or the user. That administrator can only act on things
they host; for anything else the report is a signal and nothing more. They also
cannot read an encrypted room.

So each dialog says what is actually transmitted, rather than implying that
something will be done about it:

* Room — "will send its unique ID to the administrator of your homeserver. The
  administrator will not be able to see the content of the room if it is
  encrypted."
* User — "will send their Matrix ID to the administrator of your homeserver.
  Only the reason you give here tells them what this user did."

The user wording carries the sharper caveat on purpose: `POST
/users/{userId}/report` transmits a user ID and a free-text reason and nothing
else. There is no event, no room, no evidence. If the reporter does not write
down what happened, the administrator receives a bare accusation. Saying so in
the dialog is the only place that fact can usefully appear.

## One dialog, three reports

`components/dialogs/message_dialogs.rs` grew `confirm_report_dialog`, which
holds the parts every report shares — the optional-reason entry row, the
Cancel/Report buttons, the destructive appearance:

```rust
pub(crate) async fn confirm_report_dialog(
    heading: String,
    body: String,
    parent: &impl IsA<gtk::Widget>,
) -> Option<String>
```

`None` means the user cancelled; `Some("")` means they confirmed without giving
a reason. Two wrappers on top of it, `confirm_report_room_dialog` and
`confirm_report_user_dialog`, supply the wording above.

The event report predates this and had built the same dialog inline in
`room_history/event_actions/group.rs`. It now goes through the shared helper,
so the three reports cannot drift apart in appearance. Its own heading and body
are unchanged, which keeps the existing translations valid.

One asymmetry to know about, because it is in the spec and not a choice:
`report_content` takes `Option<String>` — the reason may be omitted — while the
room and user endpoints take a required `String` that may be empty. The event
path therefore still filters an empty reason to `None`; the other two pass the
string through as it comes.

## Integration points

Model layer, both of them thin:

* `session/room/mod.rs` — `Room::report(reason)`, over the SDK's
  `matrix_room.report_room()`. Deliberately not gated on membership: the spec
  says the caller need not be joined, and reporting a room you were invited to
  and do not want is the main case.
* `session/user.rs` — `User::report(reason)`. The SDK has no wrapper for this
  one, so it sends `ruma::api::client::reporting::report_user::v3::Request`
  through `client.send()` directly, the same way `session/room/aliases.rs` and
  `session_view/explore/server_list.rs` reach endpoints the SDK does not wrap.

Three call sites:

* `session_view/room_history/mod.blp` and `mod.rs` — a `Report Room…` item in
  its own section of the room menu, action `room-history.report`. Enabled
  always, since any membership can be reported.
* `session_view/sidebar/mod.blp` and `row.rs` — the same item in the row
  context menu, action `room-row.report`. Registered outside the per-category
  match in `room_actions()`, so it is offered for every category including
  `Invited` and `Knocked`.
* `components/user_page.blp` and `user_page.rs` — a `Report to Administrator`
  row with a destructive `Report…` button, below the ignore row. Hidden for our
  own user, alongside the ignore row and by the same rule.

There is no reported state to display: the spec exposes no way to ask whether
an account has already been reported, so the row's title is static and the
result is a toast.

## Testing

Reporting cannot be exercised against a homeserver anyone else uses without
sending a real report to a real person. `testing/local-homeserver.sh` stands up
a throwaway Synapse where the administrator is an account you own; see
`testing.md`. Its `check` command confirms the server accepts both endpoints,
and `reports` prints what arrived.

## Rebasing

Upstream owns the event report. If it grows room or user reports of its own,
prefer its version and delete ours, keeping only whichever wording is more
honest about where a report goes.

The endpoints are recent — room reports are Matrix 1.13, user reports are 1.14
— so a homeserver that predates them answers `M_UNRECOGNIZED`. That surfaces as
the "Could not report…" toast, which is the correct outcome but tells the user
nothing about why. Worth revisiting if it turns out to be common; it is not
worth a version probe before then.

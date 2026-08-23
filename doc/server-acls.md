# Server ACLs — downstream implementation notes

This file is the ledger for `m.room.server_acl`: what the fork added, the
decisions behind it, and what to check when rebasing onto a new Fractal
release. See `fork.md` for why none of this goes upstream.

## Scope

* A subpage under the room details that shows and edits the ACL: the allowed
  servers, the blocked servers, and whether bare IP addresses count as servers.
* A line in the timeline for the event, instead of "An unsupported state event
  was received."

Upstream exposed exactly one thing about ACLs: the power level required to send
one, in the permissions subpage. The event itself was invisible and unreachable,
though it was already on `NON_REDACTABLE_EVENTS` so nobody could brick a room by
redacting it.

## An ACL is one event, so it is edited as one

Every other list in the details pages — room addresses, image packs — applies
each add and remove on its own, because each is a separate call. An ACL is not
like that: the allow list, the deny list and `allow_ip_literals` all live in a
single `m.room.server_acl`, and any send replaces the whole thing.

So the subpage keeps a draft, and `Save` sends it, like the join rule and
history visibility subpages. Two things fall out of that and neither is
incidental:

* **The whole ACL can be checked before it goes out.** Per-row saving would put
  a half-finished ACL live in the room between two clicks — and one of those
  intermediate states is fatal (see below).
* **Every edit is one event.** Building an ACL a row at a time would write one
  state event per row into everyone's timeline.

## The empty allow list is the trap

From the spec: `allow` defaults to an empty list, and an empty list **denies
every server**. It is not "no restriction", it is "nobody". A room in that state
cannot be fixed from the room, because no server is left to send the fix.

Two things guard it, and they are deliberately different in kind:

| State | What happens |
| --- | --- |
| `allow` is empty | `Save` refuses, with an error under the list |
| Our own server is not allowed, or is denied | Confirm dialog naming the server; goes through if confirmed |

The first is refused rather than confirmed because there is no legitimate reason
to send it — it is always a mistake. The second is severe but real: handing a
room over to another server is a thing an admin does on purpose, and it is
recoverable by anyone still in the room.

`check_acl()` returns the worse of the two when both apply, which is why the
order in it matters and why a test pins it.

## A room with no ACL starts from "everything allowed"

The consequence of the paragraph above is that a deny list on its own is
useless: adding `evil.example` to `deny` and leaving `allow` empty blocks
everyone, not just `evil.example`.

So `unrestricted_acl()` — `allow: ["*"]`, no deny, IP literals allowed — is the
baseline the page uses when the room has no `m.room.server_acl` at all. It is
what the room already does, expressed as an ACL, which makes two things work:

* Blocking one server is one action, not two.
* `changed` compares the draft against that baseline, so opening the page on a
  room with no ACL and leaving offers to save nothing. Without this, every
  visit would write an event.

## The ACL is not in the room info

`Room` gets most of its state from the SDK's `RoomInfo` stream, and the ACL is
not in it. `Room::server_acl()` reads the state event from the store on demand,
and the subpage watches for changes with its own event handler on
`SyncStateEvent<RoomServerAclEventContent>`, the same shape `Permissions` uses
for power levels. The handler's drop guard lives in the subpage, so it goes when
the page does.

A reload while there are unsaved edits keeps the edits and refreshes only the
remote side — see `load()`. Otherwise typing into the page while a sync arrives
would lose what was typed.

## Only the blocked list is spelled out in the timeline

`server_acl_message()` describes a change only when the deny list is the sole
thing that moved. That is the common moderation action, and it is the one that
can be said in one line: "Alice blocked evil.example from taking part in this
room."

Everything else — an allow list that changed, the IP literal switch, blocks in
both directions at once — falls back to "changed which servers can take part in
this room". Trying to phrase the general case produces a sentence no translator
can work with, and the event's source is one click away in the properties
dialog for anyone who needs the detail.

The `(true, true)` case matters more than it looks: a change to
`allow_ip_literals` alone leaves the deny diff empty in both directions, and
without the fallback it would read as though a server had been unblocked. There
is a test for exactly that.

## Files

New:

| File | What |
| --- | --- |
| `src/session_view/room_details/server_acl_subpage.rs`, `.blp` | The subpage, the two lists, the checks and their tests |

Integration points, which are where a rebase will conflict:

| File | Change |
| --- | --- |
| `src/session/room/mod.rs` | `server_acl()` and `set_server_acl()` |
| `src/session_view/room_details/mod.rs` | `SubpageName::ServerAcl` and its arm |
| `src/session_view/room_details/general_page.blp` | The "Server Access" row |
| `src/session_view/room_history/state/content.rs` | `RoomServerAcl` arm, `server_acl_message()` |
| `src/components/dialogs/message_dialogs.rs` | `confirm_exclude_own_server_dialog()` |
| `src/ui-blueprint-resources.in`, `po/POTFILES.in` | The new files |

## Rebase guide

1. The `_ =>` arm in `update_with_other_state()` is where upstream adds its own
   state events. Take theirs and re-add the `RoomServerAcl` arm above it rather
   than resolving the hunk whole — a lost arm is silent, it just goes back to
   "An unsupported state event was received."
2. `unrestricted_acl()` is load-bearing in two places, `reset()` and
   `update_changed()`. If it ever stops being `allow: ["*"]`, check both.
3. The subpage's event handler and `Permissions::init_power_levels()` are the
   same pattern. If the SDK changes `add_event_handler` or the drop guard, both
   move together.
4. `RoomServerAclEventContent::is_allowed()` does the glob matching for us,
   including the case folding and the IP literal rule. Do not reimplement it;
   the tests in `check_acl` lean on it being the same matcher the server uses.
5. The new `.rs` and `.blp` belong in `po/POTFILES.in` and
   `src/ui-blueprint-resources.in`. `hooks/checks-bin` catches this, but read
   its message with care: the two for the POTFILES check are swapped, so "Found
   N file(s) in POTFILES.in without translatable strings" in fact means those
   files _have_ translatable strings and are _missing_ from POTFILES.in.

## Not done

* Authoring an ACL for a room that has none is possible, but the page gives no
  hint that `allow: ["*"]` is what it starts from — the list simply shows `*`.
* No validation of what a server pattern looks like beyond trimming it and
  rejecting duplicates. A typo is accepted and only shows up as a server that
  cannot join.
* No preview of which servers currently in the room the draft would shut out.
  The member list is right there and it would be the most useful thing to add
  next, but it needs a per-server rollup of the members that does not exist.
* Reordering entries, which is invisible to the matcher anyway, and editing an
  entry in place — remove and re-add is the only way.
* Nothing warns when a room is not federated, where the whole page is moot.

# Server notices — downstream implementation notes

This file is the ledger for the Server Notices module: what the fork added, the
decisions behind it, and what to check when rebasing onto a new Fractal
release. See `fork.md` for why none of this goes upstream.

## Scope

The server notices room is how a homeserver talks to its own users in an
official capacity — most often to say it has crossed a limit and is refusing
new activity. Upstream rendered the messages and did nothing else with the
room.

* Recognise the room by its `m.server_notice` tag, and give it a
  `RoomCategory` and a sidebar section of its own, above every ordinary room.
* Show the notices that are still active — the pinned ones — as a banner over
  the timeline, with a button for the administrator's contact method.
* Ignore an `m.server_notice` message anywhere else, which the spec requires
  and which is the whole of the protection against someone impersonating the
  homeserver.
* Say what actually happened when the homeserver refuses to let the user out
  of the room.

## The tag is the only thing that identifies the room

> Clients can identify the server notices room by the `m.server_notice` tag on
> the room.

Not the sender, not the room name, not who created it. `update_category()`
checks the tag before it checks `m.favourite` and `m.lowpriority`, so a server
notices room stays in its own section even if the user has tagged it something
else. The three tags are not mutually exclusive on the wire; this client picks
one, and picks this one first, because pinning the room to the top is the
point.

**The tag is not one of the SDK's notable tags.** `RoomInfo` caches
`m.favourite` and `m.lowpriority` in a bitflag, and `Room::is_favourite()` and
`Room::is_low_priority()` read it synchronously. There is no equivalent for
`m.server_notice`, so `is_tagged_server_notice()` goes to
`matrix_room.tags()`, which reads the room account data out of the store and is
async. That is why `update_category()` is `async` all the way up.

It still updates reactively: the SDK's account-data processor calls
`on_room_info` for **every** `m.tag` event, whether or not a notable tag
changed, so the room info subscriber that drives `update_with_room_info()`
fires and the tag is re-read. The account data is saved before the room info
update is applied, so the read sees the same sync that woke it.

**An invite to the room does not carry the tag.** Room account data is not sent
for a room the user has only been invited to — the invite arrives as stripped
state and nothing else. So the server notices room looks like an ordinary
invite until it is accepted, and only then does it move into its own section.
There is nothing to be done about that at the client end; it is how the
mechanism is built.

## Ignoring the msgtype elsewhere is the security-relevant half

> Events with a `m.server_notice` `msgtype` outside of the server notice room
> must be ignored by clients.

Anybody can send `{"msgtype": "m.server_notice", "body": "..."}` into any room
they can talk in. Upstream rendered it with a warning icon and the same
presentation the real thing gets, so an ordinary user could put an
official-looking warning in a room. That is what this rule exists to stop.

The filter is in `show_in_timeline()`, which is the SDK timeline's event
filter, so such an event never becomes a timeline item at all. The filter runs
off the main thread and cannot ask a `Room` GObject for its category, so it
reads an `Arc<AtomicBool>` that the `Timeline` keeps:

* seeded in `init_matrix_timeline()` by awaiting `matrix_room.tags()` directly,
  because the room's category is loaded on an idle task and may not have
  arrived yet when the timeline is built;
* kept current by a `category-notify` handler on the room, so a tag that
  arrives later is honoured for events that arrive after it.

An event already filtered out stays filtered out until the timeline is rebuilt.
That only matters in the window between joining the notices room and the tag
being synced, and the direction is the safe one: the notice appears late rather
than a forgery appearing at all.

Notifications are a separate path and are left alone. `notification_body()`
renders an `m.server_notice` exactly like an `m.text` — sender name and body,
no official styling — so there is nothing there to impersonate with.

## Active notices are the pinned ones

> Active notices are represented by the pinned events in the server notices
> room. Server notice events pinned in that room should be shown to the user
> through special UI and not through the normal pinned events interface in the
> client.

Commune has no pinned events interface at all, so the second half is free. The
first half is `update_active_server_notice()`, which reads
`matrix_room.pinned_event_ids()` — synchronous, off the room info, so it is
driven by the same subscriber as everything else — loads each pinned event
newest first, and takes the first one whose msgtype is `m.server_notice`. Its
body and `admin_contact` become two properties on `Room`, and the banner in
`room_history/mod.blp` binds to them.

The pinned IDs are remembered in `server_notice_pinned_ids` so that the common
case — a room info update that changed something else — costs one vector
comparison rather than a round of event loads.

Outside the server notices room this is hard-wired to nothing, for the same
reason the timeline filter exists: a pinned `m.server_notice` in an ordinary
room is an event that must be ignored, and raising a banner off it would be
worse than rendering it in the timeline.

**The banner does not use markup.** `use-markup: false` on the `Adw.Banner`.
The body is a string the homeserver wrote and Pango markup in it would be
parsed. There is no formatted body to lose: the spec gives `m.server_notice`
a plain `body` and nothing else.

**The contact method is a URI from the server, so the schemes are limited.**
`is_openable_admin_contact()` allows `mailto:`, `http:`, `https:`, `tel:`,
`sms:`, `xmpp:` and `matrix:`, and the button is hidden for anything else.
Handing an arbitrary URI to `GtkUriLauncher` opens whatever handler is
registered for its scheme, which is a lot of surface to open because a server
asked. The notice itself is readable without the button.

## Leaving, and the error that says why

> The client must not expect to be able to reject an invite to join the server
> notices room. Attempting to reject the invite must result in a
> `M_CANNOT_LEAVE_SERVER_NOTICE_ROOM` error. Servers should not prevent the
> user leaving the room after joining […] however the same error code must be
> used if the server will prevent leaving the room.

So leaving is offered — the spec expects it to work — and the failure is
handled by name. `is_cannot_leave_server_notice_room()` in `session/room/mod.rs`
matches the error kind, and the three places that leave or decline a room use
it to say "Your homeserver does not allow leaving its server notices room"
instead of "Could not leave {room}". Without that, the user retries forever
against a server that will never say yes.

The category itself cannot be moved. `RoomCategory::ServerNotice::can_change_to`
allows only `Left`: the homeserver owns the `m.server_notice` tag, so marking
the room a favourite would be undone on the next sync, and the sidebar would
show a room bouncing between two sections. The three `set-*` actions in
`sidebar/row.rs` are each guarded on a category that is not this one, which is
why `ServerNotice` can join that match arm and still only get `leave` and the
read markers.

## Where the section sits

`SidebarSectionName::ServerNotice` is between `Invited` and `Favorite`, in the
enum and in `SidebarItemList`. Above every ordinary room, below the two things
that are waiting on the user to answer them — a verification request expires,
and an invite is someone standing at the door. A quota warning is urgent but it
is not a question.

`score_for_unread_room()` scores it 6, above `Invited`, so "go to the next
unread room" reaches it first. That is a different judgement from the section
order on purpose: once the user has asked to be taken somewhere, the most
important unread thing is the one to take them to.

`SectionsExpanded::default()` includes it. A session whose settings were
written before this build has a stored set without it, so the section starts
collapsed the first time it appears there, and one click fixes it for good.
Migrating the stored set was not worth a settings version.

## Files

Integration points, which are where a rebase will conflict:

| File | Change |
| --- | --- |
| `src/session/room/category.rs` | `RoomCategory::ServerNotice` and what it can change to |
| `src/session/room/mod.rs` | `is_tagged_server_notice()`, `update_active_server_notice()`, the two properties, `is_cannot_leave_server_notice_room()` |
| `src/session/room/timeline/mod.rs` | The `is_server_notice_room` flag and the msgtype rule in `show_in_timeline()` |
| `src/session/sidebar_data/section/name.rs` | The section name and its label |
| `src/session/sidebar_data/item_list.rs` | The section's place in the list, and `TOP_LEVEL_ITEMS_COUNT` |
| `src/session/session_settings.rs` | The section in `SectionsExpanded::default()` |
| `src/session_view/mod.rs` | `score_for_unread_room()` |
| `src/session_view/sidebar/room_row.rs` | The warning icon and the accessible label |
| `src/session_view/sidebar/row.rs` | The context menu and the leave error |
| `src/session_view/invite.rs` | The decline error |
| `src/session_view/room_history/mod.blp` | The banner |
| `src/session_view/room_history/mod.rs` | Its button label and what the button does |
| `testing/local-homeserver.sh` | `notice` and `limit`, and the check |

## Rebase guide

1. `RoomCategory` and `SidebarSectionName` are matched exhaustively in about a
   dozen places. The compiler finds all of them; the two that it cannot are
   `SidebarItemList::section_from_room_category()`, where the indices are
   written out by hand and must stay in step with the array in `constructed()`,
   and `TOP_LEVEL_ITEMS_COUNT`.
2. `show_in_timeline()` used to be a flat `matches!` over the supported
   msgtypes. It is now a `match` because `MessageType::ServerNotice` is
   conditional. A merge that restores the `matches!` puts the impersonation
   back without breaking the build.
3. `update_category()` became `async` at the point it started reading tags. If
   upstream makes it synchronous again, the tag read has to move.
4. If the SDK adds `m.server_notice` to `RoomNotableTags`, `is_tagged_server_notice()`
   collapses into a synchronous `matrix_room.is_server_notice()` and the async
   chain above it can go back.
5. The banner is the second `Adw.Banner` in the content box, before the
   verification info bar's neighbour. Order matters only for how they stack.

## Testing

Server notices cannot be provoked on somebody else's homeserver, so
`testing/local-homeserver.sh` does it:

* `notice` sends one to alice through Synapse's
  `POST /_synapse/admin/v1/send_server_notice` and joins her to the room. This
  exercises the tag, the section, the icon and the msgtype rule.
* `limit on` puts the server over its monthly active user limit, which makes
  Synapse send **and pin** a `m.server_notice.usage_limit_reached` notice of
  its own. That is what raises the banner. `limit off` lifts it again; Synapse
  unpins the notice the next time it looks at the account, so the banner goes
  a beat later rather than at once. While the limit is on, nobody on that
  server can send a message or register.
* `check` confirms the tag is on the room and says whether anything is pinned.

**The notice cannot be pinned by hand.** `@notices:localhost` is not a
registered user — Synapse speaks as it without ever giving it an account, and
there is no row for it in the `users` table — so the admin API cannot log in as
it, and the recipient of a notice is left at `users_default: -10` in a room
whose `state_default` is 50. Pinning is Synapse's to do, which is why `limit`
exists at all. Do not go looking for a shortcut; this was checked against
Synapse 1.159.0 on 22 August 2026.

Synapse's own notice carries `admin_contact` only if `admin_contact` is set in
`homeserver.yaml`; the generated configuration sets it, so the banner's button
appears. A notice that already exists is reused rather than resent, so a server
seeded before that line was added keeps a notice with `admin_contact: null` and
no button.

## Not done

* **Nothing at invite time.** The tag is invisible until the invite is
  accepted, so the invite is presented like any other. The stripped state does
  carry the room name the server chose, which is the only hint available.
* **One notice in the banner.** If a server pins several, the most recent is
  shown and the rest are readable in the timeline. No server is known to pin
  more than one.
* **`server_notice_type` and `limit_type` are not read.** Both are carried by
  the event and neither changes what is shown: there is one specified type,
  and its `body` already says what it means. Reading them would only let us
  ignore the server's own wording.
* **The banner cannot be dismissed.** It goes when the server unpins the
  notice, which is the server saying the thing is over. A dismissed banner
  would need somewhere to remember the dismissal, and it would be hiding the
  one message the homeserver considered worth interrupting for.
* **Non-notice events pinned in the room** are not shown at all, where the spec
  says they should be "shown just like any other pinned event in a room". There
  is no pinned events interface to show them in; when there is one, that is
  where they go.

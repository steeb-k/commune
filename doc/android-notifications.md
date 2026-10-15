# Message notifications on Android — downstream implementation notes

This file is the ledger for how the Kotlin application (`android-kotlin/`)
announces messages in the shade: one notification per room, carrying only the
newest message, with the rest of the unread counted above it. It covers what
was built, why, where it lives and what to check when rebasing. The GTK
application's notifications — one plain notification per message, no buttons —
are `src/` and are not described here; `doc/notifications.md` is the push
rules, which decide _whether_ the server counts a message, not how either
application shows it. See `fork.md` for why none of this goes upstream.

**Status: built on 15 October 2026 and never put in front of a device.** Not
the emulator, not the Pixel. What exists is the code read twice and a host
check of the core; no APK built from it had been produced on this machine when
this was written. The sync half — the sidebar's counts reaching the shade — can be
exercised on the emulator against `testing/local-homeserver.sh`. The push half
cannot: the emulator has no UnifiedPush distributor, so everything below that
starts in `Push.kt` waits for the Pixel with ntfy. The checks are in
`doc/eyeball-android.md` under the section dated 15 October 2026, written as
what to do and what to expect.

## The complaint

In the user's words, on 15 October 2026: swiping notifications away without
reading them meant the next notification showed the old messages before the
new ones; they wanted only the most recent message, with "…3 more" or similar
above it, the latest always in focus, "like any other messaging app".

The shape that produced it was the one built on 4 September: a
`Notification.MessagingStyle` per room that accumulated, each post reading the
shade's messages back out of `EXTRA_MESSAGES` and adding one, up to
twenty-five, oldest first. That shape was the visible half. The swiped
messages coming back was not a presentation problem at all, and there were two
causes under it.

## The two root causes

**Two announcers, one bookkeeping.** Messages reach the shade from two places.
`Notifier.update` mirrors the server's per-room notification counts on every
room-list update, posting the room's latest message when the count rises;
`Push.kt` posts from a UnifiedPush delivery while the app sleeps. The push
service posted and recorded nothing. When the user swiped its notification
away and the sync later caught up — the next push waking the process, or the
app being opened — `Notifier.update` saw a count above the last one it had
announced and posted the room's latest body, which was the very message the
user had just swiped. The next real message then arrived underneath it. That
is the "old before new". The sync path also only knew the latest _body_, not
which event it was, so it had nothing to tell "already shown" from "new"
except the text-and-sender of the newest entry still in the shade — which a
swipe removes.

**The account key was empty at startup.** The bookkeeping was keyed
`"$accountKey|$roomId"`, and `Notifier.accountKey` was set only by
`CommuneState.refreshAccounts()`, which runs on login, logout and the account
switcher, not at launch. So the record lived under two key families, `""` and
the session's, and every switch between them read "known" as zero and
re-announced the latest body of every unread room.

## The design

**One record, keyed by event, consulted by both sources.** `AnnouncedMessages`
in `MessageNotifications.kt` is that record, and its `claim()` is the only
place the rule lives: a message is announced if and only if its event ID has
not been seen for the room **and** its timestamp is not more than a minute
(`SKEW_MS`) behind the newest message already announced there. Claiming
records the ID and moves the timestamp in the same synchronised step, because
the two sources run on different threads. Within the minute the event ID alone
decides, which is the whole of the rule in Element X and Signal; the timestamp
is only there to drop a genuinely stale preview — a room list restored from
the store that still names a latest message from before the shade moved on —
since the badge already counts it and the newest message is what the user
asked to have in focus. The minute is for clocks: the timestamp is the origin
server's, and in a federated room two servers stamp within seconds of each
other, so a strict order would have counted a message from the slower server
in the badge and never shown it. The order the first draft of this had was
strict, and that consequence is why it is not.

The record lives in the existing `"notified"` SharedPreferences file, under
`"$account|$roomId|ts"`, `"$account|$roomId|ids"` (the last fifty event IDs,
newline-joined; older ones are caught by the timestamp) and
`"$account|$roomId|known"`. The keys of the bookkeeping it replaced have no
field suffix, cannot collide with these, and are left lying there harmlessly
rather than migrated.

**A swipe needs no bookkeeping.** Everything posted was claimed before it was
posted, so what the user dismissed is already in the record and cannot be
raised again. That is why there is no `deleteIntent`: the fix for the
complaint is that the swipe is never the thing being remembered.

**The account is asked for, not remembered.** Both callers take
`app.sessionUserId()` at the moment they need it — `Notifier.update` at the
top of each pass, the push thread after its fetch, which is what waits for the
session to exist. `Notifier.accountKey` and the line in `refreshAccounts()`
that set it are gone. An account switch needs nothing cleared: the keys simply
differ.

**The shade shows the newest message only.** `postRoomMessage` still builds a
`MessagingStyle` — the platform's conversation shape, with the avatar, the
sender, the group title and the lock-screen stand-in exactly as before — but
with one message in it. The rest is a count in the conversation title, "3
more" in a direct chat (the platform names the sender on the message line,
and with the name in the title too the bundled row read "bob · 2 more: bob
r2 m3"), "weekend plans · 3 more" in a room, and the plain name when there is
no rest. The subtext was the first place the count went,
and the emulator showed why it cannot be: once Android bundles two of the
application's notifications into a group — two rooms notifying at once is
enough — each child's header shows only its time and the subtext is gone,
while the conversation title is drawn in every state the shade has. With the
server's
count known, "more" is the count less one and the badge (`setNumber`) is the
count. The push service does not know the count, so with `count` null "more"
is the figure already in the shade plus one — read back from the
notification's own extras under `io.github.steeb_k.commune.MORE` — or zero
when the shade holds nothing for the room, and the badge stays as the shade
had it; the next sync replaces both with the server's. The collapsed text
stays "Sender: text" in a group room and "text" in a direct one; Mark as
Read, `VISIBILITY_PRIVATE` with its public version, auto-cancel, the category
and the per-room content intent are unchanged. `MAX_MESSAGES` and the
`EXTRA_MESSAGES` read-back are gone, and with them the text-and-sender dedup.
`postLock` stays: two sources still race on the read-back of "more".

**The sync path's three cases.** For a room with a count above zero that is
not the visible room, not the call room, and with notifications on,
`Notifier.update` does one of three things. A readable latest message from
someone else that `claim()` accepts is posted, its sender named through
`senderPerson` off the main thread as before, and `known` set to the count. A
readable latest message already announced raises nothing: if the room's
notification is still in the shade, `refreshRoomCount` brings its "more" and
badge in line with the server, and if it was swiped away nothing happens at
all — that branch is the whole of the fix. A room with no readable latest
message falls back on the count: only a count above `known` posts the generic
"N new messages" line, with the room as the speaker. That line is flagged
`summary` (`io.github.steeb_k.commune.SUMMARY` in the extras) and gets no
"more", since "2 more" above "3 new messages" would count the same messages
twice. It is also not in the conversation shape: with the room as the speaker
that shape read "Invite Room: Invite Room: 5 new messages" on the emulator,
and a plain title and line say it once.

`refreshRoomCount` rebuilds the active notification with
`Notification.Builder.recoverBuilder`, changing only the conversation title
of the recovered style, the number and the extra, and sets `setOnlyAlertOnce`
so the correction does not sound
again. It returns early when nothing would change.

**Clearing.** When the sync sees a room's count at zero, or the room becomes
the visible one, or is the call room, or notifications are off, the room's
record is forgotten and its notification cancelled. Mark as Read cancels and
marks the room read; the count reaching zero on the next sync is what forgets
the record. If that request fails the record stays, the room stays unread, and
the message the user dismissed with the button still does not return — the
next new message does. A push whose unread count is zero cancels the room's
notification but leaves the record to the sync.

**The push path fetches every message.** `Push.kt` used to post plain rooms
straight from the gateway payload and fetch only when the payload had no body
or was `m.room.encrypted`. It now goes through
`app.fetchPushedNotification(roomId, eventId)` for every message push, as
Element X does, because the fetched event is what has an `origin_server_ts` for
the rule to order by, and it names the sender and the room the way the sidebar
does. The payload is the fallback when the fetch throws: its body (or "New
message" when it has none) is claimed under the payload's event ID stamped
with `System.currentTimeMillis()`, so a later sync preview of the same message
is older and stays quiet. The behaviours that were already there are kept: an
unread count of zero clears, a push with no event is badge synchronisation and
only holds the CPU for the sync, `m.call.*` pushes are ignored (the fetch also
returns nothing for a call invite), our own message from another device is
dropped, and an emote is spoken by the room rather than named twice. The
payload format stays full — that settlement in `doc/track3-convergence.md`
still holds, for a smaller reason: the payload is now the fallback and the
call filter rather than the source.

## Where it lives

| What | Where |
|---|---|
| The rule and the record | `MessageNotifications.kt`: `AnnouncedMessages.claim`, `known`/`setKnown`, `forget` |
| Posting the one-message notification | `MessageNotifications.kt`: `postRoomMessage` (`count: Int?`, `timestamp`, `summary`), `moreText`, the `EXTRA_MORE` and `EXTRA_SUMMARY` keys |
| Correcting the count in place | `MessageNotifications.kt`: `refreshRoomCount` |
| Mark as Read | `MessageNotifications.kt`: `MarkReadReceiver`, unchanged |
| The sync source | `Notifier.kt`: `Notifier.update`, called from the room-list listener in `CommuneState.kt` (`notifier.update(rooms)`); `Notifier.post` names the sender off the main thread |
| The push source | `Push.kt`: `postFromPayload` (the name survives; it no longer posts from the payload first), `pushedSentence` |
| The account key | `app.sessionUserId()`, the facade's `session_user_id`; `Notifier.accountKey` removed, and its assignment in `CommuneState.refreshAccounts()` |
| The event's identity and order | `commune-core/src/facade.rs`: `FfiRoom::latest_event_id` (`Option<String>`) and `FfiRoom::latest_event_timestamp` (`u64`, ms, 0 when none), filled from `LatestPreview`'s new `event_id` and `timestamp`, which `latest_preview` takes from the same `Remote` event it already read the body from |
| The bindings | `android-kotlin/app/src/main/java/io/github/steeb_k/commune/core/commune_core.kt`: `FfiRoom.latestEventId` and `latestEventTimestamp` (a `ULong`; `Notifier` calls `toLong()`) |

The bindings are generated, never edited by hand: step 3 of "Building" in
`android-kotlin/README.md`, the same way `5b870043` did it, from a host build
of the crate. This regeneration also moved the API checksum of
`set_update_feed` — the committed bindings had drifted from a doc-comment
change in the facade since the last regeneration — which is harmless, and is
the generator being right.

## What was not done

No `deleteIntent`, by design (above). No inline reply action and no
conversation shortcut; Mark as Read is still the only button. No group summary
notification across rooms: each room is its own notification, and the system
bundles them as it likes. The "more" on the push path is an estimate until the
next sync; it is not the server's figure.

**A known race, left unguarded.** `claim()` and `postRoomMessage` are each
atomic but not together. If the sync and the push claim two _different_
messages for the same room at nearly the same moment, the one claimed first
can be posted second, and the shade then shows the older message until the
next one arrives. The cost is a briefly stale line, never a message raised
twice, and holding one lock across the sender lookup — which may fetch an
avatar — was judged worse.

**What the minute's allowance still leaves.** A sync whose room list lags by
less than a minute behind a push — a response generated before the pushed
message existed, delivered after it — posts the older message as the newest,
and the shade says so until the next message. Clocks more than a minute apart,
between two homeservers or between the phone and the server when the push
fallback stamps with the phone's clock, still drop real messages as stale.
Both are the rule doing what it says, written down so that nobody mistakes
them for a fresh bug; the first was judged the rarer and the cheaper of the
two.

## Rebase guide

None of this touches a file upstream Fractal has: the Kotlin application and
`commune-core` are the fork's own, so a Fractal release cannot conflict with
it. What can break it is the core moving underneath. If `LatestEventValue`
changes shape in a `matrix-sdk` bump, `latest_preview` in `facade.rs` is where
the event ID and `origin_server_ts` come from, and both must keep coming from
the same event as the body — an ID from one event and a body from another
would announce the wrong message under the right key. Any change to `FfiRoom`
means regenerating the Kotlin bindings in the same commit. If the push format
is ever narrowed to `event_id_only`, read the section of
`doc/track3-convergence.md` on `set_push_gateway` first: the fetch now carries
every notification, but the payload is still the fallback and the call filter.
And if someone reintroduces accumulation, it has to go through
`AnnouncedMessages` too, or the swiped messages come back exactly as before.

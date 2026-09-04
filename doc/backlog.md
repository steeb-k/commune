# The backlog

One consolidated, deduplicated list of features wanted or half-built,
verified against the code on 3 September 2026. It supersedes the scattered
lists it was built from: the 29 August Kotlin parity audit (which the check
below found substantially stale), the desktop gap-closing fix queue, and the
platform plans under `doc/`.

**How it was verified.** Each item was checked against `commune-core/src/facade.rs`
(the Android FFI surface), `android-kotlin/`, and `src/` (the GTK app). Status
tags: **MISSING** — confirmed absent; **DONE** — confirmed shipped since a list
last named it; **PARTIAL** — some of it exists; **RE-VERIFY** — needs a run or a
deeper read to judge. Re-check a tag before acting on it; the code moves.

The Android/Kotlin app is the shipping mobile target (the GTK-on-Android build
was abandoned). The GTK app is the desktop target and is now a view over
`commune-core` (Track 3, complete).

---

## A. Android / Kotlin app

### Tier 1 — the gaps that most limit it as a Matrix client

1. **DONE 3 Sep — Formatted (HTML) message bodies.** The send path already
   existed (`compose_message` writes Markdown, mentions and emoticons as the
   GTK composer does); the render path was the gap. `commune-core/src/matrix/rich_text.rs`
   is the GTK text pipeline without the widgets: the same sanitizer and
   element subset, blocks and runs, link/identifier/`@room` detection, mention
   pills named from the room, custom emoticons, emote name prefix. The facade
   puts it on every text event as `rich` (flat blocks with quote depth, indent
   and list marker); `RichText.kt` draws it. Verified on the emulator against
   a posted batch (markup, quote, lists, code block, pills, plain-text links,
   emote, heading, colours, rule, details).
2. **BUILT 3 Sep, RE-VERIFY on the Pixel — Push notifications decrypt.**
   `session/notifications/pushed.rs` is the GTK `android_push.rs` fetch path
   without the GTK: bounded wait for the sessions of a push-started process,
   `NotificationClient` (`MultipleProcesses`, `/context`, 25 s timeout), the
   `NotificationBody` the core already words. `fetch_pushed_notification` on
   the facade; `Push.kt` takes that route when the payload has no body or is
   `m.room.encrypted`, and words it with the GTK sentences. The payload format
   stays full (the 1 Sep decision), so unencrypted pushes are still instant.
   Not verifiable on the emulator (no UnifiedPush distributor): needs the
   Pixel with ntfy and an encrypted room.
3. **DONE 3 Sep — Search in encrypted rooms.** `search_room` already went to
   the local index for an encrypted room; `reindex_room_search` is now on the
   facade and the search page offers "Index the loaded messages" in an
   encrypted room when nothing was found, where the GTK page puts its button.
   `is_encrypted` rides on `FfiRoom`. Verified on the emulator: a clear-text
   message in the encrypted room is found; the button runs without error.
4. **DONE 3 Sep — Per-message encryption authenticity shield.** `shield` on
   the event (warning/caveat + the SDK's code); the bubble shows the GTK
   icons' equivalents beside the timestamp row and a tap says the GTK
   sentence. Verified with a clear-text message in an encrypted room.
5. **DONE 3 Sep — Open `matrix:` / matrix.to links.** Intent filters for the
   `matrix` scheme and the matrix.to host; `parse_matrix_link` on the facade
   (the GTK `MatrixIdUri` parser); a joined room opens in place, anything
   else asks in a dialog (Join / Chat) — the reduced form of the GTK room
   preview and profile dialog. Links and pills inside messages take the same
   route. Event links open the room; there is no jump-to-event yet.
6. **DONE 3 Sep — User-directory search.** `search_users` on the facade; the
   invite dialog and the direct chat dialog list matches under the field as
   one types (invite hides current members). Note Synapse's default only
   returns users who share a room.
7. **DONE 3 Sep — Account management while logged in.** `session/account.rs`
   (change password, deactivate, list/remove third-party IDs, add an email
   through its validation link; one password-answering UIA helper like the
   device sign-out) and an "Account" group on the settings page
   (`AccountManagement.kt`). Verified on the emulator: the password change
   took (API login with the new one), the addresses dialog loads. Deactivation
   shares the UIA helper and was not run against a live account; adding an
   email depends on the homeserver sending mail. The OAuth account-management
   URL the GTK deactivate page opens is not offered (the harness has none).

### Tier 2

* **DONE 3 Sep — Spaces write.** `session/room/spaces.rs` is the GTK module
  (child event in the space counts, parent claim in the room is best effort,
  parents listed only when believable); facade `add_room_to_space`,
  `remove_room_from_space`, `parent_spaces`; the create dialog has a
  "Create a space" switch (`create_room` takes `is_space`); the room details
  page lists the room's spaces with Remove and an Add picker. Verified:
  `m.space.child` written with `via`, a created room carries `m.space`.
* **DONE 3 Sep — Pin/unpin write path.** `pin_event`/`unpin_event` on the
  facade, `is_pinned` on every event, Pin/Unpin in the message sheet.
  Verified against `m.room.pinned_events`.
* **DONE 3 Sep — Enable encryption on a room.** `enable_room_encryption`;
  an Encryption row on the room details with the GTK warning. Verified
  (`m.room.encryption` written).
* **DONE 3 Sep — URL preview cards.** `preview_url` on a text event by the
  GTK rule (never in a room that is or may be encrypted; a "Show Link
  Previews" session setting); `url_preview` asks the remote cache;
  `UrlPreviewCard.kt` draws site, title, description, image. Verified with
  gnome.org in the DM.
* **DONE 3 Sep — Crypto: show your own QR, and reset identity.**
  `verification_qr_code` (the flow's `QrVerification` bytes) drawn with the
  zxing encoder while waiting after accepting a request; `reset_cross_signing`
  (password-answered UIA, the OAuth stage refused as unsupported) behind a
  "Reset Crypto Identity" settings row. Reset verified (alice's master key
  appeared); the QR needs a second real session to verify — owed.
* **DONE 3 Sep — Unban a member.** `unban_user`; the member sheet shows
  Unban for a banned member. Not exercised (no banned member on the harness).
* **DONE since the audit** (verify only if touching them): image packs
  (create/rename/delete, sticker and emoticon usage), thread timeline and the
  pinned-events list, notification keywords (add/remove) and per-room mode,
  `set_power_levels`, kick, ban, report event, ignore/unignore.

### Tier 3 — polish (re-verified against the code 3 Sep)

* **DONE 3 Sep — Per-room drafts.** `save_draft`/`load_draft` on the facade
  keep the composer's text in the SDK store, as the GTK composer does; the
  composer loads it on open and saves a moment after typing stops. Verified.
* **DONE 3 Sep — Presence on Android.** `user_presence` on the facade (the
  core's `PresenceList`), a badge and the status message on member rows, a
  "Share Presence" switch (the GTK `share-presence` setting, default on).
  Verified with bob's status message.
* **DONE 3 Sep — A failed-session page.** `error` on `FfiSessionInfo` and
  `remove_session`; the account switcher's row says why a session could not
  be restored and offers Remove, as the GTK switcher row shows the error.
  Not exercised (no failed session on the emulator).
* **NOT PARITY — waveforms on voice messages, media-viewer paging, a
  tombstone banner, verify-a-specific-device.** The GTK app has none of
  these either (its audio row draws no waveform, its media viewer shows one
  item, a tombstoned room only changes category, its session rows show the
  verified state without a per-device request). They stay as ideas, not
  gaps.
* **DONE 3 Sep — Several attachments at once.** Both pickers now return
  many files (GTK `open_multiple_future`, Android `OpenMultipleDocuments`),
  a drop of several files on the GTK history and Android's share sheet
  (SEND and SEND_MULTIPLE, with a room picker when no room is open) feed
  the same queue, and the preview offers "Send All (N)" beside "Send" —
  Element Classic's shape. Each file is its own message, sent in the order
  picked, each awaited before the next. Verified on the emulator: two
  pictures picked together arrived as two image events in order. The GTK
  side compiles and lints; its dialog is owed an eyeball.
* **RE-VERIFY — avatar cropping.** Some cropping code exists in the Kotlin
  app; nobody has checked it against the GTK avatar editor.

## B. Desktop / GTK

Re-verified 3 Sep 2026 on a Linux build of this branch (WSL archlinux under
WSLg, `meson setup` needs `GIT_DIR`/`GIT_WORK_TREE` pointed at the worktree's
gitdir because the `.git` file carries a Windows path; the app runs inside
`dbus-run-session` with an unlocked `gnome-keyring-daemon`, or the Linux
secret store has no bus).

* **FIXED 3 Sep — Threads list inserting new roots.** Confirmed broken
  first: the SDK's `ThreadListService` only refreshes roots it already
  lists, so a thread that begins while the list is open never appears. The
  core's `ThreadList` now presents a mirror of the SDK's list and puts a
  new thread in front the moment its first reply arrives (root fetched,
  reply as latest event), following later replies itself. Verified: a
  third thread appeared live at the top and its second reply made it
  "2 replies".
* **CONFIRMED — Start a thread.** The affordance is the "Reply in Thread"
  entry of the message actions group (`event_actions/group.rs`), which the
  empty threads list names. The popover could not be captured under WSLg's
  X server, so this stands on the code path.
* **CONFIRMED, NOT MISSING — `m.room.policy` set/unset render.** The state
  rows read "{sender} made localhost check the messages of this room." for
  a valid set (the content needs `public_keys`; one without parses as the
  unset, which the spec says is right) and "{sender} stopped the checking
  of this room's messages." for the unset. The redacted rule reads
  "{sender} removed a moderation rule about users."
* **The four minors:** the redacted-rule sentence renders (above); the
  thread view hides the root's chip, as intended; the identity-server row
  is refreshed when the dialog opens but goes stale while it stays open
  (changed the account data live: the row kept the old server until the
  dialog was reopened — a small follow-up: an `m.identity_server`
  observable on the core's global account data, and the row bound to it);
  the no-microphone toast branch cannot be reached under WSLg, which offers
  an audio source (the recording started instead).
* **Ideas floated for the next build:** a storage settings page; a move to
  sliding sync (the 2.0 direction).

## C. Platform ports and infrastructure

* **macOS port is unfinished** — plan in `doc/macos-plan.md`.
* **Android sub-plans**, each a doc: `doc/android-attachments-plan.md`,
  `doc/android-media-plan.md`, `doc/android-push-plan.md`.
* **Windows:** two dialogs shared the "dialog is a separate native window, so
  `widget.root()` is not the app `Window`" bug and are fixed; the rule is to use
  `Application::default().main_window()` from inside a dialog. Windows Sandbox
  validation of a clean release install is still owed (`doc/windows.md`).

---

## D. Owed verification (not features)

* **Desktop sweep tail:** calls (need a device and the local Synapse harness).
  The verification flows were tried 3 Sep between the Linux GTK build and the
  emulator (alice on both): the GTK side showed its QR and the emoji, both
  sides listed the same seven emoji, both confirmed — and the emulator's SDK
  then cancelled with `m.timeout` seconds later (GTK: "reached a timeout"),
  with the clocks in agreement. A first attempt was cancelled by the GTK
  side (`m.user`) right after the emulator accepted. Both are open defects
  in the flow, not in the UIs. The rest of the Track 3 sweep is confirmed.
  Sheet: `doc/eyeball-track3.md`.
* **Device eyeball for the two 3 September Android fixes:** the verification
  request notification, and our own message staying out of notification
  previews.

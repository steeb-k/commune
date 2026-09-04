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

### Tier 3 — polish (from the audit; RE-VERIFY each)

Waveforms on voice messages, avatar cropping, media-viewer paging, per-room
drafts, presence on Android (the core has it; the facade does not expose it for
Kotlin), tombstone banner, verify-a-specific-device, a failed-session page.

---

## B. Desktop / GTK

From the August gap-closing run; the Track 3 rewrite may have closed some, so
each is RE-VERIFY unless noted.

* **RE-VERIFY — Threads list inserting new roots.** `thread_list.rs`'s
  `apply_diff` now handles Append and Insert; likely fixed, confirm with a live
  new thread.
* **RE-VERIFY — Start a thread.** The threaded timeline exists
  (`Timeline::new_threaded`); confirm the create-a-thread affordance the empty
  state advertises actually exists.
* **LIKELY MISSING — `m.room.policy` set/unset render nowhere.** No policy-rule
  rendering found in the room history. See `doc/policy-lists.md`,
  `doc/policy-servers.md`.
* **RE-VERIFY — four minors:** the redacted-rule sentence, the thread-view root
  chip, identity-server row staleness, the no-microphone toast branch.
* **Ideas floated for the next build:** a storage settings page; a move to
  sliding sync (the 2.0 direction).

---

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

* **Desktop sweep tail:** calls (need a device and the local Synapse harness),
  and a second logged-in client for the verification emoji and QR flows. The
  rest of the Track 3 sweep is confirmed. Sheet: `doc/eyeball-track3.md`.
* **Device eyeball for the two 3 September Android fixes:** the verification
  request notification, and our own message staying out of notification
  previews.

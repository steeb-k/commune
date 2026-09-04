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

1. **MISSING — Formatted (HTML) message bodies.** `formatted_body` appears zero
   times in the facade; bold, code, quotes, links and mention pills all render
   and send as raw text. The single largest gap. Needs both a send path and a
   render path, and a decision on the markup subset.
2. **MISSING — Push notifications never decrypt.** `Push.kt` posts straight from
   the gateway JSON (`content.body`), so an encrypted room's push is the literal
   "New message". Decrypting needs the SDK's `NotificationClient` on the push
   path, which the facade does not yet expose to Android.
3. **PARTIAL — Search in encrypted rooms.** `search_room` exists, but there is no
   reindex or local index exposed; server `/search` returns nothing for
   encrypted rooms. The desktop reindex is not on the FFI. RE-VERIFY on device.
4. **MISSING — Per-message encryption authenticity shield.** `FfiTimelineItem`
   has no shield/verified-sender field; nothing marks an unverified or
   unencrypted message.
5. **MISSING — Open `matrix:` / matrix.to links.** The manifest carries only the
   login-redirect custom scheme and LAUNCHER; no intent filter claims matrix.to
   or `matrix:` URIs, so a Matrix link from elsewhere cannot open the app.
6. **MISSING — User-directory search.** No `search_users` on the facade; an
   invite needs a literal `@user:server`.
7. **MISSING — Account management while logged in.** No change-password,
   deactivate, or third-party IDs (email/phone). `reset_password` is the
   pre-login forgotten-password flow only.

### Tier 2

* **MISSING — Spaces are read-only.** Only `space_children` (read); no create a
  space, no add/remove a room to a space on the facade.
* **MISSING — Pin/unpin write path.** Only `set_pinned_listener` (read the pinned
  list); no toggle-pin.
* **MISSING — Enable encryption on a room** from Android — not on the facade.
* **MISSING — URL preview cards.** Not exposed to Android.
* **MISSING — Crypto: display your own QR, and reset/rotate identity.** Scanning
  a QR exists (`qr_code_scanned`); showing one for the other side, and resetting
  cross-signing, do not.
* **MISSING — Unban a member.** `kick_user` and `ban_user` exist; no unban.
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

# Closing every parity gap: the plan

The remaining differences between the Kotlin app and the GTK
application, enumerated from a sweep of `src/` on 28 Aug 2026, ordered
into executable chunks. The standing rule for all of it: **wire behavior
is read out of the GTK sources and mirrored, never improvised** — every
chunk below names the GTK module that is its specification. UI is
Android-native; the events are Commune's.

Workflow per chunk: mirror the GTK module, add the facade surface,
regenerate the bindings and the `.so`, build the Kotlin UI, verify
against the local homeserver (alice/bob; `testing/local-homeserver.sh`
has the credentials), then commit, push, and update the memory ledger.
Batch facade work inside a chunk into one bindings cycle.

## 1. Pusher privacy (tiny — do first)

Keep pushing full content (the gateway hop is TLS), and let the OS
redact: both notification builders (`Notifier.build`, `Push.
postFromPayload`) set `setVisibility(VISIBILITY_PRIVATE)` and a
`setPublicVersion(...)` notification that says only "New message" with
the app name. Android shows the public version on the lock screen when
the user hides sensitive content, the full one otherwise. Verify on the
emulator: `settings put secure lock_screen_allow_private_notifications
0`, lock, check redaction.

## 2. Small wire nits (tiny)

* **Upload-limit preflight**: mirror `session_view/room_history/
  message_toolbar/mod.rs::send_attachment` — ask the server's
  `max_upload_size` (SDK caches it) before uploading, refuse oversized
  files with a message instead of a failed upload. Apply to attachment,
  GIF, voice and (later) avatar sends.
* **Sticker full ImageInfo**: `FfiSticker` carries the pack image's raw
  `info` JSON; `send_sticker` deserializes it into the event's
  `ImageInfo` whole, as `image_packs/pack_image.rs::sticker_content`
  sends everything the pack declared.

## 3. Safety and notification settings (small)

* **Ignored users** — spec: `account_settings/safety_page/
  ignored_users_subpage`. Facade: `ignored_users()`,
  `ignore_user(user_id)`, `unignore_user(user_id)` over the SDK's
  account ignore API (`m.ignored_user_list`). UI: Safety group row →
  list subpage; also an Ignore action in the member dialog.
* **Notification keywords** — spec: `account_settings/
  notifications_page.rs` (the `keywords` ListBox). Facade: list/add/
  remove keyword rules through the SDK's notification settings. UI:
  rows under the Notifications group.

## 4. Room details subpages (medium, one batch)

Spec: `session_view/room_details/*`. One facade cycle, one "Room
Settings" section in details:

* **Avatar** (`edit_details_subpage`): upload + `m.room.avatar`;
  `set_room_avatar(path, mime)` beside the existing name/topic save.
* **Join rules** (`join_rule_subpage`): `m.room.join_rules` —
  public / invite / knock, restricted where the room has a parent
  space, exactly the options the GTK page offers.
* **History visibility** (`history_visibility_subpage`):
  `m.room.history_visibility`.
* **Addresses** (`addresses_subpage`): publish/unpublish aliases
  (directory endpoints), set canonical and alternative addresses
  (`m.room.canonical_alias`).
* **Server ACL** (`server_acl_subpage`): `m.room.server_acl` editing.
* **Permissions** (`permissions/`, incl. `add_members_subpage`): the
  full `m.room.power_levels` matrix — role thresholds, per-event
  overrides, invite/kick/ban/redact levels.
* **Room upgrade** (`upgrade_dialog`): `POST /upgrade` with the
  version choice and the same warnings.

Verify each against synapse state (curl the state events as bob).

## 5. Event menu extras (medium bundle)

Spec: `session_view/room_history/event_actions/` (`context_menu.blp`
lists the full menu; `properties_dialog` is the source viewer).

* **Forward** (room picker → resend content), **Copy message link**
  (matrix.to event URL), **Properties** (raw event JSON via a facade
  `event_source(room, event)`), **Report** (report endpoint with
  reason), **Save image/video/audio** (reuse the MediaStore path),
  **Revoke invite** (membership-event action), **Discard** a failed
  local echo. **Multi-select** (select mode with batch copy/forward/
  remove) last — it is the only structural one.

## 6. Composer batch (medium)

Spec: `session_view/room_history/message_toolbar/`.

* **Location sending**: read the GTK send path for the exact content
  shape at implementation time (`location_error_toast` neighborhood),
  then Android's LocationManager (no Play Services dependency) → the
  same `m.location` event. RECORD the permission flow like the mic.
* **Attachment preview** (`attachment_dialog`): show the picked file
  (image preview, name, size) with confirm/cancel before sending.
* **Custom emoji** (`composer_parser.rs` Emoticon chunks +
  `completion/`): image-pack emoticons complete via `:shortcode:` and
  send GTK's exact plain/HTML pair (`data-mx-emoticon` img tags).
* **Read-receipt list** (`read_receipts_list`): tapping the receipt
  avatars opens the named list.

## 7. User-to-user verification (medium)

Spec: `identity_verification_view/` and `room_history/
verification_info_bar`. Extend the existing verification listener to
requests from _other users_ (in-room `m.key.verification.request`),
add Verify User to the member dialog, the in-room info bar when a
request is pending, and the same SAS emoji flow the own-session path
already has. Verify with the `verify_driver` example as the peer.

## 8. Login expansion (medium-large)

Spec: `login/`.

* **Method discovery** (`method_page`): after the homeserver page, ask
  the server which flows it supports; offer password and/or SSO.
* **SSO / OAuth** (`in_browser_page` + `local_server.rs`): Android
  version is Custom Tabs plus a redirect the app catches (custom
  scheme intent-filter; the GTK loopback server has no Android
  equivalent). Wire it to the SDK's OAuth/SSO login API.
* **Registration** (`register_page`): the UIA flow to the extent GTK
  supports it (terms, email); recaptcha stages need a WebView step.
* **Password reset** (`reset_password_page`): the email-token flow.
* **Advanced dialog** (`advanced_dialog`): the well-known/versions
  inspection niceties, last.

## 9. Multi-account (medium-large)

Spec: `account_switcher/` + `account_chooser_dialog/`. The core's
`SessionList` already stores many sessions; the work is an **active
session** concept:

* Facade: `sessions() -> Vec<FfiSessionInfo>`, `set_active_session
  (session_id)`; replace every `first_ready_session()` with an
  `active_session()` resolver (one helper, mechanical sweep). Login
  appends and activates; logout removes the active one and falls back
  to the next, or to the greeter.
* Switching re-arms the room-list and verification listeners (the
  same re-arm the login fix added), reloads profile/settings, resets
  per-room UI state.
* Kotlin: the sidebar header (avatar + name) opens the switcher sheet
  — accounts with avatars, tap to switch, Add Account into the login
  flow. Notification bookkeeping (`notified` prefs) becomes
  per-session keyed.

## 10. Image-pack management (medium)

Spec: `account_settings/image_packs_page` + `room_details/
image_packs_subpage` + `session/image_packs/mod.rs` (the create logic:
make the packs room and the `io.github.steeb_k.Commune.
image_packs_room` account-data pointer when absent — mirror exactly).
Facade CRUD: create pack, add/rename/remove images (upload → mxc →
state update), delete pack, enable/disable a room's packs
(`m.image_pack.rooms` edits, stable name on write). UI: an Image Packs
page under settings and the room-details subpage.

## 11. Inline audio player (small, cosmetic)

Voice/audio bubbles get an in-bubble mini player (play/pause +
progress via ExoPlayer) instead of the full-screen jump. No wire
change.

## 12. Calls (the project — last, own phases)

Spec: `session/calls/` (~5,000 lines: `call.rs` state machine,
`state.rs`, `turn.rs`, `ringtone.rs`, GStreamer `pipeline.rs`) plus
`session_view/call_view` and `room_history/call_row`. These are 1:1
`m.call.*` calls with a TURN server (the test harness runs
`commune-test-turn`).

* **Phase a — read**: full pass over `session/calls` to split what is
  portable signaling/state (call.rs, state.rs, turn.rs) from what is
  GStreamer media (pipeline.rs, ringtone.rs).
* **Phase b — core**: port signaling and call state into
  commune-core behind a listener trait: the core speaks `m.call.*`
  events, negotiates TURN (`/voip/turnServer`), and emits SDP offers/
  answers/candidates to the embedder; media itself is delegated.
* **Phase c — Android media**: `org.webrtc` (Google's stable WebRTC
  package) as the pipeline replacement; a foreground CallService
  (`phoneCall` type), full-screen incoming-call intent with ringtone,
  a CallScreen, and `call_row` rendering in the timeline.
* **Phase d — verify**: emulator against the GTK desktop app over the
  harness TURN container; the final cross-device test is the user's.

## Sequencing

1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12. Quick wins first,
then settings depth, then the composer/timeline, then the structural
ones (login, multi-account) once the facade has stopped moving, calls
as the closing project. Ship a release APK to the device at every
stable point rather than only at the end.

## Post-plan items (user-flagged 28 Aug, before calls get built)

* **Verification onboarding** — a fresh login lands unverified with no
  path to fix it: neither prompted nor discoverable, which strands E2EE
  history ("a very weak login process that can leave users unable to
  read their own chats" — the user's phone session hit exactly this).
  Follow the GTK app's post-login process (`session_verification_view`
  and the greeter's flow) and mirror it: after login, walk the user
  through verify-with-another-device / recovery key, and keep the
  entry point findable afterwards.
* **QR scanning must stay portrait** — the zxing scan activity forces
  landscape today; lock it to the app's orientation instead.

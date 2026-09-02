# Track 3 — the GTK application onto `commune-core`

`doc/kotlin-plan.md` calls `commune-core` the shared core. Until this work
lands, that word describes an intention. What exists is a **second
implementation**: the application's `Cargo.toml` has no path dependency on the
crate, the crate carried its own `Cargo.lock`, and the desktop application had
never compiled a line of it. `commune-core/src` is 19,488 lines, of which only
`facade.rs` is FFI; the rest re-implements `src/session/` (31,038 lines),
`src/secret/` (2,200), `src/session_list/` (786) and the portable half of
`src/utils/`, written by reading those files rather than by extracting them.

Nothing enforces the agreement. No shared crate, no shared tests, no compiler
error when the two drift. `can_call`, `handle_answer` and `handle_select_answer`
had each drifted, and the only thing that caught them was hitting the bugs on a
real device. The consequence is that every fix to the GTK application is two
fixes, and the second is invisible until somebody notices it is missing — which
is what the 32 gaps in `doc/kotlin-parity-plan.md`'s successor audit actually
measure. The mirror-the-GTK-sources rule is doing the work an integration
mechanism should do, and it depends on the right file being read each time.

This document is the plan for making the word true, and the ledger of what the
convergence finds on the way.

## The rulings

Taken 29 August 2026, and they govern every session of this work:

* **Full spine.** `session/`, `session_list/`, `secret/` and the portable
  `utils/` all move. Not the leaves alone.
* **The GTK implementation is the authority.** Where the core's transcription
  disagrees with `src/`, the GTK version is lifted into the core and the
  transcription is deleted. It is the mature lineage, and this closes Kotlin
  parity gaps as a side effect rather than shipping transcription bugs to the
  desktop.
* **The work stays on `fractal-kotlin`.** `main` is untouched until the
  convergence is proven end to end, then one merge.
* **Convergence comes before the parity gaps.** Most of the Tier-1 gaps —
  formatted message bodies, the search merge, per-message shields, the
  push-rule vocabulary — exist in the GTK application today and arrive for
  free.

## The end state

```text
src/            GObject view-models, widgets, .blp   (UI only)
 └─ core_bridge/  observable→notify, VectorDiff→GListModel, wrapper cache
commune-core/   the single implementation            (headless, no glib)
 └─ facade.rs     a thin UniFFI adapter, no logic
android-kotlin/ Compose over the generated bindings
```

## The phases

**Phase 0 — one workspace, one lock. Done.** The crate is a workspace member;
its own `Cargo.lock` is gone. The UniFFI surface is behind a `ffi` feature, so
the GTK application links the crate without an FFI runtime and the Android
build turns it on (`cli` implies it). The application's own gates — `cargo
+nightly fmt --all`, `cargo-machete`, `cargo-deny` — now cover the core for
free, and all three pass unchanged; `deny.toml` needed no entries for the
newly-visible UniFFI tree. **The lock gained 32 entries and changed none**:
`commune-core` itself and UniFFI's bindgen tree, nothing else moved, so no
desktop platform's resolved graph shifted. The application is not yet a
consumer — the path dependency arrives with the first module, because
`cargo-machete` fails an unused one.

**The bridge is built where it is first needed, not up front.** The order this
plan first gave — bridge, then leaves — was wrong, and Phase 0 is where that
showed: the leaves are stateless functions with no observables and no list
models, so a bridge written before them would have had no consumer to be
designed against. The leaves go first.

**The `VectorDiff` half of the bridge still has no consumer, and cannot get
one in Phase 2.** This plan said it arrived with `session_list/`, "the one
leaf that holds state". `session_list/` is not a leaf. The application's
`SessionList` holds `crate::session::Session` `GObject`s; the core's holds
`commune_core::session::Session` and constructs them itself in
`restore_stored_session`. There is no seam that lets one be a view over the
other until `src/session/` has moved, which is Phase 4 — the whole spine. So
the list half of the bridge is written with `room_list/`, alongside the
property half, and Phase 2 gets no further into that directory than the
settings underneath it. What follows describes the bridge as a whole; it is
built when its consumers exist.

**Phase 1 has started, with its first piece.** `src/core_bridge/` exists and
holds `settings_store.rs`, the `GSettings`-backed `SettingsStore` that
`secret/` was allowed to pass `None` for and that `session_list/` cannot be.
It is here rather than in `utils/` because it is bridge work by the
definition below: it exists solely to make one side's shape acceptable to the
other.

The shape is forced by threads. `SettingsStore` is `Send + Sync`, because the
core reaches it from wherever its work happens and `SessionList::restore` is
`async` on a tokio worker; `gio::Settings` is a `GObject` and is neither. So
the values are mirrored in memory: reads never touch `GSettings`, writes
update the mirror and hand the real write to the main context with
`invoke()`. The obvious alternative — a round-trip to the main context per
call — would block a tokio worker on the main loop and deadlock the moment
the main loop was waiting on that task. A `changed` handler keeps the mirror
from going stale, because the application still writes some of its own
settings directly and a mirror that can drift will.

What this buys is that an upgrade does not look like the sidebar forgetting
the order of the accounts: the session list's order and every session's
settings are already in the `sessions` key of
`io.github.steeb_k.Commune`, and the core's own JSON-file fallback would have
quietly started from defaults.

**Phase 1 — the bridge**, at `src/core_bridge/`. New GTK-side code, written
once so every later module is mechanical: a `bridge_properties!` macro driving
`notify_*()` from one `select_all` over the core's `eyeball` subscribers (one
task per object, never one per property — `Room` alone has 43); an
`ObservableVectorModel` presenting a `VectorDiff` stream as a `gio::ListModel`;
a wrapper cache so GObject identity survives diffs, which the sidebar's
`FilterListModel`/`SortListModel` stacks and every `.blp` binding depend on;
and `UserFacingError` implemented for the core's error types, which is where
`gettext` re-enters.

**Phase 2 — the leaves.** `secret/`, `tls.rs`, `http.rs`, `utils/matrix/`,
`klipy.rs`, the image-pack event types, and the session settings under
`session_list/`. About 7,000 lines of duplication, at no behavioural risk: `src/secret/mod.rs` and its
core twin differed only by `pub(crate)`→`pub`, the `APP_ID`/`PROFILE` constants
becoming `config::app_id()`/`config::profile()`, and stripped `gettext`.
`commune_core::config::init()` is called from `src/application.rs` startup with
the Meson values, the GLib directories and a `GSettings`-backed
`SettingsStore`. `secret/` passes `None` for the settings store to begin
with, because nothing under it reads one; the `GSettings` implementation is
owed by the time the session settings move.

**Leaf 1, `secret/`, is done.** The 2,200 lines under `src/secret/` are gone —
five backends, the encrypted token file, the Android Keystore glue — and
`src/secret.rs` is what stands in their place: a `glib::Boxed` newtype, an
`impl UserFacingError`, and one function that hands the core a translated
sentence. The application is a consumer of `commune-core` from this commit on,
and there is one tokio runtime in the process rather than two, because
`crate::RUNTIME` is now the core's.

`secret/` carried the first piece of glue, found before a line of it was
written. `StoredSession` is a construct-only `GObject` property
(`src/session_list/session_info.rs`), constructed in three places —
`src/session/mod.rs`, `src/session_list/failed_session.rs`,
`src/session_list/new_session.rs` — so it derives `glib::Boxed`. The core's
cannot, and the orphan rule forbids deriving `Boxed` for a foreign type, so the
application keeps a newtype around the core's struct with the derive on the
wrapper and a `Deref` through it. Every field access reads as it does today;
only `new()` and `delete()` are forwarded by hand, because an associated
function and a by-value receiver are the two things `Deref` cannot carry.

Two more pieces turned up in the writing, and both are the same shape as the
first — the boundary is not the types, it is the sentences.
`ClientSetupError` had to become the core's, because `StoredSession::new()`
returns one; the enum was a character-for-character transcription, so the
application deleted its own and kept only the `UserFacingError` impl. And the
core had turned two sets of translatable strings into English on the way past,
which is the pair of ledger rows below. **The rule that comes out of this leaf,
for every leaf after it: a value crosses into the core, a sentence does not.**

**Leaf 3, `utils/matrix/`, is the first one that is a split rather than a
move**, and it is what the plan's note about a 176-line divergence actually
meant: the application's `mod.rs` was a _superset_ of the core's, not a copy
of it. `mod.rs` goes from 750 lines to 191, and everything still defined in
it returns a `Pill`, produces a `glib::DateTime`, or is a `gettext` call.

`MatrixIdUri` is the interesting one. Three `glib` impls —
`StaticVariantType`, `ToVariant`, `FromVariant` — blocked it, because the
orphan rule forbids them once the type is foreign. The `secret/` answer, a
newtype, was not needed: all three only ever round-tripped through `String`,
and only two sites used them. A `MatrixIdUriExt` trait carries
`variant_type`/`as_variant`/`from_variant` and `into_pill` instead, and the
`GAction` parameter is the same string it always was. **Not every glib
integration needs a wrapper — check what the impl actually does first.**

The submodules went the same way and each of them differently.
`url_preview.rs` was a straight copy — its fourteen tests were byte for byte
the core's, so deleting them cost no coverage at all. `mutual_rooms.rs` had
no twin: it is the MSC2666 endpoint written out over the SDK's HTTP client
because the pinned ruma only knows the unstable path, with no `glib` or
`gettext` in it, so it **moved** rather than deduplicating, and the Kotlin
variant gets shared rooms as a side effect. `ext_traits.rs` was the
application's file minus one trait, and that trait — `TimelineEventItemIdExt`
— is `GVariant` conversion, so it stayed and the other four are re-exported.

**`media_message.rs` is not a leaf either, and the core's `media.rs` is not
its twin.** They are unrelated modules that both say "media": the core's
fetches media into files for a Compose UI to decode, while the application's
is `gtk::FileDialog`, toasts, `glib::DateTime` filenames and a dozen
`gettext` calls wrapped around some event content types. The data types
inside it might belong below the UI one day, but that is a design question
about the timeline and belongs to Phase 4, not a mechanical dedup.

`password.rs` is **not** a leaf and the plan naming it as one was an error.
It is `adw::PasswordEntryRow`, a meter and a label; its own first line says
the rules are `validate_password` and that these two functions draw them.
The rules moved, the drawing stays.

**Leaf 2 is done too: `tls.rs`, `http.rs`, the image-pack event types and
`klipy.rs`.** Another 1,150 lines out of `src/`, and the shape of it is worth
recording because it is the shape the rest of Phase 2 will have. Three of the
four were pure re-exports — `utils/mod.rs` carries
`pub(crate) use commune_core::{http, tls};` and `image_packs/mod.rs` binds the
core's event module to the name it already used, so not one consumer changed.
Nothing was left behind because there was nothing in them that could not
leave: no `glib` type, no `gettext` call, no sentence.

`klipy.rs` is the one with a seam, and it is `secret/`'s seam again — two
`glib::Boxed` newtypes with `Deref` through, and exactly one `gettext`. The
nineteen tests those files carried moved with them and pass in the core, and
`rustls`/`rustls-pemfile` left the manifest with `tls.rs`.

Six dependencies left the application's manifest with the backends — `oo7`,
`security-framework`, `matrix-sdk-store-encryption`, `rmp-serde`, `zeroize`
and `serde_bytes`, along with the `Win32_Security_Credentials` feature — and
`Cargo.lock` changed by exactly the seven lines that says, with no version
moving anywhere: the core already depended on all of them.

**Leaf 4 is the session settings, and it is what `session_list/` had to
offer Phase 2.** `src/session_list/session_list_settings.rs` is gone and
`src/session/session_settings.rs` went from 347 lines to 175 — 479 lines of
serde, persistence and migration replaced by `commune_core::settings`, which
already reads and writes the same `"sessions"` key of the same schema through
the `SettingsStore` the application now provides. `StoredSessionSettings`,
`SectionsExpanded`, `MediaPreviewsSetting` and its global enum, and the
`CURRENT_VERSION` constant all leave `src/`.

**Nothing had drifted.** That is the finding, and it is worth as much as a
divergence would have been: the two `StoredSessionSettings` matched field for
field, serde attribute for serde attribute, the two `SectionsExpanded`
defaults held the same eight variants, and both `load` implementations
truncated over-long session IDs the same way. The JSON on disk is byte-safe
across this change, which is the only thing a user could have noticed.

What survives in `src/` is the shell, and it is a smaller thing than the
`secret/` seam. `SessionSettings` stays a `GObject` because three `.blp` rows
bind to it **bidirectionally** — `public-read-receipts-enabled` and
`typing-enabled` on the safety page, `notifications-enabled` on the
notifications settings — and a `bind_property` needs a real property with a
real `notify`. So the shell holds a `commune_core::settings::SessionSettings`,
forwards every method to it, and adds the three `notify_*()` calls.

**It needs nothing from the `eyeball` half of the bridge, and the reason
generalises.** The core has no observables on these values and does not need
any: nothing but a setter on the shell can change a session setting, so a
`notify` emitted by that setter cannot be missed. The property bridge is only
owed where something _other_ than the `GObject` can change the value — which
is most of `room_list/` and `Room`, and none of this.

`SessionListSettings` needed no shell at all. It was a `#[property(get)]` on
`SessionList`, and nothing binds it: two Rust call sites read it. It is the
core's type behind a plain method now — the `utils/matrix/` lesson a second
time, that a `glib` impl is worth checking before it is worth wrapping.

Two seams were needed and both are temporary. `SidebarSectionName` is
duplicated identically but for the `glib::Enum` derive and a translated
`Display`, and it cannot converge while it is a property type whose two
conversions go through `RoomCategory`, itself a `glib::Enum` in 26 files —
so a nine-arm `From` sits at the settings boundary until `room_list/` moves.
And `global_account_data.rs`'s version-0 migration read three fields of
`StoredSessionSettings` directly; it goes through the core's `version()` and
two `legacy_*()` accessors instead.

**One real bug in `settings_store.rs`, found by writing its first consumer.**
Every write deferred to the main context with `invoke()` — and a closure
handed to `invoke` is only ever run by the main loop, so one queued while the
application is quitting never runs at all. Until this leaf nothing wrote
through the store, so nothing could be lost; from here every session setting
does, and the last thing a user changes before quitting is exactly the change
worth keeping. A write that is already on the main context now goes straight
through, and only the core writing from its own worker is deferred, where
there is no such deadline.

**Phase 3 — decompose `facade.rs`.** The prerequisite that chunk 18 of
`doc/kotlin-plan.md` does not name. `impl CoreApp` runs from line 766 to 6397 —
135 exported methods, 5,627 lines — and it holds permissions, server ACLs, the
upgrade rules, call signalling, image packs, verification and device management
_inside the UniFFI object_. The GTK application cannot consume that shape. Real
logic moves down into core modules with typed arguments and real error enums;
one-line SDK passthroughs (`kick_user`, `ban_user`, `set_member_power_level`,
`upgrade_room`) stay passthroughs on both sides, because a shared wrapper for
them would only be a second SDK. The Kotlin application is the regression suite
for this phase. **The decomposition is planned in full below — twelve commits
against a module map**; read that section rather than this paragraph.

**Phase 4 — the spine**, bottom-up, one module per session, each a revertible
commit: `room_list/` and the category rules first, then `room/` itself, members
and permissions, the small session-level models, aliases and search, the
timeline, notifications, verification, and calls last. Every module follows the
same five steps — read the GTK module, correct the core to it, re-verify the
Kotlin application, rewrite the GTK module as a view-model, run its eyeball
section.

**Phase 5 — strings**, the 156 `gettext` sites in the model layer, moved per
module as it migrates and never in one pass, updating `po/POTFILES.in` each
time so translations survive.

**Phase 6 — the ledgers.** Roughly 55 documents describe the GObject shape.
They are updated with the commit that invalidates them, and from Phase 2 on
every commit touches `src/`, so `client-comparison.html`, `spec-gaps.html` and
`upstream-defects.html` take the round trip AGENTS.md describes.

## Phase 3 in detail — the decomposition of `facade.rs`

Read end to end on 1 September 2026, and the measurements below are that
read rather than an estimate. The file is 8,565 lines. `#[uniffi::export]
impl CoreApp` runs from line 766 to 6397 — **135 exported methods, 5,627
lines of body** — and around it sit 56 `uniffi` records and enums, six
`with_foreign` listener traits, 39 free functions, two private `impl
CoreApp` blocks holding five helpers, `CallFlows`, `VerificationFlows`, and
six unit tests over the call-party guard.

Reading it whole is what this section is for. The plan below is the
decomposition; the sessions that follow it are meant to be mechanical, and
each one a revertible commit the way Phase 2's leaves were.

### The three decisions that govern every commit

**One. The listener traits invert into observables.** Six
`#[uniffi::export(with_foreign)]` traits — `RoomListListener`,
`MemberListListener`, `TimelineListener`, `TypingListener`,
`VerificationListener`, `CallListener` — are the shape the GTK application
cannot consume, because a `GObject` view-model wants a `Subscriber`, not a
foreign object it must implement. Every `set_*_listener` in the file is the
same body: spawn a task, `wait_for_ready_session`, subscribe to something
the core already has, and push a full snapshot on each change, keeping an
`AbortHandle` in a `Mutex` so a second registration can cancel the first.
The core keeps the subscription; `facade.rs` keeps the task, the abort
handle and the snapshotting, because snapshots-rather-than-diffs and
one-room-at-a-time are FFI shortcuts, not core behaviour. This is the first
commit, and it moves no methods: nothing else can be written against a
moving target.

**Two. `CoreError` is 279 rendered English sentences, and every one of them
is the leaf-1 finding again.** The enum has a single variant,
`Failed { msg: String }`. Forty distinct string literals and 86 `format!`
sites feed it. `secret/linux.rs` cost fifteen translated sentences; this is
that failure at fourteen times the scale, and it is not yet a shipped bug
only because nothing but the Kotlin application — which has no translations
— has ever read one. Each group below gets a real error enum whose variants
carry values, and `facade.rs` gains the `impl From<…> for CoreError` that
renders English as the Kotlin side's fallback. The GTK application gets its
`UserFacingError` impls in Phase 5, with the msgids its existing catalogue
already holds. **This also clears the nine `result_unit_err` clippy
errors**: `Timeline`'s `Result<(), ()>` signatures and `with_room_event`'s
are the same debt, and the Gates section leaves them for exactly this phase.

**Three. The preamble is nine tenths of the boilerplate, and it collapses on
contact.** `first_ready_session()` is called 102 times, `RUNTIME.spawn` 113
times, and the same four-line `ok_or_else` prelude repeats: "No session" 66
times, "Invalid room ID" 39, "Unknown room" 34. It exists because every
method takes `String` and must parse and resolve it. Once the logic lives on
`Session` and `Room` with typed arguments — `&RoomId`, `&UserId`,
`OwnedEventId` — the facade does that resolution once, in a helper returning
`Result<Room, CoreError>`, and each exported method becomes three lines.
**The measured 5,627 should come out well under 2,000, and most of the
reduction is this rather than cleverness.**

### The module map

Line counts are the summed spans of each group's methods inside
`impl CoreApp`. The free functions and state machinery each group also owns
are named in the notes below and are not in the count.

| # | Group | Methods | Lines | Core destination | GTK authority | Error enum |
|---|---|---|---|---|---|---|
| 1 | Safety — the ignored users | 3 | 85 | `session/ignored_users.rs` | `session/ignored_users.rs` (282) | `IgnoredUsersError` |
| 2 | The account's other sessions | 3 | 143 | `session/user_sessions.rs` | `session/user_sessions_list/` (987) | `DeviceError` |
| 3 | The account itself | 16 | 228 | `session/mod.rs` | `account_settings/general_page/` (755) | `AccountError` |
| 4 | Push registration | 2 | 90 | `session/notifications.rs` | `utils/android_push.rs` (700) | `PushError` |
| — | The push rules | 5 | 156 | stay in `facade.rs` — one-line SDK passthroughs | `notifications_settings.rs` (762) | — |
| 5 | Media fetch, search, members | 12 | 456 | `matrix/media.rs`, `session/room/search.rs`, `session/room/media_history.rs`, `session/room/member.rs` | `session/room/search.rs` (767), `member_list.rs` (387), `typing_list.rs` (102), `room_details/history_viewer/timeline.rs` (260) and `event.rs` (149), `utils/matrix/media_message.rs` (515) | `SearchError`, `MediaHistoryError` |
| 6 | Room list, joining, directory | 8 | 324 | `session/room_list.rs`, `session/directory.rs`, `session/remote/room.rs`, `session/remote/space_children.rs`, `session/create_room.rs` | `session_view/explore/` (1,287), `session/remote/room.rs` (550), `session/remote/space_children.rs` (521), `session/room_list/` (730), `components/dialogs/room_preview.rs` (630), `session/user.rs` (547), `session_view/create_room_dialog.rs` (342) | `JoinError`, `DirectChatError`, `RemoteRoomError`, `SpaceChildrenError`, `DirectoryError`, `CreateRoomError` |
| 7 | Login and registration | 9 | 354 | `login.rs`, `config.rs` (`OAuthClientConfig`, `app_name`, `device_display_name`) | `login/` (3,290 over ten files), `components/dialogs/auth/mod.rs` (the stage selection, 755) | `LoginError`, `RegisterError`, `ResetPasswordError` |
| 8 | Image packs, stickers, GIFs | 13 | 525 | `session/image_packs.rs`, `session/room/timeline.rs` (`send_sticker`, `send_gif`), `config.rs` (the packs room's name and topic) | `session/image_packs/` (1,323), `room_history/message_toolbar/mod.rs` (`send_sticker`, `send_gif`, `upload_gif`), `components/image_pack_editor/mod.rs` (522), `account_settings/image_packs_page/mod.rs` (565) | `ImagePacksError`, `SendGifError` |
| 9 | Verification and security | 14 | 621 | `session/verification.rs`, `session/security.rs` | `session/verification/` (1,538), `session/security.rs` (491), `components/crypto/` (the setup views' requests), `account_settings/encryption_page/import_export_keys_subpage.rs` | `VerificationError`, `BootstrapError`, `RecoveryError`, `RoomKeysError` |
| 10 | Timeline and messaging | 22 | 862 | `session/room/timeline.rs`, `session/room/composer.rs`, `session/room/mod.rs` | `session/room/timeline/` (3,033), `room_history/message_toolbar/` (the toolbar and `composer_parser.rs`), `room/mod.rs` (redact, report, invite, permalink), `room_details/edit_details_subpage.rs`, `room_history/event_actions/group.rs` | `TimelineError`, `RoomDetailsError` |
| 11 | Room settings — avatar, join rule, history, addresses | 8 (+ `set_room_details`, taken by group 10) | 559 | `session/room/join_rule.rs`, `session/room/aliases.rs`, `session/room/mod.rs` | `session/room/join_rule.rs` (442), `aliases.rs` (544), `room_details/join_rule_subpage.rs`, `history_visibility_subpage.rs`, `addresses_subpage/`, `edit_details_subpage.rs`, `general_page.rs` (publish) | `RoomSettingsError`, `AliasError` |
| 12 | Permissions, ACL, upgrade, moderation | 10 | 553 | `session/room/permissions.rs`, `server_acl.rs`, `upgrade.rs` | `session/room/permissions.rs` (733), `room_details/permissions/permissions_subpage.rs`, `server_acl_subpage.rs`, `upgrade_dialog/`, `general_page.rs` (upgrade info) | `PermissionsError`, `ServerAclError`, `AclProblem` |
| 13 | Calls | 9 | 671 | `session/calls/` | `session/calls/call.rs` (1,607), `mod.rs` (989), `turn.rs` (360) | `CallError` |

Group 3 also carries `session_settings` and its three setters, already
one-line passthroughs over `commune_core::settings` since leaf 4, and they
stay that way. Group 8 owns nine free functions — `collect_image_packs`,
`collect_enabled_packs`, `stored_packs_room`, `ensure_packs_room`,
`read_pack_content`, `send_pack_content`, `set_pack_enabled_inner`,
`read_account_data`, `parse_sticker_pack` — about 400 further lines. Group 9
owns `VerificationFlows` and `VerificationFlow`, about 200. Group 12 owns
`read_state_content`, `can_send_state`, `allow_room_ids`, `send_canonical`,
`build_upgrade_info` and `cmp_room_versions`, about 180, of which
`build_upgrade_info` is the one piece of genuinely intricate rule-following
in the file. Group 13 owns `CallFlow`, `CallFlows`, `InstalledHandlers`,
`merge_outcome`, `sdp_has_video`, `first_stream_id`, `opaque_party_id`,
`send_call_event`, two lifetime constants and the six tests — about 450 —
and the largest single block anywhere in the file is `set_call_listener`'s
seven event handlers, 365 lines inside one method.

### What stays in `facade.rs`

The 56 `uniffi` records and enums and their `From` conversions; the six
listener traits and the tasks that feed them; `init_core`,
`gif_search_available` and `decode_blurhash`, which are free functions
already; the `ffi_*` item builders — `ffi_timeline_item`,
`ffi_message_kind`, `ffi_state_change`, `ffi_reactions`, `ffi_in_reply_to`,
`ffi_send_state`, `ffi_thread_replies`, and the `From` that replaced
`ffi_history_event` — because they are the FFI's own shape and nothing else
consumes them; and the resolution helpers the preamble collapses into,
`session()` and `room()`. Per the ruling already in this
document, `kick_user`, `ban_user`, `set_member_power_level` and
`upgrade_room` stay one-line SDK passthroughs on both sides.

### The order, and why

Smallest-with-real-logic first, to prove the pattern on something whose
failure mode fits in one screen — and the shared scaffolding rides in with
it rather than going first, for the reason the record of commit 1 below
gives. Then outward by size, with two constraints overriding that order:
**login comes before the groups that need a session**, because it is where
the Android-only hardcoding below is fixed and a GTK login has to keep
working through it; and **calls come last**,
because the call-guard fix already in the ledger is still owed a live check,
and putting the rewrite in front of that check would mean the harness could
not tell which change it was measuring.

Groups 11 and 12 are one subject split in two, because 1,112 lines is more
than one session should take on and they divide cleanly: the first is state
events read and written whole, the second is the power-level matrix and what
it authorises.

### What the read found

**`join_room` and `create_direct_chat` reimplement logic the core already
has.** `RoomList` exposes `join_by_id_or_alias`, `knock` and `direct_chat`;
the facade uses none of the three and walks `room_list().snapshot()` by hand
against the raw `Client` instead. This is the second-implementation problem
appearing _inside_ the core, which is a sharper version of the thing this
track exists to fix. Group 6 deletes the facade's copies.

**`search_gifs` and `fetch_gif_preview` touch no session at all.** They are
free functions wearing a method, and they become `#[uniffi::export]` free
functions beside `gif_search_available` in group 8.

**`forward_event` has no GTK precedent, and says so in its own doc
comment** — the application's Forward menu item is a stub whose action is
never registered. Under the mirror-the-GTK-sources rule that makes it a
no-precedent design, to be flagged rather than lifted: group 10 keeps it,
marks it, and leaves the question of what forwarding should send where it
belongs.

**The `sdp_stream_metadata_changed` receive handler is still missing**, as
the ledger records. Group 13 is the commit that rewrites the handler set, so
that is where adding it costs nothing extra — but the ledger assigns the
verification to Phase 4 module 9, and it needs the two-device check that
module carries. **Write the handler in group 13, verify it in Phase 4.**

(The group numbers in this section and the notes under the table were one
lower than the table's until commit 5 — an earlier numbering that survived
the table being renumbered. They now match the table, which is the
authority.)

Four further findings are divergence-ledger rows, and are in the table
below: the Android-only login redirect, the Android-only pusher strings, the
upload-size refusal, and the packs room's name and topic. The last two are
the leaf-1 rule again — a value crosses into the core, a sentence does not —
and the packs room's name is the one of the four a user reads without
looking for it, because it sits in the sidebar.

### Gates for this phase

Every commit takes the gates listed below, and Phase 3 adds one: **`cargo
clippy -p commune-core --all-targets --features ffi -- -D warnings` must be
green by the end of it.** It currently finds fifteen errors in `facade.rs` —
`too_many_lines` and `struct_excessive_bools` — on top of the thirteen the
Linux lint stops at. The core has never been held to `-D warnings`; the
phase that rewrites the code all of it is in is the phase that settles that.
**The per-commit requirement is that the count is no worse, and that a
group's own lints leave with it** — most of the fifteen sit in code a later
commit deletes outright, so fixing them early is churn in the diff of the
commit that removes them.

The Kotlin application is the regression suite. It is the only consumer of
these 135 methods, so a group that compiles and whose Kotlin screens still
work is a group that moved correctly.

### Commit 1 — the ignored users, and two corrections to this plan

**The scaffolding is not its own commit, and the ordering above is wrong to
say so.** This document records the lesson twice already — the bridge is
built where it is first needed, and Phase 0's original order was reversed
for exactly that reason — and a resolution helper written against zero
callers is that mistake at a smaller scale. So the helpers arrive with their
first real consumer instead: `CoreApp::session()` and `parse_user_id()` land
here, with three call sites rather than a hundred imagined ones, and the
error-rendering seam lands as one `From` impl rather than twelve written
blind.

**`src/session/ignored_users.rs` is the authority, and it had two things the
facade did not.** Both are behaviour, and both are in the ledger below.
The application subscribes to the SDK's ignore-list changes and re-reads the
account data whenever one arrives; the facade read it once, per call, so
ignoring somebody from the desktop never reached a phone that had the screen
open. And the application refuses a redundant request — adding a user
already on the list is a warning and a no-op — where the facade always spent
the round trip.

`commune-core/src/session/ignored_users.rs` is the two of them plus the list
itself, hung off `Session` exactly as `RoomList` is: a `OnceLock` field, a
`Session::ignored_users()` accessor, and a `load()` from `prepare()`. What
stayed in the application is the `items_changed` computation — the shortest
splice that turns the old list into the new one exists to stop
`gio::ListModel` rebuilding rows, and belongs to the bridge.

**The list is a `Vec`, not the application's `IndexSet`.** The keys of
`m.ignored_user_list` are already unique, order is the only other thing the
set preserved, and the index lookups it made cheap were `gio::ListModel`'s.
That is `utils/matrix/`'s lesson a third time: check what the type is
actually doing before carrying it across.

**One thing the move would have broken, caught by reading
`active_session()`.** A cached read is not the same guarantee as a fetch:
`SessionList::active_session()` returns a session as soon as its `Session`
exists, which is _before_ `prepare()` has run, so `ignored_users()` reading
only the cache would answer "nobody" during startup where the old one
answered correctly. `ensure_loaded()` is what keeps the old guarantee, and it
is why that method is still `async` — it earns the keyword rather than
carrying it out of habit.

**The FFI surface is unchanged**, deliberately: the same three methods with
the same signatures, so the generated Kotlin is byte-identical and the Kotlin
application is a regression suite that needed no edit to be one. That is the
property every commit of this phase should try to keep.

**On the clippy count, this plan's instruction was too strong.** "Each commit
should leave the count lower than it found it" is wrong where the remaining
errors sit in code a later commit rewrites: nine of the fifteen are redundant
closures and `map(…).unwrap_or(…)` inside the call handlers, the join-rule
reader and the upgrade builder, and five more are `too_many_lines` on methods
that stop existing when their group moves. Fixing those now is churn that
muddies the diff of the commit that deletes them. **The requirement is that a
commit leaves the count no worse, and that a group's own lints go with it.**
This one added two — a `#[must_use]` on a type already carrying it, and an
`async` with nothing to await — and removed both before landing.

### Commit 2 — the account's other sessions

`src/session/user_sessions_list/` is 987 lines across three files and the
facade's version of it was 143. The difference is not shape: it is four
behaviours, and all four are in the ledger below.

**It merges two sources, and the facade merged one.** `/devices` knows the
display name, the last-seen time and the IP; the crypto store knows whether
cross-signing vouches for the device. The application walks the crypto
devices first, takes each one's API half where there is one, and then adds
whatever the API listed that the crypto store did not — a device that does
not support encryption at all, which is still a session the account has. The
facade walked `/devices` alone and asked `get_device()` per row, so a device
the crypto store knew and the API did not was invisible.

**It degrades, and the facade did not.** When one of the two sources fails
the application lists what the other gave it; only losing both is an error.
The facade returned an error the moment `/devices` failed and showed
nothing — the worse answer for exactly the case a person opens this screen
in, which is when something is wrong with the account.

**It follows the list.** `devices_stream()` carries device updates, and the
application reloads on any that names this user. The subtle half is the one
worth keeping: an update with _nothing_ in it is how a disconnection
arrives, and it does not say whose, so an empty update is taken rather than
skipped. The facade had no subscription.

**It breaks ties.** The other sessions sort by last-seen descending and then
by device ID. The facade sorted by `(!is_current, u64::MAX - last_seen_ts)`
with no tiebreak, so devices the server never dated came back in whatever
order the response happened to have — a list that reshuffles under the
reader between two identical reads. Four tests cover the order, including
that one.

**`sign_out` gained a distinction the facade could not express.** The
application answers the homeserver's user-interactive authentication with a
dialog that speaks more than passwords; the core cannot show a dialog, so
the facade did a password-only two-pass and reported whatever came back.
`DeviceError` now separates _the homeserver wants the password_ from _the
homeserver wants something this core cannot answer_, and the first attempt
reads the offered flows rather than assuming. The GTK application will hand
this to its `AuthDialog` when the account settings migrate; the Kotlin one
gets a sentence that is true.

**The list loads on first use rather than from `prepare()`.** Two requests
that only the account settings screen ever looks at do not belong on the
startup path — the ignored users are different, being one cached read that
the sidebar's filtering wants anyway.

The FFI surface is unchanged again: same three signatures, and the
regenerated Kotlin came back byte-identical.

### Commit 3 — the account's own profile, and one answer instead of two

**This group is smaller than the table said, and the table is corrected
above.** Of the sixteen methods counted here, thirteen are already thin:
`has_sessions`, `session_user_id`, `sessions`, `set_active_session`,
`logout` and the session-settings setters are one-line passthroughs over
`SessionList` and `commune_core::settings`, and moving them would be churn
for its own sake. Three had logic, and it is the same piece of logic three
times.

**`Session` already keeps the profile as an observable, and the facade
fetched past it.** `SessionInner` holds a
`SharedObservable<SessionProfile>`, filled from an on-disk cache at startup
and then from the homeserver, and `session_display_name()` reads it. But
`account_profile()` called `fetch_user_profile()` itself and returned the
result without touching the observable — so one core held two answers to the
question "what is this account called", they were fetched separately, and
nothing kept them in step. It now refreshes the observable and reads it,
which is one round trip as before, one answer, and the disk cache updated on
the way past.

**Neither setter told the profile it had changed, and the application says
in a comment why that matters.** `src/account_settings/general_page/mod.rs`
updates its own copy after a successful display-name or avatar change, with
the reason written out twice: *"If the user is in no rooms, we won't receive
the update via sync, so change the avatar manually if this request
succeeds."* An account in no rooms is never told about its own profile
change. The facade updated nothing, and `Session::profile()` — which the GTK
application will bind to — would have kept the old name until the process
restarted. Both setters now correct the observable, which is why
`set_avatar` splits the upload from the avatar-URL write the way the
application does: the URI the upload returns is the thing the local copy has
to be corrected with, and `upload_avatar()` does not hand it back.

**Two gaps found and deliberately not filled**, because Phase 3 is
decomposition and this document's own ruling is that convergence comes
before the parity gaps:

* **There is no way to remove the account's avatar.** The application has
  `remove_avatar()` sending `set_avatar_url(None)` behind a confirmation;
  the facade has `remove_room_avatar` and no equivalent for the account, so
  the Kotlin application can replace an avatar and never clear one. Adding
  it is a new FFI method, which is a feature, not a move.
* **`set_account_avatar` does not check the upload size** where
  `set_room_avatar`, `send_attachment` and `send_voice_message` all do. The
  inconsistency is the facade's own; the application does not check either,
  so mirroring it is correct here and the guard is a question for whoever
  makes the four consistent.

The FFI surface is unchanged for the third time.

### Commit 4 — the pusher, and the field the transcription dropped

**The authority for this one is not where the plan said it was.** The table
above named `session/notifications/notifications_settings.rs`, and that is
the authority for the push _rules_ — which the facade already reaches
through the SDK's own `NotificationSettings` in five one-line methods that
stay exactly as they are, by the same ruling that keeps `kick_user` a
passthrough. The authority for the _pusher_ is
`src/utils/android_push.rs`, the GTK Android port's own UnifiedPush
implementation. That file is residue this track does not aim at, but it is
still the mature version of this exact feature, and reading it is what this
phase is for.

**It sets `PushFormat::EventIdOnly` and the facade does not — and this
commit does not change that, which is the more useful finding.** With the
field unset the specification has the homeserver POST the whole event to the
push gateway: for an unencrypted room, the sender, the room and the message
body, through a third-party service the user chose only as a wake-up. The
application can afford to narrow it because its Android notification path
fetches the event by ID after the push arrives.

**The Kotlin application cannot, and checking before changing it is what
caught this.** `Push.kt` posts straight from the gateway payload — its
header comment says so — and reads four fields `event_id_only` does not
carry: `type`, to stop a call push becoming a message notification, which
its own comment records as a bug already fixed once; and
`sender_display_name`, `room_name` and `content.body`, without which every
notification reads "Commune / New message". Narrowing the format in the core
would have emptied the notifications of the only application that uses it
and revived that bug, silently, with every gate still green.

**So the push format is a contract with whoever writes the notification, not
a field the core gets to choose.** **Settled 1 September 2026: keep the
payload.** The notification arrives complete and immediately, without waking
a sync to re-fetch what the push already carried, and the disclosure to the
gateway is accepted — an encrypted room discloses only its metadata in any
case, the body being ciphertext. The constraint that comes with the decision
is recorded on the method itself: whoever narrows the format changes
`Push.kt` in the same commit, or Android notifications silently become
"Commune / New message" and call pushes start posting one.

**It skips a registration that would change nothing.** The application asks
`/pushers` first and returns early when the endpoint is already registered.
The facade re-POSTed on every call.

**And one thing this commit deliberately does not do, because the
application's source says it is dangerous.** A pusher under our application
id with some _other_ pushkey looks stale and is not ours: the application id
is identical for every install of this client, so "everything under our app
id" on the homeserver includes the user's other phones, and tidying them
away would silently unregister push on all of them. `android_push.rs` avoids
this by remembering the endpoint it moved off — `State::previous_endpoint`,
with the reasoning written out — and deleting only that one. The core keeps
no such record, so it deletes only what it can prove is its own: the legacy
application id, which is keyed on this device's own pushkey. **The
consequence is honest and worth naming: an endpoint that changes without the
old one being retired leaves a pusher the homeserver keeps POSTing to.**
Closing that needs somewhere to remember the previous endpoint, which is a
design question and not a transcription fix.

Two smaller things stay as they are and are already in the ledger: the
pusher still announces itself as `Commune on Android` whatever the embedder
is, and `lang` is still `"en"` — which `EventIdOnly` makes moot, since a
homeserver sending only an event ID has no text to localise.

### Commit 5 — media, search and members, and the search that found nothing

**Of the twelve, five stay and seven move — and the plan's table needed a
column corrected, not a row.** `set_member_list_listener`,
`set_typing_listener` and `clear_member_list_listener` are decision One's
tasks and abort handles, and stay by that decision; `send_typing` is a
one-line passthrough over `Room::send_typing_notification`, which already
carries the settings check; `get_avatar`, `get_room_avatar` and
`get_mxc_media` are one-line passthroughs over `matrix::media`, which was
already core. The table's `MediaError` does not exist and should not: the
fetch returns `Option` on both sides, as `get_media_file` did before this
phase, and inventing an enum for a `None` would be churn. What the table
was missing is the media history's own module and its authority, and the
media message's — both are in the row now.

**Search in an encrypted room found nothing, and never could have.** The
facade sent every search to `/search`, and a homeserver cannot search what
it cannot read: for an encrypted room the answer is an empty page, every
time, with no error. `src/session/room/search.rs` chooses its backend by
`is_encrypted()` — the server for a room it can read, the local search
index for one it cannot — and sanitises the term for the index's query
parser, because the parser returns an error rather than nothing for a
query a person could reasonably type. `commune-core/src/session/room/search.rs`
is that object, headless: the two backends, the term sanitiser with the
application's four tests, the recency sort the index needs because it ranks
by relevance, the paging, and the generation counter that drops a response
arriving after the term changed. What stayed in the application is the
abort handle and the `gio::ListStore`. `reindex()` moved too — it feeds the
index from the event cache after the fact, which is core logic with no FFI
consumer yet.

**The results were deserialised without the application's helper, and the
helper does two things the facade did not.** `original_message_event_from_raw`
drops an edit event — an `m.room.message` carrying `m.new_content`, whose
body reads `* corrected text` — and applies a bundled edit to the original,
so a result shows the message as it now reads. The facade matched on
`AnySyncMessageLikeEvent::RoomMessage` directly, so an edit showed up as a
result of its own and the original showed its first wording. The core has
the helper and the search now uses it for both backends, as the application
does.

**The page is wider on the FFI, on purpose.** The application pages twenty
results and asks for more as the list scrolls; nothing on the Kotlin side
asks for a second page, so the facade asked for thirty in one go, and it
still does — `RoomSearch::with_page_size` takes it as a parameter and the
facade says why. Narrowing it to the application's twenty would have shown
the Kotlin user a third less for no gain. Paging and `reindex()` are not on
the FFI: that is a feature, and the Kotlin search screen has no scroll-to-load
to call it from.

**A room joined after startup never got its typing, and its member list was
never reloaded.** The application's `set_category` re-runs `set_up_typing()`
and `members.reload()` the moment the room's state becomes joined, because
the list an invite had was likely not complete. The core's own comment said
the wiring was not done — _"a freshly joined room's typing arrives after a
restart"_ — and it is now: `set_category` takes `&Arc<Self>` and does what
the application's does.

**`room_members` polled.** Fifty sleeps of two hundred milliseconds,
checking `state()` each time, was the facade's way of waiting for the load
the first `member_list()` starts. The list's state is an observable;
`MemberList::loaded()` subscribes to it and returns at `Ready` or `Error`,
which is also the more useful answer on error: the application presents what
the store gave it when the server would not list the members, and so does
this, immediately rather than ten seconds later.

**`get_timeline_media` and `get_history_media` extracted the source by
hand, four message types each, and the timeline one also stickers.** The
application's `MediaMessage` (`src/utils/matrix/media_message.rs`) is the
authority: five variants, `from_message`, and a fetch through the SDK's
`MediaEventContent`, which is where an encrypted source gets its keys.
`matrix::media::MediaMessage` is the portable half — the enum, the source
and `into_file()`; what stayed behind is `display_name()`, `filename()` and
`save_to_file()`, which are sentences and a dialog. `Timeline::media_message`
finds the item by ID, because that is what crosses the FFI, and keeps the
sticker arm: the application's `Event::media_message()` has none, but the
enum does, and the Kotlin sticker bubble is a consumer.

**The media history is the same request on both sides, and the difference
is in what the application does with the last page.** The filter, the page
size and the classification into media, file and audio were already a faithful
transcription. `HistoryViewerTimeline::load_inner` appends a chunk only when
the response carries an `end` token; a final chunk that arrives without one
is dropped with `has_reached_start` set. The specification lets a homeserver
omit `end` on the last page with events in it, so the application can lose
the oldest page of a room's media. The facade returned the chunk and the
token together, and the Kotlin viewer appends before it checks the token,
which is the better behaviour. The core does what the facade did —
`Room::media_history_page` hands back both — and the choice of what to do
with a final chunk is the bridge's, recorded in the ledger so the GTK
migration meets it knowingly rather than by transcription.

**Three gaps found and not filled**, by the same ruling as commit 3:

* **The core's `Member` has no `latest_activity`.** The application's
  `MemberList::load` walks the live timeline after loading and stamps each
  member with the timestamp of their last unread-worthy event, which is what
  its members page sorts by. `FfiMember` has no such field, so nothing
  consumes it yet; it is a field to add with the sort that wants it.
* **The application seeds the list with the own member and the direct
  member before loading.** The core's list is empty until the store answers.
  Nothing on the Kotlin side reads the list before `loaded()` returns.
* **The application's `Room` cannot search yet.** `RoomSearch` is core; the
  GTK `GObject` that wraps it, with its abort handle and list store, is Phase
  4's.

**The FFI surface is unchanged for the fifth time, and the bindings diff
caught something the compiler cannot.** `uniffi` copies a method's doc
comment into the generated Kotlin _and folds it into the method's
checksum_, so rewording the doc comment on `search_room` and
`get_timeline_media` moved two checksums and the bindings came back
different with every signature untouched. The doc comments are restored
verbatim — one now incomplete rather than wrong, which its body says —
and the explanation lives in an ordinary comment inside the method. The
rule for the rest of the phase: **an exported method's doc comment is
part of the surface; say what changed in the body, not above it.**

### Commit 6 — joining, the directory and the spaces, and a room the core could not describe

**Two stay, six move, and the table's row was a third of the authority.**
`rooms` is a passthrough over the room list's snapshot and
`set_room_list_listener` is decision One's task; `change_room_category`
was already a passthrough over `Room::change_category` and only lost its
preamble. The other five each turned out to need something the plan's row
did not name: the application never joins a room it has not first
_described_, and the description — `session/remote/room.rs`, a `RemoteRoom`
built from a room summary — is what the join, the directory and the space
hierarchy all hand around. It is now `session/remote/room.rs` in the core
too, as a value: the summary's fields, the lookup that tries the summary
endpoint and falls back to the hierarchy endpoint when a homeserver lacks
it, and the identifier matching that finds the session's own copy of a
remote room. The row names the files the read added.

**`join_room` joined first and knocked on any failure — and never told the
sidebar it was joining.** The application looks the room up, then knocks
_if the join rule allows knocking_ and joins otherwise
(`components/dialogs/room_preview.rs` and `explore/public_room_row.rs`,
the same eight lines in both). The facade went straight to the client:
`join_room_by_id_or_alias`, and on _any_ error a knock, so a public room
behind a flaky connection got a knock request it could not want. It also
bypassed `RoomList::join_by_id_or_alias`, which is where the list records
that a join is in flight — the state the sidebar's "joining" row and the
preview's spinner read — so a join from the Kotlin side was invisible
until the room arrived. And it parsed a `RoomOrAliasId` where the
application parses a `MatrixRoomIdUri`: a pasted matrix.to link, and the
`via` servers a link carries for a room the homeserver cannot reach on
its own, were refused. All three follow the application now:
`Session::remote_room` describes, `RoomList::knock_or_join` decides, and
the parser is the application's.

**`RoomList::join_by_id_or_alias` and `knock` returned English sentences,
inside the core.** `Result<OwnedRoomId, String>` with "Could not join room
{identifier}" already rendered — the leaf-1 rule, in a file that was
transcribed before the rule was written. Nothing called them, which is the
only reason it was not a shipped bug; they return `JoinError` now, and the
application's `gettext_f` gets the identifier back as a value.

**`create_direct_chat` could create a duplicate, and the application's
source says why.** `User::get_or_create_direct_chat` checks the room list
first, as the facade did — and then asks the SDK's `get_dm_room`, with the
reason written out: *"the local check needs the room's direct member to be
computed; the SDK's reads `m.direct` itself, so it still finds the direct
chat whose membership does not currently look like one — which is exactly
the case that used to end in a duplicate room."* The facade had the first
check only, and its first check took the first joined room with that
direct member where `RoomList::direct_chat` takes the one with the latest
activity. Both are the core's `RoomList::get_or_create_direct_chat` now —
on the list, because the core has no `User` object to hang it on, which is
a placement choice this record owns.

**`space_children` listed every descendant, flat and unordered, from one
batch.** The application walks the whole hierarchy (up to ten batches of
twenty, and says when it stopped early), builds each space's children
from its `m.space.child` events in the order the specification defines,
drops a child whose event names no `via` server — that is how the
relationship is undone — and shows a tree that opens a subspace on demand,
with a guard against a space that contains itself. The facade sent one
request with no limit, filtered out the root, and returned whatever the
server had walked in whatever order it walked it: grandchildren beside
children, a removed room still listed if the server still returned it,
and a space of more than one batch silently cut. `session/remote/space_children.rs`
is the application's object as a value. **The FFI's list is flat, so the
facade walks the tree depth-first** — every row the application's tree
could show, in the order it would show them, with the same cycle guard —
which keeps every room the Kotlin screen used to show and gives them the
application's order.

**`create_room` and `explore_rooms` were faithful, and moved anyway.**
Both build a request the application builds identically — the space
kind and its power-level override the dialog adds are in
`Session::create_room` too, behind an `is_space` the FFI does not yet
pass — and the one thing the move adds is the error the dialog
distinguishes: `CreateRoomError::AddressTaken` for the homeserver's
`RoomInUse`, which the Kotlin form now gets as a sentence rather than an
SDK error dump. The directory query carries the server and third-party
network the application's server chooser sets; the FFI passes neither.

**Four gaps found and not filled**, by the standing ruling:

* **Nothing on the FFI says a space listing was truncated**, or that a
  child is suggested, or carries a child's `via` servers. The Kotlin
  screen's Join button re-looks the room up by ID alone, so a room only
  reachable through a server the space names cannot be joined from there.
* **The Kotlin space screen is a flat list**, hence the depth-first walk
  above. A tree wants a depth on the record.
* **No server chooser and no third-party networks on the explore page.**
* **No space creation from the Kotlin side.** `is_space` is in the core's
  options and `false` on the FFI.

One semantic change is deliberate and recorded: a space child's `is_joined`
used to mean "joined"; it now means what the application's `local_room`
means — the session has a room under that ID or alias, joined or not —
so a room the user left shows as one they can open rather than one they
can join, which is what the application shows.

The FFI surface is unchanged for the sixth time, the bindings came back
byte-identical, and the exported doc comments were left alone this time.

### Commit 7 — login, and the redirect that was Android's

**All nine move, into `commune-core/src/login.rs`, as one object.** The
application's `Login` is a navigation stack holding one client; the core's
`LoginFlow` is that client and every step it can take — discover, log in
with a password, build the OAuth 2.0 authorization or the SSO URL, finish
either, register, check a username, ask for the reset email, set the new
password — in the order the pages drive them and with the pages left
behind. The stage selection of `components/dialogs/auth/mod.rs`, which is
logic rather than dialog, came too as `AuthStage::next`, parameterised by
the stages the caller can answer: the application answers five with its
pages, and the FFI answers the two that need nobody.

**The redirect is the embedder's, and the ledger's oldest open row closes
here.** A login through the OAuth 2.0 API ends with the browser sent back to
the application, and where it can be sent is not the core's to know: the
desktop listens on a loopback address, Android registers a custom scheme.
`CoreConfig::oauth_client` now carries the client URI and the redirect URIs
each embedder registers — the GTK application hands over the loopback pair
its `client_registration_data` always built, through
`login::oauth_client_config()`, and `init_core` hands over Android's scheme
and the `steeb-k.github.io` client URI its reverse-DNS check needs. Every
core method that needs the redirect for one login takes it as an argument.
The Android constants stay in `facade.rs`, because the facade _is_ the
Android embedder's Rust half; what left is the core's knowledge of them.
**The pusher's two strings close the same way**: `app_name` names the
application on a device and a pusher and an OAuth client, and
`device_display_name` is what the Android embedder sets to say which
platform it is, the desktop passing nothing because it never registers a
pusher.

**Discovery treated every failure as "no OAuth".** `homeserver_page.rs`
asks for the authorization server's metadata and then tells two answers
apart: `is_not_supported()` falls through to the Matrix native flows, and
any other error aborts with "Could not set up login". The facade asked
`.is_ok()` and fell through on both — so a homeserver whose authorization
server was down for a minute was shown as a password-login homeserver, and
a person typing their password into it was refused by an endpoint that no
longer serves them. `LoginFlow::discover` makes the distinction the
application makes.

**The password login built a client of its own and ignored the discovered
one.** `SessionList::login_with_password` took a URL, built a second client
by `homeserver_url` — no `.well-known`, so `matrix.org` went to
`matrix.org` rather than where its `.well-known` points — and logged in
through that, while the client discovery had just built sat unused in
`pending_login`. The application's method page logs in with the client
its homeserver page built. The facade does the same now, and discovers
first only when nothing did; the `SessionList` method is gone, and with
it two rendered English sentences and a third client-building path.

**`adopt_logged_in_client` returned a `String`.** "Could not create the
session", rendered, inside the core — the leaf-1 rule in
`session_list.rs`, which was transcribed before the rule was written, as
`room_list.rs` was. It returns `ClientSetupError` now, which already had
its `UserFacingError`.

**Three things the application distinguishes that the facade did not.**
A homeserver refusing registration with `M_FORBIDDEN` means "this
homeserver does not allow creating an account", not the catch-all
"Invalid credentials", and `RegisterError::Forbidden` says so. A reset
email refused with `M_THREEPID_NOT_FOUND` means no account uses that
address, and `M_THREEPID_DENIED` means the homeserver cannot send email
at all; the facade showed the SDK's error text for both, and
`ResetPasswordError` carries each as a value. And the username availability
check the register page debounces — free, taken, invalid, reserved, or
"the homeserver did not say" — is `LoginFlow::check_username_availability`,
with no FFI consumer yet.

**Five gaps found and not filled**, by the standing ruling:

* **No username availability check on the Kotlin register page.** The
  core has it; the FFI does not expose it.
* **No account creation through the browser.** `oauth_authorization`
  takes `create_account` and sends `prompt=create` after checking the
  homeserver advertises it, as the application does; the FFI passes
  `false` and registers through the native endpoint only.
* **The terms are accepted without being shown.** The application's
  dialog shows the policies and waits; the FFI's headless walk answers
  the stage on the understanding that creating the account is accepting
  them, which is what it did before and is recorded on the method.
* **A resend of the reset email is a new session on the FFI.** The
  application keeps the secret and bumps `send_attempt` for the same
  address, which the spec uses to tell a resend from a retry;
  `FfiResetHandle` carries neither, so the facade always sends a first.
* **No autodiscovery toggle.** The application's advanced dialog can take
  a typed URL as it stands; the Kotlin flow always discovers, and the
  core's `discover` takes the flag.

The FFI surface is unchanged for the seventh time and the bindings came
back byte-identical. **The clippy count is 13, from 15**: the two
`too_many_lines` on the old login methods left with them, which is the
rule working as written.

### Commit 8 — image packs, and the room whose name was English forever

**Eleven of the thirteen move; the two GIF passthroughs stay, and the
plan's instruction for them is declined.** `search_gifs` and
`fetch_gif_preview` were already one-line calls into `klipy` and `http`,
which are core. The plan wanted them turned into `#[uniffi::export]` free
functions beside `gif_search_available`, and that is exactly the kind of
change this phase's own rule forbids: a method that becomes a free function
is a different Kotlin binding. They stay methods, and the table's
`PackError` was never going to exist — the pack module's error is
`ImagePacksError`, and sending a GIF has the application's own
`SendGifError`.

**The event types were the core's since Phase 2; the object was not.**
`events/image_packs.rs` holds `PackContent`, the two names of every event
and the shortcode grammar, and the application's `ImagePacks` built on
them. The facade did not: it parsed the pack events into `serde_json::Value`
by hand, nine free functions and four hundred lines, and every one of the
divergences below is something the typed model already did that the
hand-rolled JSON did not. `commune-core/src/session/image_packs.rs` is the
application's object headless — the enabled packs under both names,
watched by the SDK's handlers under both names; the packs of a room, read
under both names and written back under the one they came from; the packs
room; the rule that a pack with no images is a deleted one.

**The packs room's name and topic close the ledger row that was worse
than an error message.** They are written into `m.room.name` and
`m.room.topic` the one time the room is created and never translated
again, so they arrive through `CoreConfig::packs_room_name` and
`packs_room_topic` as `credential_label` does: the GTK application passes
its two `gettext` calls, the FFI passes `None` and gets the English.

**`set_pack_enabled` wrote only the stable event, so a pack enabled under
the unstable one could not be disabled.** The application keeps the two
account-data events apart precisely so that disabling can remove a pack
from whichever holds it — a pack another client enabled under
`im.ponies.emote_rooms` would otherwise come back on the next load. The
facade merged both on read and wrote only `m.image_pack.rooms`, so from the
Kotlin side that pack was un-disableable. The core's `set_pack_enabled`
writes the unstable event when that is where the pack is.

**A pack created from Kotlin was enabled nowhere, and numbered wrong.**
The application's editor enables a new pack everywhere on its first save,
with the reason written out: a pack is only usable in the room it lives in,
and a pack that was just created lives in a room that exists for that, so
it would be usable nowhere the user meant. The facade never enabled one —
which is why it also listed the packs room's packs explicitly, since
nothing else would have shown them. And it numbered packs from `pack-2`
where the application takes the empty state key first, because the clients
in the wild use it for the pack of a room. Both follow the application now.

**Stickers and GIFs were sent past the timeline.** The application sends
both through the SDK timeline, which is what gives them a local echo and
the send queue's retry; the facade called `Room::send` directly, so a
sticker sent from a phone in a tunnel was lost rather than queued.
`Timeline::send_sticker` and `Timeline::send_gif` are the application's
paths, `upload_gif` and its 16 MiB bound included; the facade's own 20 MiB
bound and its upload-size check for GIFs — which the application does not
make, leaving the homeserver to refuse — go with it.

**The facade read a personal pack the specification dropped.** MSC2545's
`im.ponies.user_emotes` was not carried into the stable specification,
which expects a personal pack to be a room pack enabled globally instead,
and `events/image_packs.rs` says so in its module comment: not supported
here either. The facade read it anyway, under the name "My Stickers". It
does not now, which is the authority's decision and is recorded as one.

**An invalid shortcode was accepted.** The application refuses a shortcode
outside the grammar before it saves; the facade wrote whatever came.
`save_pack` refuses with `ImagePacksError::InvalidShortcode`.

**Three things the FFI keeps that the application does not have, all
recorded on the methods:**

* **A named pack with no images is presented, not treated as deleted.**
  The Kotlin flow creates a pack and adds its images afterwards; the
  application's editor refuses to save one without an image. The core's
  `room_state_packs` applies the application's rule, and a crate-internal
  `room_state_packs_including_empty` is what the FFI's two-step flow reads.
* **The packs room stands in for the open room.** `packs_for_room` lists
  the packs enabled everywhere and then the room's own; `sticker_packs`
  and `emoticon_packs` name no room, so the packs room — the one room the
  Kotlin application writes into, and where a pack it created before this
  commit lives unenabled — takes that place.
* **An added image has no width or height.** The editor decodes the file
  to record them; this side has no decoder, so the `info` carries the size
  and MIME type it knows.

**One thing the facade did that the authority does not, dropped with it and
worth a second look by whoever adds it properly:** the facade honoured a
per-image `usage` list, which the specification allows an image to carry
to override its pack's. The application's `PackImage` keeps it among the
unknown properties and never reads it. The core follows the application.

The FFI surface is unchanged for the eighth time and the bindings came
back byte-identical. The clippy count holds at 13. **One gate caught what
the others could not**: the crate-internal reader above is only read by
the facade, and the Windows check runs with the `ffi` feature on, so its
re-export was fine there and an unused import under `-D warnings` in the
Linux application clippy, which builds the core without the feature. It
is gated on the feature now, and the Windows side of every commit should
run `cargo check -p commune-core --all-targets` without `--features ffi`
as well as with.

### Commit 9 — verification and security, and the state machine the facade had flattened

**Thirteen of the fourteen move; the listener stays as a bridge, and
`VerificationFlows` is gone.** The application's `IdentityVerification` is
a thirteen-state machine over the SDK's request and the SAS or QR
verification it turns into, and `VerificationList` is what follows
incoming requests and creates outgoing ones. The facade had a two-variant
map of SDK handles and four listener calls, with the machine's decisions
made inline in seven places. `commune-core/src/session/verification.rs`
is the application's two objects headless, states, method intersection,
timeouts and automatic steps included; `set_verification_listener` now
follows the core's list and turns its states into the four calls the
listener has. `session/security.rs` is the application's `SessionSecurity`
— three watched states and the three flags derived from the recovery
state — with the setup views' and the encryption page's operations on the
session beside it.

**Requests from other users arrived as to-device requests, which the
application refuses.** `verification_list.rs` takes a to-device request
only when it is a self-verification, because verifying another user
happens in a room; the facade took every to-device request. It also took
requests already done, cancelled or passive, which the application skips,
and in-room requests in rooms the user had left or been banned from. All
three follow the application now.

**An unanswered request was never dismissed.** The application dismisses
a received request nobody accepted after `REQUEST_RECEIVED_TIMEOUT`, two
minutes; the facade kept it forever, and the Kotlin screen with it.

**Accepting used the SDK's default methods, and so did requesting.** The
application accepts with the intersection of what both sides support and
requests with its own list — SAS, showing a QR code, reciprocating, and
scanning one where there is a camera. The facade called `accept()` and
`request_verification()` bare, offering methods it could not drive. The
FFI now declares what it can do: SAS and scanning, since it can read a QR
code but has nowhere to show one.

**The bare `m.key.verification.start` handler had no precedent and is
gone.** The facade handled a legacy start arriving without a request. The
application does not — every client the SDK talks to sends a request
first — and under the mirror rule a no-precedent handler is a design to
flag, not a behaviour to keep. It is recorded here, and removed.

**`security_state` computed three states on demand and watched nothing.**
The application's `SessionSecurity` follows the SDK's identity, device,
verification and recovery streams and keeps six values current; the
facade asked the SDK three questions per call. The core object is the
observable, `ensure_loaded` waits for the encryption tasks the application
waits for, and the FFI reads it. Three of the six values —
`cross_signing_keys_available`, `backup_enabled`,
`backup_exists_on_server` — are not on the FFI record.

**Recovery's two refusals were one.** The recovery view tells "the
passphrase or key is invalid" from "could not access recovery data"; the
facade formatted the SDK error for both. And a recovery that succeeded
with secrets still missing was reported as plain success, where the
application shows an incomplete page: `RecoveryOutcome` says which, and
the FFI still returns success because the Kotlin flow polls the recovery
state afterwards.

**Cross-signing was bootstrapped with a hand-written password stage.** The
application walks the `AuthDialog`; the facade built the password data
itself. It goes through `AuthStage` now, the same stage selection login
uses, with `AuthStage::password_data` as the one stage this side can
answer.

**Four gaps found and not filled**, by the standing ruling:

* **No method choice on the Kotlin side.** The application asks the user
  to choose when both SAS and QR are possible; the FFI's follower starts
  SAS, as the application does when SAS is the only method.
* **No passphrase for recovery, and no key reset.** `enable_recovery`
  takes a passphrase in the core and the FFI passes none;
  `reset_recovery_key` has no FFI caller.
* **No QR code to show.** The core keeps the `QrVerification` for the
  embedder to render; the FFI declares it cannot.
* **A room left mid-verification is watched by category, not by
  membership.** The application watches its own member's membership; the
  core has no member object per room yet and watches the room's category
  becoming Left, which is the same event one step later.

The FFI surface is unchanged for the ninth time and the bindings came back
byte-identical. The clippy count holds at 13.

### Commit 10 — timeline and messaging, and the composer the facade had improvised

**Nineteen of the twenty-two move; the three listeners stay, and so do the
two that are compositions of passthroughs.** What the message toolbar sends
through the timeline — messages, replies, edits, attachments, voice
messages, locations, stickers — is sent from `session/room/timeline.rs` now,
with the toolbar's upload-size preflight ahead of it; back-pagination has
the application's guard; and the room-level actions — redact, report,
invite, the event permalink, the name and topic — are on
`session/room/mod.rs` as the application's `Room` has them. The nine
`Result<(), ()>` signatures are `TimelineError` now, which is the debt the
Gates section left for this group, and `with_room_event`, the helper that
existed to carry the unit error, is gone. `mark_room_read` stays: it is the
room history's two receipt calls, and `retry_sends` stays: it is the
session's two send-queue calls. `forward_event` stays for the reason its
own doc comment gives. The three listeners are the FFI's snapshot shape.

**The composer is the core's.** The application's `ComposerParser` walks a
`GtkTextBuffer` and turns text, mention pills and emoticon pills into the
content of a message event. The walk is the widget's; what the chunks
become on the wire is not, and `session/room/composer.rs` is that:
`compose_message(chunks, markdown_enabled)` is `into_message_event_content`
line for line — the plain body with the names and shortcodes, the formatted
body with the links and image tags, the emote command stripped, the empty
message refused, and `m.mentions` always present. Seven tests pin it. What
the facade had was an approximation written from memory of the wire, and
it was wrong in five ways:

* **`m.mentions` was absent from a message that mentioned nobody.** The
  application always adds the mentions, empty or not, "to avoid triggering
  legacy pushrules"; the facade added them only when there was a user to
  name. Every plain message from the Kotlin application was evaluated by
  the legacy rules on every recipient's homeserver.
* **`/me` was sent as text, and an empty message was sent.** The composer
  turns the command into an `m.emote` and refuses whitespace; the facade
  did neither.
* **The mention link and the emoticon tag were built by hand.** The
  application's mention URI is `UserId::matrix_to_uri()`, percent-encoded;
  the facade wrote `https://matrix.to/#/` and the raw ID. The emoticon tag
  is escaped with `g_markup_escape_text`, apostrophes included; the facade's
  escaper knew four characters. The application renders the formatted body
  from Markdown with the tags in it; the facade rendered Markdown, then
  substituted tags into the HTML.
* **A reply and an edit were plain text.** The toolbar sends the composer's
  content in both cases; the facade sent `text_plain`, so a reply lost its
  Markdown and an edit its mentions.
* **An edit went through the timeline item.** The toolbar makes the edit
  event through the room and sends it through the send queue, so the event
  edited need not be among the loaded items; the SDK's `Timeline::edit`
  fails for one that is not.

Finding the chunks in the Kotlin composer's plain text — `@Name`,
`:shortcode:`, `@room` — is the FFI's shortcut and stays in `facade.rs` as
`composer_chunks`. `@room` is recorded as such: the application mentions the
room only through a pill its completion offers where the user may notify
the room and the room is not a direct chat; the Kotlin composer offers
nothing, and the FFI finds the word as the push rules find it.

**Redaction went through the timeline item too.** The application's remove
action is `Room::redact`, through the room, and it is a no-op in a room
that is not joined; the facade's `Timeline::redact` needed the event loaded.
`Room::redact`, `Room::report_events` and `Room::invite` are the
application's three, with their `Result<(), Vec<failed>>` signatures.

**A video was sent as a file, and so was an audio file.** The toolbar's
`send_file_inner` chooses the attachment info by MIME type — image, video,
audio, file — and the SDK chooses the `msgtype` from it; the core knew
image and file. A video picked on the Pixel arrived as an `m.file` download.
The dimensions, durations, thumbnails and waveforms the toolbar measures
with the desktop's media stack are the embedder's to add: the core sends
the size, and the FFI passes the size and, for a voice message, the
duration.

**The voice message was named after the recorder's temporary file, and
the file was never removed.** The toolbar sends the bytes under
`"Voice message.ogg"` — translated — and deletes the recording; the facade
sent the path, so the body other clients show was `voice-1756…ogg`, and
the cache kept every recording. `Timeline::send_voice` takes the file
name from the embedder, reads the bytes and removes the file.

**The upload-size sentence is a value.** `check_upload_size` is the core's,
in `timeline.rs`, and refuses with `TimelineError::UploadTooLarge {
max_bytes }`. The English — "This file is too large, the homeserver takes
up to {size}" — is `UserFacingError`'s, with `format_size` moved to
`utils.rs` as the core's fallback formatter. This closes the ledger row from
Phase 2. The facade's other toasts name the action, as the application's
do, so `timeline_failure(error, sentence)` renders the limit from the value
and everything else with the caller's sentence.

**The location body was the core's sentence.** "User Location {geo_uri} at
{iso8601_datetime}" is a `gettext_f` in the toolbar. It is the embedder's
now: `Timeline::send_location(geo_uri, body)`, with the facade writing the
English and the UTC stamp it wrote before.

**The permalink failed where the application falls back.**
`Room::matrix_to_event_uri` returns the unrouted link when the SDK cannot
compute the routed one; the facade returned an error.

**The event source was fetched.** The properties dialog shows the loaded
item's `original_json`, and offers the view only when it has one; the
facade asked the server. `Timeline::event_source` reads the item.

**Pagination had no guard.** The application's `can_paginate_backwards`
refuses a load while one runs, before the timeline is ready, and once the
start was reached — which a pinned timeline is from the start, since the
SDK refuses to paginate it. The facade asked the SDK every time, twenty
events, and `MAX_BATCH_SIZE` was the facade's literal. The guard, the flag
and the constant are the timeline's.

**The room details were sent untrimmed, and to a room not joined.** The
details page trims, turns an emptied field into a removal, and refuses
when the room is not joined; `Room::set_name` and `Room::set_topic` do the
same, with `RoomDetailsError` telling the two failures apart as the page's
two toasts do.

**Five gaps found and not filled**, by the standing ruling:

* **An added reaction is not recorded among the recent emoji.** The
  application's `Room::toggle_reaction` records it in
  `io.element.recent_emoji`; the core has no global account data object.
* **The media measurements.** Image dimensions, video dimensions and
  duration, audio duration and waveform, and the thumbnails — the toolbar's
  loaders are GTK's and GStreamer's.
* **No per-event retry.** The application's retry is the failed echo's
  `SendHandle::unwedge`; the FFI's `retry_sends` restarts the queues.
* **`@room` is not gated on the permission.** The application's completion
  offers it only where `can_notify_room`; the core has no permissions
  object until group 12. _Closed by commit 12._
* **No preload.** The application loads a batch when a live timeline opens
  with fewer than twenty items; the FFI's listener never asks.

**The Linux core clippy could not run at all, and now can.** Proving the
nine `result_unit_err` errors gone meant running `cargo clippy -p
commune-core --all-targets` on Linux with only the `result_large_err`
allow, which no gate had done: it overflowed the query depth limit
computing the layout of an async block in `session/room/search.rs`, at
this commit's parent as much as here. The compiler's own suggestion,
`#![recursion_limit = "256"]` on the crate, lets it run; the library then
passes with the nine errors gone, and the only remaining hits are
`too_many_lines` in the two example drivers, which predate Phase 3 and are
not the core's. The Gates section's Linux command can drop
`-A clippy::result_unit_err`.

The FFI surface is unchanged for the tenth time and the bindings came back
byte-identical. The clippy count holds at 13.

### Commit 11 — room settings, and the two objects the room had not grown yet

**All eight move.** The table's ninth, `set_room_details`, went with group
10. `session/room/join_rule.rs` is the application's `JoinRule` headless —
the simplified value, the knock flag, the room a restricted rule names,
whether we or anyone may join — following `room_info.join_rule()` on every
room-info update; `session/room/aliases.rs` is its `RoomAliases`, the
canonical and alternative aliases following the room info and the seven
edits the addresses subpage makes; and the room itself gained the history
visibility as an observable, `rules()`, `set_history_visibility`,
`is_published`/`set_published` from the general page, and
`set_avatar`/`remove_avatar` from the edit-details page. Both objects hang
off `RoomInner` and update where the application updates them, in
`update_with_room_info`. The subpage's `compute_join_rule` and its seven
tests moved with it, plus one for the value.

**The facade read state events per call where the application follows the
room.** `room_join_rule`, `room_history_visibility` and `room_addresses`
deserialised the raw state event from the store on every call, with a
default when it was missing; the application's objects follow the SDK's
own room info. The values are the same today; the shape is the one the
GTK view-models will bind to.

**A rule the page cannot edit was reported as changeable.** The page's
`can_change` is `value.can_be_edited() && may send the state event`; the
facade checked the power level alone, so an unsupported rule — a
restricted rule with no room membership among its allows, or one the
specification has not named — showed an editable page. The same for the
history visibility: the FFI's enum has no unsupported variant, so an
unsupported value shows as the most restrictive one and `can_change` is
false, where the facade showed it as `Joined` and editable.

**The join rule was sent whatever the room's version, and whether or not
it changed.** The page hides the knock switch and the membership row where
the version cannot take them and saves only a change; the facade sent
`knock_restricted` to a version 7 room and sent the current rule again.
The FFI refuses what the version does not support, since it has no picker
to hide it from, and sends nothing when nothing changed. The same
no-change guard for the history visibility.

**Address edits that changed nothing were sent, and the two reasons an
address cannot be added were one.** `RoomAliases` refuses to set a
canonical alias that already is, to remove one that is not, to remove an
alt alias not in the list, and to add one already listed — before
resolving it — and tells "not registered" (404) from "belongs to another
room" from anything else, and "already registered" (409) from anything
else on registration. The facade sent the no-op events, resolved before
checking, and folded the reasons. `AliasError` names all of them, and the
facade's `alias_failure` renders the three the application shows in its
error labels from the value and the rest with the toast's sentence.

**The avatar could be changed in a room not joined.** The edit-details page
refuses both the change and the removal there; `Room::set_avatar` and
`Room::remove_avatar` do too. The facade's upload-size preflight stays
ahead of the upload as an FFI extra — the application does not ask — and
is recorded as such.

**Where the notification-mode methods stand.** `room_notification_mode`
and `set_room_notification_mode` sit beside this group in the file and in
no group of the table. They are the SDK's `NotificationSettings` calls,
two lines each; the application's `NotificationsSettings` object that
wraps them is the account-settings page's state and a Phase 4 view-model
question, so they stay passthroughs, and the table gains no row.

**Two gaps found and not filled**, by the standing ruling:

* **The membership room's name.** The application resolves the room a
  restricted rule names to a local or remote room and follows its display
  name; the core hands out the ID, and the FFI its list of IDs.
* **`we_can_join` reads the room state, not our member.** The application
  watches its own member's membership; the core has no member object per
  room and reads `RoomState::Banned`, which is the same fact one step
  later.

The FFI surface is unchanged for the eleventh time and the bindings came
back byte-identical. The clippy count falls from 13 to 8: three
`map_unwrap_or` and one `too_many_lines` sat in the methods this group
rewrote.

### Commit 12 — permissions, the ACL and the upgrade, and the object every page was waiting for

**Six of the ten move; four stay passthroughs by the standing ruling.**
`session/room/permissions.rs` is the application's `Permissions` object
headless: the room's power levels, loaded from the store and followed
through the `m.room.power_levels` event, and the fourteen questions the
interface asks of them as one observable `PermissionsState`, with
`is_allowed_to`, `can_do_to_user`, `can_set_user_power_level_to`,
`set_user_power_level` and `set_power_levels`. The permissions subpage's
rows are `PowerLevelsMatrix` — the page reads the power levels into rows
and collects the rows back by rules that are the page's, not the widget's,
and both directions are here with four tests. `session/room/server_acl.rs`
is `Room::server_acl`/`set_server_acl` with the subpage's `check_acl`,
`acls_are_equal` and `unrestricted_acl`, its nine tests moved.
`session/room/upgrade.rs` is the upgrade dialog's `UpgradeInfo` with
`with_room_versions` and `with_privileged_creators`, and `Room::upgrade_info`
as the general page computes it; `cmp_room_versions` is the application's
digit-sequence-aware comparison, its test moved. `kick_user`, `ban_user`,
`set_member_power_level` and `upgrade_room` stay one request each.

**Every `can_change` in the file asked the store; the pages ask the
permissions object.** `can_send_state` read the power levels from the SDK
on every call and never asked whether our member is joined; the
application's `is_allowed_to` answers `false` for a member that is not.
The helper is the object now, after `ensure_loaded`, and the groups before
this one — join rule, history visibility, addresses — go through it. So
does the group-10 gap: `@room` in the Kotlin composer is now offered only
where the application's completion would offer the pill, our member
allowed to notify the room and the room not a direct chat.

**An ACL that allows no server could be sent.** The subpage refuses it —
it shuts every homeserver out of the room, and nothing is left to send the
repair — and confirms one that shuts our own server out. The facade sent
both, and the Kotlin screen has no guard of its own, so a Pixel could brick
a room. `check_acl` is the core's, and the FFI refuses both problems: it has
no dialog to confirm the second with, so it says what the list would do
and declines. The page also sends only a change; so does the FFI.

**The permissions matrix read `redact` raw.** The page shows redacting
others as at least redacting one's own, since the latter is what the former
is measured against; the facade showed the stored value, so a room where
`m.room.redaction` needs more than `redact` showed two rows the page would
never show. The read rules and the write rules are one type now, and the
FFI saves only a change, compared as the page reads it.

**The version order was a simplification.** The facade's
`cmp_room_versions` ordered whole numbers numerically and everything else
lexicographically, "without changing the order of the versions that
exist"; the application's compares digit sequences wherever they fall, so
`org.matrix.msc3757.10` sorts before `org.matrix.msc3757.11`. The
application's is the core's, with its forty assertions.

**The upgrade's `can_upgrade` asked the SDK for `is_direct` and the
successor.** The general page asks the room — `is_direct`, `is_tombstoned`
— and the permissions; `Room::can_upgrade` does the same.

**Two gaps found and not filled**, by the standing ruling:

* **The FFI cannot confirm shutting its own server out.** The page asks;
  the FFI refuses. A `confirmed` flag on `set_room_server_acl` would change
  the bindings.
* **The power levels are loaded on first use, not with the room.** The
  application loads them when the room is built; the core loads them when
  something asks, which is what the FFI does, and the GTK view-model will
  ask at construction.

The FFI surface is unchanged for the twelfth time and the bindings came
back byte-identical. The clippy count falls from 8 to 5: two redundant
closures and one `too_many_lines` sat in the methods this group rewrote.

## What never enters the core

* `timeline_diff_minimizer/` — it exists to minimise `GListModel` splices, and
  the core passes `VectorDiff` through untouched. It belongs to the bridge.
* `grouping_list_model/`, `expression*.rs`, `fixed_selection.rs`,
  `placeholder_object.rs` — GTK list plumbing.
* Anything that renders a sentence. State-event humanisation moves as semantic
  values; the English stays where `gettext` can reach it.
* GStreamer, glycin, libshumate, aperture, ashpd — platform media and portals.
* One-line SDK calls, per Phase 3.

## The divergence ledger

The point of doing this with the GTK sources as the authority is that the
convergence surfaces every place the transcription drifted. Each one is
recorded here as it is found, with the phase that closes it.

| Found | Where | Divergence | Closes in |
|---|---|---|---|
| 29 Aug 2026 | `facade.rs` call handlers | **No `is_remote_party` guard at all.** The application puts every call handler but `handle_reject` behind `is_remote_party(sender, party_id)` (`src/session/calls/call.rs:354`); the core checked only whether the sender was us. `m.call.hangup` checked nothing whatever, so any participant in the room could end somebody else's call by sending a hangup carrying its ID, and a third party's candidates were fed into a live connection. Surfaced as a `dead_code` warning on the unused `CallFlow::remote_user_id` the moment the crate came under the application's roof — which is the mechanism this whole track exists to build. **Closed 31 Aug**, with the guard ported verbatim and six tests over its truth table. Unit-tested only: it has **not** been put in front of a live call yet, and calls are the one area of this application where that has meant a shipped bug before. The guard only ever rejects events, so it cannot invent behaviour, but it could in principle reject one it should have taken — the case to watch is whether a party ID stays identical across a peer's invite, candidates and hangup, which is what the specification says and what the harness would confirm. | Done, live check owed |
| 31 Aug 2026 | `facade.rs`, `m.call.sdp_stream_metadata_changed` | The core sends this event but has **no handler for receiving it**. The application has `handle_stream_metadata` (`src/session/calls/call.rs:1298`), which is how the far end muting its microphone or camera reaches the interface. On the Kotlin side a remote mute is currently invisible. | Phase 4, module 9 |
| 31 Aug 2026 | `secret/linux.rs` | **Fifteen translated sentences turned into English on the way past.** The application collapses `oo7`'s errors into fifteen messages a person can act on — "The collection or item is locked", "Make sure xdg-desktop-portal is installed, and it is at least at version 1.5.0" — each of them a `gettext` call. The core's transcription did the collapsing in the same place and dropped the `gettext`, then handed the result over as `SecretError::Service(String)`: a rendered sentence, in English, with nothing left for the UI layer to translate. Nobody would have noticed until a Linux user in a translated locale hit a locked keyring. **Closed 31 Aug**: the core classifies into a `KeyringError` value and renders English only as its own fallback, and `src/secret.rs` holds the fifteen `gettext` calls, with the msgids unchanged so no translation was invalidated. This is the general rule for the rest of Phase 2 — a value crosses, a sentence does not. | Done |
| 31 Aug 2026 | `klipy.rs` | **The KLIPY API key was hard-coded into a tracked file.** The application takes it from the `klipy-api-key` Meson option, empty by default, into a generated and git-ignored `src/config.rs` — `doc/gif-search.md` explains that this is because the repository is public and a committed key is a published key. The transcription put the literal in `commune-core/src/klipy.rs`, with a comment saying the core has no build system to take an option from. It went to `origin/fractal-kotlin` in `ce5ffbc2` on 28 August and was found on 31 August while pricing this leaf; the key must be treated as compromised whatever happens to the history. The same file dropped `is_available()`, so the Kotlin build could not turn the feature off the way the desktop can. **Closed 31 Aug**: the key is an embedder-supplied `CoreConfig` field like `credential_label`, `is_available()` is back, and `gif_search_available()` is exported so the Kotlin picker can hide its tab. The literal is out of the source and out of the history. | Done, key needs rotating |
| 31 Aug 2026 | `klipy.rs` | `Gif::title()`'s fallback for a GIF the API gave no title for was `gettext("GIF")` and became a bare `"GIF"`. It is the fallback body of the event, so it is a sentence, and it goes into the room — it is what a client with no image support shows and what a screen reader announces. **Closed 31 Aug** with leaf 2: `Gif::title()` in `src/utils/klipy.rs` shadows the core's method rather than reaching it through `Deref`, and `to_selection()` overwrites the title the core put in, because that is the one that becomes the event body. | Done |
| 31 Aug 2026 | `secret/linux.rs`, `secret/macos.rs` | **The label on the stored credential lost its translation.** It is the one string either variant writes that a person reads outside the application — Seahorse and Keychain Access both show it — and the application has always run it through `gettext_f`. The core hard-coded the English. It cannot do otherwise, so **closed 31 Aug** from the other end: `CoreConfig` carries the sentence as a template and the core only substitutes `{user_id}` into it. The application passes its translated one at startup; the Kotlin variant passes `None` and gets the English, which is what it wants until it has translations of its own. | Done |
| 31 Aug 2026 | `facade.rs` candidates and negotiate handlers | Both drop **every** event whose sender is our own user. The application drops only its own party's echo, because a party is a user _and_ a device: another of our own devices answering our invite is a legitimate remote party. Kept as-is deliberately — the broader check is documented in the core as the fix for a real bug where the echo of our own answer ended the call, and narrowing it wants a two-device test rather than a guess. | Phase 4, module 9 |
| 1 Sep 2026 | `facade.rs` login flows | **The OAuth and SSO redirect is Android's, hardcoded.** `ANDROID_REDIRECT_URI` is `io.github.steeb-k.commune:/oauth2redirect`, and `oauth_client_registration_data()` builds a fixed native-application registration around it. The desktop application does not use a custom scheme at all: `src/login/local_server.rs` runs a loopback HTTP server and registers _its_ address, because a desktop browser has nowhere to send an app scheme. A GTK login through this core would open an authorization URL the browser could never come back from. The redirect and the registration are embedder facts, like `credential_label` and `klipy_api_key` before them, and belong in `CoreConfig`. **Closed 1 Sep**: `CoreConfig::oauth_client` carries each embedder's client URI and redirect URIs — the GTK application's loopback pair, `init_core`'s Android scheme — and every login step that needs the redirect for one login takes it as an argument. The Android constants live in `facade.rs`, which is the Android embedder's Rust half; the core no longer knows them. | Done |
| 1 Sep 2026 | `facade.rs`, `set_push_gateway` | **The pusher describes an Android device, in English, whatever the embedder is.** `app_display_name` is `"Commune"` and `device_display_name` is `"Commune on Android"`, both literals; the `LEGACY_APP_ID` deletion that runs first cleans up after a specific Android debug build. The device name is what a user sees in another client's session list when they audit what is pushing to them, so a desktop session announcing itself as Android is wrong in the one place the string is read. Embedder values, `CoreConfig` again — and the legacy cleanup is Android's alone and should say so. Commit 4 moved the pusher and left these as they were. **Closed 1 Sep** with the login redirect: `CoreConfig::app_name` names the application on the pusher, on a new device and on the OAuth client, and `CoreConfig::device_display_name` is the Android embedder's to set — the desktop passes none, since it never registers a pusher. The legacy cleanup still runs unconditionally, keyed on this device's pushkey, which is harmless where there is nothing to delete. | Done |
| 1 Sep 2026 | `facade.rs`, `check_upload_size` | **The upload-size refusal is a rendered English sentence, with a private byte formatter.** The core builds `"This file is too large, the homeserver takes up to {size}"` and formats the number with its own `format_size`. The application says the same thing at `src/session_view/room_history/message_toolbar/mod.rs:1310` as a `gettext_f` over `glib::format_size`. It is the most commonly hit error in the file — every oversized attachment, avatar and pack image goes through it — and it is a sentence, so it must not cross: the core owes a value (`UploadTooLarge { max_bytes }`) and the two embedders own the wording. The two formatters agree on decimal units, so the rendered text is identical today; only the translation is lost. **Closed 1 Sep**: `TimelineError::UploadTooLarge { max_bytes }`, with `format_size` in `utils.rs` as the core's English fallback. | Done |
| 1 Sep 2026 | `facade.rs`, `ensure_packs_room` | **The packs room is created with an English name and topic.** `"Sticker Packs"` and `"The sticker and emoticon packs that you created. Invite someone here to share them."` are literals; `src/session/image_packs/mod.rs:627` wraps both in `gettext`. This one is worse than a lost error message, because a room name is not an error: it is written into `m.room.name` on the server, it shows in the sidebar next to the conversations, and it is _permanent_ — a user whose packs room was created by the Kotlin build keeps the English name after they translate their client, because nothing re-creates the room. Embedder-supplied strings, and the room the core makes should carry whichever the embedder passed. **Closed 1 Sep**: `CoreConfig::packs_room_name` and `packs_room_topic`, the GTK application passing its `gettext` calls and the FFI passing `None` for the English, as `credential_label` before them. | Done |
| 1 Sep 2026 | `facade.rs` ignored users | **The core never followed the list, and never refused a redundant request.** `src/session/ignored_users.rs` subscribes to the SDK's ignore-list changes and re-reads `m.ignored_user_list` whenever one arrives; the facade read the account data once per call and had no subscription at all, so ignoring somebody from the desktop never reached a phone with the Ignored Users screen open — it would sit on a stale list until it was closed and reopened. The application also guards both directions: adding a user already on the list, or removing one that is not, is a warning and a no-op rather than a round trip the server will ignore. Neither guard existed in the core. **Closed 1 Sep** with Phase 3's first commit, which also found the thing the move would have broken: `SessionList::active_session()` returns a session before `prepare()` has run, so a cache-only read would answer "nobody" during startup where the old fetch answered correctly — `ensure_loaded()` keeps that guarantee. | Done |
| 1 Sep 2026 | `facade.rs`, `list_devices` | **Four divergences in one method, and the worst is what it does when something is wrong.** `src/session/user_sessions_list/` merges `/devices` with the crypto store, so a device known to one source and not the other is still listed; the facade walked `/devices` alone. The application lists what it has when one source fails and errors only when both do; **the facade returned an error the moment `/devices` failed, which is exactly the case a person opens the sessions screen in.** The application follows `devices_stream()` — taking an _empty_ update, because that is how a disconnection arrives without saying whose — and the facade fetched once per call. And the application breaks a sort tie on device ID where the facade had none, so devices the server never dated came back in a different order on every read. **Closed 1 Sep** in `session/user_sessions.rs`, with four tests over the ordering. | Done |
| 1 Sep 2026 | `facade.rs`, `sign_out_device` | **The core could not tell "wrong password" from "this homeserver wants something else".** Signing a device out goes through user-interactive authentication, which the application answers with an `AuthDialog` that speaks several stages; the facade retried once with a password whatever the homeserver had asked for, and reported the resulting failure as an ordinary error. On a homeserver whose sign-out stage is not `m.login.password` — an OAuth 2.0 one, for instance — that is a request that can never succeed and a message that never says so. **Closed 1 Sep**: the first attempt reads the offered flows, and `DeviceError` separates `NeedsPassword` from `UnsupportedAuth`. The GTK application hands the first to its dialog when the account settings migrate; until then no embedder is worse off, and the Kotlin one stops showing a sentence that is not true. | Done |
| 1 Sep 2026 | `facade.rs` account profile | **A profile change never reached the profile, and the application's source says in a comment why that is not academic.** `src/account_settings/general_page/mod.rs` updates its own copy of the display name and the avatar after a successful change, because _"if the user is in no rooms, we won't receive the update via sync"_ — an account in no rooms is never told about its own profile change. `set_display_name` and `set_account_avatar` wrote to the homeserver and touched nothing locally, so `Session::profile()`, the observable the GTK application will bind to, kept the old value until the process restarted. Separately, `account_profile()` called `fetch_user_profile()` past that same observable, so one core held two independently-fetched answers to the same question with nothing keeping them in step. **Closed 1 Sep**: both setters correct the observable — which is why `set_avatar` splits the upload from the avatar-URL write, as the application does, since `upload_avatar()` never hands back the URI the local copy needs — and `account_profile()` refreshes the observable and reads it. | Done |
| 1 Sep 2026 | `facade.rs`, `set_push_gateway` | **No push format is set, so the homeserver POSTs whole events to the push gateway — and narrowing it is not the core's call.** `src/utils/android_push.rs` sets `PushFormat::EventIdOnly` and its comment calls that _"mandatory for content, not merely preferred: events are E2EE, so a full payload would carry ciphertext at best — and the metadata that does not need to travel, still would."_ The facade leaves `format` unset, which the specification reads as "send everything": in an unencrypted room the gateway receives the sender, the room and the message body. **But the application can narrow it only because its Android notification path fetches the event by ID afterwards, and the Kotlin application posts straight from the payload** — `Push.kt` reads `type` to keep a call push from becoming a message notification (a bug its own comment records fixing), and `sender_display_name`, `room_name` and `content.body` for the text. Setting `EventIdOnly` in the core would empty every Kotlin notification and revive that bug with every gate still green; it was written, caught by reading `Push.kt`, and reverted. **Settled 1 Sep: keep the payload.** The notification arrives whole and instantly rather than waking a sync to re-fetch what already arrived, and the disclosure is accepted — an encrypted room gives up only metadata regardless. The method now carries the constraint that comes with that: narrowing the format later means changing `Push.kt` in the same commit. | Closed by decision |
| 1 Sep 2026 | `facade.rs`, `set_push_gateway` | **The obvious cleanup would unregister the user's other phones, and the application's source is what says so.** A pusher held under our application id with a different pushkey looks stale; it is usually another device. `android_push.rs` deletes only the endpoint that registration itself moved off, remembered in `State::previous_endpoint`, precisely because "the `app_id` is the same for every Commune on Android". The core has no such record and so removes nothing but the legacy application id, which is keyed on this device's own pushkey. **Left open deliberately**: an endpoint that changes without the old one being retired leaves a pusher the homeserver keeps POSTing to. Closing it wants somewhere to remember the previous endpoint, which is a design question rather than a transcription fix. | Open by decision |
| 1 Sep 2026 | `facade.rs`, `search_room` | **Searching an encrypted room always found nothing.** Every search went to `/search`, and a homeserver cannot search content it cannot read: for an encrypted room the answer is an empty page, silently, every time. `src/session/room/search.rs` searches the server only where it can read the room and the local search index otherwise, with the term sanitised for the index's query parser — which returns an error rather than nothing for `who's there?` — and the results re-sorted by recency, since the index ranks by relevance. The facade also deserialised results itself rather than through `original_message_event_from_raw`, so an edit event was listed as a result of its own, body beginning with an asterisk, and the original never showed its corrected wording. **Closed 1 Sep** in `session/room/search.rs`, with the sanitiser's four tests; paging and `reindex()` are in the core and not on the FFI, which has no scroll-to-load to call them from. | Done |
| 1 Sep 2026 | `session/room/mod.rs`, `set_category` | **A room joined after startup never got its typing subscription, and its member list was never reloaded.** The application's `set_category` re-runs `set_up_typing()` and `members.reload()` when the state becomes joined, because the list an invite had was likely not complete. The core ran `set_up_typing` at construction only, and said so in a comment — _"a freshly joined room's typing arrives after a restart"_ — so accepting an invite or joining from the directory gave a room that showed nobody typing and an invite-time member list until the process restarted. **Closed 1 Sep**: `set_category` does what the application's does. | Done |
| 1 Sep 2026 | `facade.rs`, `room_members` | **The facade polled the member list's state every 200 ms for up to ten seconds.** The list's state is an observable the application binds to; the facade slept on it. On a server that refuses to list the members the old wait ran out its full ten seconds before answering with what the store had. **Closed 1 Sep**: `MemberList::loaded()` subscribes and returns at `Ready` or `Error`. | Done |
| 1 Sep 2026 | `history_viewer/timeline.rs` | **The application drops the last page of a room's media history when the homeserver omits `end`.** `load_inner` appends a chunk only under `if let Some(end_token) = events.end`; the specification lets a homeserver omit `end` on a final page that still holds events. The facade returned the chunk and the token together and the Kotlin viewer appends before checking the token, which is the better behaviour and is what `Room::media_history_page` keeps. Not a transcription drift but the reverse, and recorded so the GTK migration decides it rather than inherits it. | Phase 4, bridge decision |
| 1 Sep 2026 | `facade.rs`, `join_room` | **Joined first and knocked on any failure, without telling the room list.** The application describes the room (`session/remote/room.rs`, summary endpoint with a hierarchy fallback) and then knocks if the join rule allows it and joins otherwise; the facade went to the client directly, knocked on _any_ join error — a public room behind a bad connection got a knock request — and bypassed `RoomList::join_by_id_or_alias`, so the list never recorded the join in flight and the sidebar never showed it. It also parsed a bare `RoomOrAliasId` where the application parses a `MatrixRoomIdUri`, refusing a pasted matrix.to link and dropping its `via` servers. **Closed 1 Sep**: `Session::remote_room`, `RoomList::knock_or_join`, and the application's parser. | Done |
| 1 Sep 2026 | `session/room_list.rs`, `join_by_id_or_alias` and `knock` | **Two rendered English sentences inside the core** — `Result<OwnedRoomId, String>` carrying "Could not join room {identifier}" — in a file transcribed before the leaf-1 rule was written. Nothing called them, which is why it never shipped. **Closed 1 Sep** with `JoinError`, which carries the identifier as a value for the application's `gettext_f`. | Done |
| 1 Sep 2026 | `facade.rs`, `create_direct_chat` | **Could create a second direct chat with the same person.** `User::get_or_create_direct_chat` checks the room list, then the SDK's `get_dm_room` over `m.direct`, with a comment saying the second check exists because the first misses a direct chat whose membership does not currently look like one — "exactly the case that used to end in a duplicate room". The facade had the first check only, and took the first match where `RoomList::direct_chat` takes the latest-active. **Closed 1 Sep** as `RoomList::get_or_create_direct_chat`. | Done |
| 1 Sep 2026 | `facade.rs`, `space_children` | **One batch, every descendant, no order.** The application walks up to ten batches and says when it stopped, orders each space's children by their `m.space.child` events as the specification defines, drops a child whose event names no `via` server, and guards against a space that contains itself. The facade sent one unbounded request and returned the server's walk as it came: grandchildren beside children, unordered, silently cut for a large space. **Closed 1 Sep** in `session/remote/space_children.rs`; the FFI flattens the tree depth-first because the Kotlin screen is flat. Not on the FFI yet: truncation, suggestion, the `via` servers a Join from that screen would need. | Done, FFI fields owed |
| 1 Sep 2026 | `facade.rs`, `discover_login` | **Every failure of the authorization server's discovery read as "no OAuth".** `login/homeserver_page.rs` tells `is_not_supported()` — fall through to the Matrix native flows — from any other error, which aborts with "Could not set up login". The facade asked `.is_ok()`, so a homeserver whose authorization server was briefly unreachable was presented as a password-login homeserver, and a password typed into it was refused by an endpoint that no longer serves that account. **Closed 1 Sep** in `LoginFlow::discover`. | Done |
| 1 Sep 2026 | `session_list.rs`, `login_with_password` | **A second client, built by URL, logged in while the discovered one sat unused.** The facade's password login called `SessionList::login_with_password`, which built its own client from the typed URL — without `.well-known`, so `matrix.org` went to `matrix.org` and not where its `.well-known` points — and ignored the client discovery had just built; the application's method page logs in with the client its homeserver page built. The same method returned two rendered English sentences in a `String`, and `adopt_logged_in_client` a third. **Closed 1 Sep**: the method is deleted, the facade logs in with the pending flow's client, and `adopt_logged_in_client` returns `ClientSetupError`. | Done |
| 1 Sep 2026 | `facade.rs`, `register_user` and the reset flow | **Three refusals the application names, shown as SDK error text.** `M_FORBIDDEN` on registration means the homeserver does not allow creating an account (the register page says so explicitly, because the catch-all reads "Invalid credentials"); `M_THREEPID_NOT_FOUND` on the reset email means no account uses the address, and `M_THREEPID_DENIED` that the homeserver cannot send email at all — `reset_password_page.rs` has a sentence for each. The facade formatted the SDK error for all three. **Closed 1 Sep** as `RegisterError::Forbidden`, `ResetPasswordError::EmailNotFound` and `EmailDenied`, values with the sentences as the Kotlin fallback. | Done |
| 1 Sep 2026 | `facade.rs`, `set_pack_enabled_inner` | **A pack enabled under the unstable event could not be disabled from Kotlin.** `session/image_packs/mod.rs` keeps `m.image_pack.rooms` and `im.ponies.emote_rooms` apart so that disabling removes a pack from whichever holds it; the facade merged both on read and wrote only the stable one, so a pack another client enabled under the unstable name came back on every load. The same method wrote enabled packs as bare JSON, dropping the unknown properties the specification requires clients to preserve. **Closed 1 Sep**: the core's `set_pack_enabled`, over the typed `EnabledPacks`. | Done |
| 1 Sep 2026 | `facade.rs`, `create_image_pack` | **A new pack was enabled nowhere and numbered from `pack-2`.** The application's editor enables a pack everywhere on its first save — a pack lives in a room that exists only to hold it, so unenabled it is usable nowhere the user meant — and takes the empty state key first, since the clients in the wild use it for the pack of a room. The facade did neither, and so listed the packs room's packs explicitly because nothing else would have shown them. **Closed 1 Sep**: `packs_room`, `unused_state_key`, `save_pack` and `set_pack_enabled` are the core's; the packs room still stands in for the open room on the FFI, which has no room to name. | Done |
| 1 Sep 2026 | `facade.rs`, `send_sticker` and `send_gif` | **Sent past the timeline.** The application sends both through the SDK timeline, which is what gives them a local echo and the send queue's retry; the facade called `Room::send`, so a sticker sent without connectivity was lost rather than queued. The GIF path also checked the upload size, which the application does not — it bounds the download at 16 MiB and lets the homeserver refuse — and bounded the download at 20. **Closed 1 Sep** as `Timeline::send_sticker` and `Timeline::send_gif`. | Done |
| 1 Sep 2026 | `facade.rs`, `collect_image_packs` | **Read `im.ponies.user_emotes`, a personal pack the specification dropped.** `events/image_packs.rs` records that the stable specification expects a personal pack to be a room pack enabled globally, and that the unstable personal pack is not supported here; the facade read it under the name "My Stickers". It also honoured a per-image `usage` list, which the application keeps among an image's unknown properties and never reads. **Closed 1 Sep** by following the application on both; the per-image usage is a specification feature the authority does not implement, noted for whoever adds it. | Done, per-image usage noted |
| 1 Sep 2026 | `facade.rs`, `add_pack_image` | **Any shortcode was accepted.** The application refuses one outside the grammar — ASCII alphanumerics, dashes and underscores, up to a hundred bytes — before it saves; the facade wrote whatever came, and a shortcode with a colon in it breaks the `:shortcode:` completion that reads it back. **Closed 1 Sep**: `save_pack` refuses with `ImagePacksError::InvalidShortcode`. | Done |
| 1 Sep 2026 | `facade.rs`, `set_verification_listener` | **To-device requests from other users were taken, finished requests too, and no request was ever dismissed.** `verification_list.rs` takes a to-device request only for a self-verification — another user is verified in a room — skips requests already done, cancelled or passive, ignores in-room requests in rooms the user left, and dismisses a received request nobody answered after two minutes. The facade did none of the four. It also handled a bare legacy `m.key.verification.start` the application has no handler for. **Closed 1 Sep** in `session/verification.rs`; the legacy handler is removed as a no-precedent design. | Done |
| 1 Sep 2026 | `facade.rs`, `accept` and `request_verification` | **Verification methods were the SDK's defaults, not ours.** The application accepts with the intersection of both sides' methods and requests with its own list; the facade called `accept()` and `request_verification()` bare, so the other side could pick a method this client could not drive — showing a QR code it could not display. **Closed 1 Sep**: `VerificationList::set_supported_methods`, the FFI declaring SAS, scanning and reciprocating. | Done |
| 1 Sep 2026 | `facade.rs`, `security_state`, `recover` | **Three states asked per call, nothing watched; two recovery refusals shown as one.** `session/security.rs` follows four SDK streams and keeps six values current; the facade asked three questions on each call. The recovery view separates an invalid key from inaccessible data and shows an incomplete recovery as such; the facade showed SDK text and plain success. **Closed 1 Sep**: `SessionSecurity`, `RecoveryError::InvalidKey`, `RecoveryOutcome`. Not on the FFI: the three backup flags, the incomplete outcome (the Kotlin flow polls the state instead). | Done, FFI fields owed |
| 1 Sep 2026 | `facade.rs`, `send_message` | **The message content was improvised: no `m.mentions` without a user to name, `/me` sent as text, an empty message sent, the mention URI and the emoticon tag built by hand.** The application's `ComposerParser` always adds the mentions, empty or not, so that legacy push rules are not evaluated; turns `/me` into an emote; refuses whitespace; links with `matrix_to_uri()` and escapes with `g_markup_escape_text`. **Closed 1 Sep** in `session/room/composer.rs`, seven tests; finding the chunks in the Kotlin composer's plain text stays the FFI's shortcut. | Done |
| 1 Sep 2026 | `facade.rs`, `send_reply`, `edit_message`, `redact_event` | **A reply and an edit were plain text; an edit and a redaction went through the timeline item.** The toolbar sends the composer's content for both, makes the edit event through the room and sends it through the send queue; the remove action redacts through the room and is a no-op in a room not joined. The facade sent `text_plain` and needed the event among the loaded items. **Closed 1 Sep**: `Timeline::send_reply`, `Timeline::edit`, `Room::redact`. | Done |
| 1 Sep 2026 | `facade.rs`, `send_attachment`, `send_voice_message` | **A video or an audio file was sent as `m.file`; a voice message was named after the recorder's temporary file, which was never removed.** `send_file_inner` chooses image, video, audio or file by MIME type; `send_voice_message` sends the bytes as `"Voice message.ogg"` and deletes the recording. **Closed 1 Sep**: `Timeline::send_attachment`, `Timeline::send_voice` with the file name from the embedder. The measurements — dimensions, durations, thumbnails, waveform — stay the embedder's. | Done, measurements owed |
| 1 Sep 2026 | `timeline.rs`, `send_location` | **The location body was the core's English sentence.** "User Location {geo_uri} at {iso8601_datetime}" is a `gettext_f` in the toolbar, so it must not cross. **Closed 1 Sep**: the body is a parameter, and the facade writes the Kotlin side's English. | Done |
| 1 Sep 2026 | `facade.rs`, `event_permalink`, `event_source` | **The permalink failed where the application falls back to the unrouted link; the event source was fetched where the application reads the loaded item.** **Closed 1 Sep**: `Room::matrix_to_event_uri`, `Timeline::event_source`. | Done |
| 1 Sep 2026 | `facade.rs`, `paginate_backwards` | **No guard on loading.** The application refuses a load while one runs, before the timeline is ready, and once the start was reached, which a pinned timeline is from the start. The facade asked the SDK every time. **Closed 1 Sep**: `Timeline::can_paginate_backwards`, `is_loading_start`, `MAX_BATCH_SIZE`. | Done |
| 1 Sep 2026 | `facade.rs`, `set_room_details` | **Untrimmed, and sent to a room not joined.** The details page trims, removes on an emptied field, refuses when not joined, and has a toast per field. **Closed 1 Sep**: `Room::set_name`, `Room::set_topic`, `RoomDetailsError`. | Done |
| 1 Sep 2026 | `facade.rs`, `toggle_reaction` | **An added reaction is not recorded among the recent emoji.** The application's `Room::toggle_reaction` records it in `io.element.recent_emoji`; the core has no global account data object to record it in. Open. | Later phase |
| 1 Sep 2026 | `facade.rs`, `room_join_rule`, `room_history_visibility` | **A rule the page cannot edit was reported as changeable.** The page's `can_change` is `value.can_be_edited()` and the power level; the facade checked the power level alone, and mapped an unsupported history visibility to `Joined`, editable. **Closed 1 Sep**: `JoinRuleValue::can_be_edited`, `HistoryVisibilityValue::Unsupported`; the FFI's history enum has no unsupported variant, so that value shows as `Joined` with `can_change` false. | Done, FFI variant owed |
| 1 Sep 2026 | `facade.rs`, `set_room_join_rule`, `set_room_history_visibility` | **Sent whatever the room's version, and whether or not it changed.** The page hides what the version cannot take and saves only a change; the facade sent `knock_restricted` to any room and re-sent the current rule. **Closed 1 Sep**: the FFI refuses what `Room::rules()` says the version lacks and sends nothing for no change; `compute_join_rule` and its tests are `session/room/join_rule.rs`. | Done |
| 1 Sep 2026 | `facade.rs`, `set_room_address` | **No-op edits were sent; the refusals were folded.** `RoomAliases` refuses to set a canonical alias that already is, remove one that is not, remove an alt alias not listed, or add one already listed, and tells not-registered (404) from another-room from already-registered (409). The facade sent the events and gave one sentence. **Closed 1 Sep**: `session/room/aliases.rs`, `AliasError`. | Done |
| 1 Sep 2026 | `facade.rs`, `set_room_avatar`, `remove_room_avatar` | **The avatar was changed in a room not joined.** The edit-details page refuses both. **Closed 1 Sep**: `Room::set_avatar`, `Room::remove_avatar`. The facade's upload-size preflight before the upload is an FFI extra the application does not make; kept and recorded. | Done |
| 1 Sep 2026 | `facade.rs`, `set_room_server_acl` | **An ACL that allows no server could be sent, and one shutting our own server out was sent unconfirmed.** The subpage refuses the first — it shuts every homeserver out and nothing can send the repair — and confirms the second; the facade sent both, and the Kotlin screen has no guard. **Closed 1 Sep**: `check_acl` in `session/room/server_acl.rs`, nine tests; the FFI refuses both problems, having no dialog for the second. | Done, confirmation owed |
| 1 Sep 2026 | `facade.rs`, `can_send_state` | **Every `can_change` asked the store and never asked whether our member is joined.** The application's `Permissions::is_allowed_to` is false for a member not joined, and follows the power-levels event. **Closed 1 Sep**: `session/room/permissions.rs`, `PermissionsState`; the helper goes through the object. The group-10 `@room` gap closes with it. | Done |
| 1 Sep 2026 | `facade.rs`, `room_permissions_matrix` | **`redact_others` was the stored `redact`, not the page's `max(redact_own, redact)`.** The page shows redacting others as at least redacting one's own. **Closed 1 Sep**: `PowerLevelsMatrix::from_power_levels` and `apply_to`, four tests. | Done |
| 1 Sep 2026 | `facade.rs`, `cmp_room_versions` | **The version order was a simplification of the application's.** Whole numbers numerically, the rest lexicographically, where the application compares every digit sequence. **Closed 1 Sep**: `session/room/upgrade.rs`, the application's comparison and its test. | Done |

## Gates

Every commit: the application's `cargo check` and `cargo clippy --all-targets
-- -D warnings`; `cargo test -p commune-core --features ffi`, because
`facade.rs` and its tests are behind that feature and a plain `cargo test`
silently skips them; the Kotlin core still builds for
its ABIs; the pre-commit hook. All of these need the UCRT64 toolchain and the
`x86_64-pc-windows-gnu` target on this machine, and the worktree needs
`src/config.rs` and `hooks/checks-bin.exe` copied in from the main checkout,
because Meson writes them and git ignores them.

**And a Linux `cargo check`, which this machine can do after all.** Half of
what this track touches is behind `#[cfg(target_os = "linux")]` — the keyring,
the Secret Portal, and now the fifteen `gettext` calls in `src/secret.rs` —
and none of it is compiled by a Windows build, so a rename there is invisible
until somebody builds on Linux. Both WSL distributions on this machine can
compile it from `/mnt/c`, with `CARGO_TARGET_DIR` pointed at `/tmp` so the two
platforms do not fight over `target/`:

* `cargo check -p commune-core --all-targets` under **Ubuntu-24.04**, which
  needs nothing installed and takes about ninety seconds. This covers the
  core's Linux backend.
* `cargo check --all-targets` under **archlinux**, which has `gtk4`,
  `libadwaita-1` and `gstreamer-1.0` and so can compile the whole
  application. This is the only thing on this machine that compiles
  `src/secret.rs`'s Linux arm at all. It also runs
  `android-kotlin/build-core.sh --all`, which is the only thing that compiles
  `commune-core/src/platform/android.rs`.

Call `wsl.exe` from PowerShell with a script file rather than a command
string; see the `wsl-invocation-gotchas` note. If a distribution starts
returning `Input/output error` for ordinary commands, the host disk is full:
`wsl --shutdown` and check `C:` before anything else.

**The GTK application building for Android is not a gate, because it is not a
goal.** The Kotlin variant over this core is what replaces it — that is the
whole reason this track exists — so the desktop application is a desktop
application, and its Android arms are the port's residue rather than a target
to keep green. An earlier revision of this section listed the missing
GTK-on-Android build as a hole and priced rebuilding the pixiewood
environment for it. **Do not.** Hours would go into re-verifying a
configuration nothing is meant to ship.

What does still have to build for Android is `commune-core`, which
`android-kotlin/build-core.sh --all` covers and which is in the gate list
above. The Android-only code left under `src/` — `utils/android.rs`,
`android_push.rs`, `android_sync_service.rs`, the notification and setup
glue — is a separate question about when the port's remains get deleted, not
a question about this track.

**The Linux clippy is not green, and it is the core that is not green.**
`cargo clippy --all-targets -- -D warnings` passes on Windows and fails on
Linux with thirteen errors, every one of them in `commune-core` and none in
`src/`. Nine are `result_unit_err` on the `pub async fn … -> Result<(), ()>`
signatures in `session/room/timeline.rs`; four are `result_large_err` on
`oo7::Error` in `secret/linux.rs`. None of the four signatures was touched by
the migration — what changed is that they can be seen at all. Two things
converged to hide them: `secret/linux.rs` cannot be compiled on Windows on any
toolchain, and the MSYS2 clippy is 0.1.97 where the Arch one is 0.1.98, which
flags the `Result<(), ()>` returns the older one lets through.

Because clippy stops at the first crate that fails, those thirteen also hide
`src/`. To lint the application on Linux until Phase 3 clears them, allow
exactly the two lints they are:

```sh
cargo clippy -p commune --all-targets -- -D warnings   -A clippy::result_unit_err -A clippy::result_large_err
```

They are left alone deliberately. `timeline.rs`'s unit errors are the facade's
shape and belong to Phase 3, where the error enums are the point; boxing
`oo7::Error` is a change to the backend this leaf just finished stabilising,
and doing it in the same commit would mean the migration and a refactor could
not be told apart in a bisect. The core has more of this debt behind the `ffi`
feature — `cargo clippy -p commune-core --all-targets --features ffi -- -D
warnings` finds fifteen further errors in `facade.rs`, `too_many_lines` and
`struct_excessive_bools`. **The core has never been held to `-D warnings` and
does not pass it.** Phase 3 is where that is settled, because that is the
phase that rewrites the code all of it is in.

Per module, from Phase 2 on: the module's section of `doc/eyeball-tests.md` on
the GTK application and of `doc/eyeball-android.md` on the Kotlin one. Calls
are verified against the local homeserver with `uiautomator dump`, never by
screenshot.

**The branch's old gate is retired.** "`git diff main fractal-kotlin -- src po
data meson.build meson.options build-aux hooks Cargo.toml Cargo.lock` is empty"
was the premise of the branch, and Phase 0 broke it deliberately: the root
`Cargo.toml` now carries a `[workspace]` table and `Cargo.lock` carries the
core's dependencies. What replaces it is that every migrated module passes its
eyeball section, and `main` sees none of it until the full checklist does.

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
| 5 | Media fetch, search, members | 12 | 456 | `matrix/media.rs`, `session/room/search.rs`, `session/room/member.rs` | `session/room/search.rs` (767), `member_list.rs` (387), `typing_list.rs` (102), `room_details/history_viewer/` | `MediaError`, `SearchError` |
| 6 | Room list, joining, directory | 8 | 324 | `session/room_list.rs`, `session/directory.rs`, `session/remote/space_children.rs` | `session_view/explore/` (1,287), `session/remote/space_children.rs` (521) | `JoinError`, `DirectoryError` |
| 7 | Login and registration | 9 | 354 | `login.rs` | `login/` (3,290 over ten files) | `LoginError` |
| 8 | Image packs, stickers, GIFs | 13 | 525 | `session/image_packs/` | `session/image_packs/` (1,323) | `PackError` |
| 9 | Verification and security | 14 | 621 | `session/verification.rs`, `session/security.rs` | `session/verification/` (1,538), `session/security.rs` (491) | `VerificationError`, `SecurityError` |
| 10 | Timeline and messaging | 22 | 862 | `session/room/timeline.rs`, `session/room/mod.rs` | `session/room/timeline/` (3,033), `room_history/message_toolbar/` | `TimelineError` |
| 11 | Room settings — details, join rule, history, addresses | 9 | 559 | `session/room/join_rule.rs`, `session/room/aliases.rs` | `session/room/join_rule.rs` (442), `aliases.rs` (544), the `room_details/` subpages | `RoomSettingsError` |
| 12 | Permissions, ACL, upgrade, moderation | 10 | 553 | `session/room/permissions.rs`, `server_acl.rs`, `upgrade.rs` | `session/room/permissions.rs` (733), `room_details/permissions/` (2,491), `upgrade_dialog/` (642) | `PermissionsError` |
| 13 | Calls | 9 | 671 | `session/calls/` | `session/calls/call.rs` (1,607), `mod.rs` (989), `turn.rs` (360) | `CallError` |

Group 2 also carries `session_settings` and its three setters, already
one-line passthroughs over `commune_core::settings` since leaf 4, and they
stay that way. Group 7 owns nine free functions — `collect_image_packs`,
`collect_enabled_packs`, `stored_packs_room`, `ensure_packs_room`,
`read_pack_content`, `send_pack_content`, `set_pack_enabled_inner`,
`read_account_data`, `parse_sticker_pack` — about 400 further lines. Group 8
owns `VerificationFlows` and `VerificationFlow`, about 200. Group 11 owns
`read_state_content`, `can_send_state`, `allow_room_ids`, `send_canonical`,
`build_upgrade_info` and `cmp_room_versions`, about 180, of which
`build_upgrade_info` is the one piece of genuinely intricate rule-following
in the file. Group 12 owns `CallFlow`, `CallFlows`, `InstalledHandlers`,
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
`ffi_send_state`, `ffi_thread_replies`, `ffi_history_event` — because they
are the FFI's own shape and nothing else consumes them; and the resolution
helper the preamble collapses into. Per the ruling already in this
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

Groups 10 and 11 are one subject split in two, because 1,112 lines is more
than one session should take on and they divide cleanly: the first is state
events read and written whole, the second is the power-level matrix and what
it authorises.

### What the read found

**`join_room` and `create_direct_chat` reimplement logic the core already
has.** `RoomList` exposes `join_by_id_or_alias`, `knock` and `direct_chat`;
the facade uses none of the three and walks `room_list().snapshot()` by hand
against the raw `Client` instead. This is the second-implementation problem
appearing _inside_ the core, which is a sharper version of the thing this
track exists to fix. Group 5 deletes the facade's copies.

**`search_gifs` and `fetch_gif_preview` touch no session at all.** They are
free functions wearing a method, and they become `#[uniffi::export]` free
functions beside `gif_search_available` in group 7.

**`forward_event` has no GTK precedent, and says so in its own doc
comment** — the application's Forward menu item is a stub whose action is
never registered. Under the mirror-the-GTK-sources rule that makes it a
no-precedent design, to be flagged rather than lifted: group 9 keeps it,
marks it, and leaves the question of what forwarding should send where it
belongs.

**The `sdp_stream_metadata_changed` receive handler is still missing**, as
the ledger records. Group 12 is the commit that rewrites the handler set, so
that is where adding it costs nothing extra — but the ledger assigns the
verification to Phase 4 module 9, and it needs the two-device check that
module carries. **Write the handler in group 12, verify it in Phase 4.**

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
| 1 Sep 2026 | `facade.rs` login flows | **The OAuth and SSO redirect is Android's, hardcoded.** `ANDROID_REDIRECT_URI` is `io.github.steeb-k.commune:/oauth2redirect`, and `oauth_client_registration_data()` builds a fixed native-application registration around it. The desktop application does not use a custom scheme at all: `src/login/local_server.rs` runs a loopback HTTP server and registers _its_ address, because a desktop browser has nowhere to send an app scheme. A GTK login through this core would open an authorization URL the browser could never come back from. The redirect and the registration are embedder facts, like `credential_label` and `klipy_api_key` before them, and belong in `CoreConfig`. | Phase 3, group 6 |
| 1 Sep 2026 | `facade.rs`, `set_push_gateway` | **The pusher describes an Android device, in English, whatever the embedder is.** `app_display_name` is `"Commune"` and `device_display_name` is `"Commune on Android"`, both literals; the `LEGACY_APP_ID` deletion that runs first cleans up after a specific Android debug build. The device name is what a user sees in another client's session list when they audit what is pushing to them, so a desktop session announcing itself as Android is wrong in the one place the string is read. Embedder values, `CoreConfig` again — and the legacy cleanup is Android's alone and should say so. | Phase 3, group 3 |
| 1 Sep 2026 | `facade.rs`, `check_upload_size` | **The upload-size refusal is a rendered English sentence, with a private byte formatter.** The core builds `"This file is too large, the homeserver takes up to {size}"` and formats the number with its own `format_size`. The application says the same thing at `src/session_view/room_history/message_toolbar/mod.rs:1310` as a `gettext_f` over `glib::format_size`. It is the most commonly hit error in the file — every oversized attachment, avatar and pack image goes through it — and it is a sentence, so it must not cross: the core owes a value (`UploadTooLarge { max_bytes }`) and the two embedders own the wording. The two formatters agree on decimal units, so the rendered text is identical today; only the translation is lost. | Phase 3, group 9 |
| 1 Sep 2026 | `facade.rs`, `ensure_packs_room` | **The packs room is created with an English name and topic.** `"Sticker Packs"` and `"The sticker and emoticon packs that you created. Invite someone here to share them."` are literals; `src/session/image_packs/mod.rs:627` wraps both in `gettext`. This one is worse than a lost error message, because a room name is not an error: it is written into `m.room.name` on the server, it shows in the sidebar next to the conversations, and it is _permanent_ — a user whose packs room was created by the Kotlin build keeps the English name after they translate their client, because nothing re-creates the room. Embedder-supplied strings, and the room the core makes should carry whichever the embedder passed. | Phase 3, group 7 |
| 1 Sep 2026 | `facade.rs` ignored users | **The core never followed the list, and never refused a redundant request.** `src/session/ignored_users.rs` subscribes to the SDK's ignore-list changes and re-reads `m.ignored_user_list` whenever one arrives; the facade read the account data once per call and had no subscription at all, so ignoring somebody from the desktop never reached a phone with the Ignored Users screen open — it would sit on a stale list until it was closed and reopened. The application also guards both directions: adding a user already on the list, or removing one that is not, is a warning and a no-op rather than a round trip the server will ignore. Neither guard existed in the core. **Closed 1 Sep** with Phase 3's first commit, which also found the thing the move would have broken: `SessionList::active_session()` returns a session before `prepare()` has run, so a cache-only read would answer "nobody" during startup where the old fetch answered correctly — `ensure_loaded()` keeps that guarantee. | Done |
| 1 Sep 2026 | `facade.rs`, `list_devices` | **Four divergences in one method, and the worst is what it does when something is wrong.** `src/session/user_sessions_list/` merges `/devices` with the crypto store, so a device known to one source and not the other is still listed; the facade walked `/devices` alone. The application lists what it has when one source fails and errors only when both do; **the facade returned an error the moment `/devices` failed, which is exactly the case a person opens the sessions screen in.** The application follows `devices_stream()` — taking an _empty_ update, because that is how a disconnection arrives without saying whose — and the facade fetched once per call. And the application breaks a sort tie on device ID where the facade had none, so devices the server never dated came back in a different order on every read. **Closed 1 Sep** in `session/user_sessions.rs`, with four tests over the ordering. | Done |
| 1 Sep 2026 | `facade.rs`, `sign_out_device` | **The core could not tell "wrong password" from "this homeserver wants something else".** Signing a device out goes through user-interactive authentication, which the application answers with an `AuthDialog` that speaks several stages; the facade retried once with a password whatever the homeserver had asked for, and reported the resulting failure as an ordinary error. On a homeserver whose sign-out stage is not `m.login.password` — an OAuth 2.0 one, for instance — that is a request that can never succeed and a message that never says so. **Closed 1 Sep**: the first attempt reads the offered flows, and `DeviceError` separates `NeedsPassword` from `UnsupportedAuth`. The GTK application hands the first to its dialog when the account settings migrate; until then no embedder is worse off, and the Kotlin one stops showing a sentence that is not true. | Done |
| 1 Sep 2026 | `facade.rs` account profile | **A profile change never reached the profile, and the application's source says in a comment why that is not academic.** `src/account_settings/general_page/mod.rs` updates its own copy of the display name and the avatar after a successful change, because _"if the user is in no rooms, we won't receive the update via sync"_ — an account in no rooms is never told about its own profile change. `set_display_name` and `set_account_avatar` wrote to the homeserver and touched nothing locally, so `Session::profile()`, the observable the GTK application will bind to, kept the old value until the process restarted. Separately, `account_profile()` called `fetch_user_profile()` past that same observable, so one core held two independently-fetched answers to the same question with nothing keeping them in step. **Closed 1 Sep**: both setters correct the observable — which is why `set_avatar` splits the upload from the avatar-URL write, as the application does, since `upload_avatar()` never hands back the URI the local copy needs — and `account_profile()` refreshes the observable and reads it. | Done |
| 1 Sep 2026 | `facade.rs`, `set_push_gateway` | **No push format is set, so the homeserver POSTs whole events to the push gateway — and narrowing it is not the core's call.** `src/utils/android_push.rs` sets `PushFormat::EventIdOnly` and its comment calls that _"mandatory for content, not merely preferred: events are E2EE, so a full payload would carry ciphertext at best — and the metadata that does not need to travel, still would."_ The facade leaves `format` unset, which the specification reads as "send everything": in an unencrypted room the gateway receives the sender, the room and the message body. **But the application can narrow it only because its Android notification path fetches the event by ID afterwards, and the Kotlin application posts straight from the payload** — `Push.kt` reads `type` to keep a call push from becoming a message notification (a bug its own comment records fixing), and `sender_display_name`, `room_name` and `content.body` for the text. Setting `EventIdOnly` in the core would empty every Kotlin notification and revive that bug with every gate still green; it was written, caught by reading `Push.kt`, and reverted. **Settled 1 Sep: keep the payload.** The notification arrives whole and instantly rather than waking a sync to re-fetch what already arrived, and the disclosure is accepted — an encrypted room gives up only metadata regardless. The method now carries the constraint that comes with that: narrowing the format later means changing `Push.kt` in the same commit. | Closed by decision |
| 1 Sep 2026 | `facade.rs`, `set_push_gateway` | **The obvious cleanup would unregister the user's other phones, and the application's source is what says so.** A pusher held under our application id with a different pushkey looks stale; it is usually another device. `android_push.rs` deletes only the endpoint that registration itself moved off, remembered in `State::previous_endpoint`, precisely because "the `app_id` is the same for every Commune on Android". The core has no such record and so removes nothing but the legacy application id, which is keyed on this device's own pushkey. **Left open deliberately**: an endpoint that changes without the old one being retired leaves a pusher the homeserver keeps POSTing to. Closing it wants somewhere to remember the previous endpoint, which is a design question rather than a transcription fix. | Open by decision |

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

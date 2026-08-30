# The Kotlin variant — plan

Decided 27 August 2026: Commune grows a native Android variant — a
Kotlin/Jetpack Compose UI over a **shared, headless Rust core extracted from
the code we already have**. The GTK application keeps every desktop platform;
GTK-on-Android has proven that the _stack_ can be made to work, but at the
cost of directly patching GTK4's IME layer (eight carried patches and
counting), and the list of things that need glue is not shrinking fast enough
to justify the next years of it.

This is the "Route B" that `android-plan.md` priced and declined in favour of
the GTK spike. The spike did its job: everything learned building it — the
Keystore secret backend, UnifiedPush, the notification pipeline, the TLS
stack, the sync service, the OAuth redirect — carries over to this plan
unchanged, exactly as that document predicted.

The work happens on the `fractal-kotlin` branch (worktree:
`../commune-kotlin`) so main can keep moving. Merge main into this branch
regularly, the way `android-port` did.

## Why "extract the core" is cheaper than the docs thought

`android-media-plan.md` wrote off `session/` — 27k lines of GObject models —
as "not portable to Kotlin". A measured pass over the tree (27 Aug 2026)
says the _encoding_ is GObject but the _content_ is not:

| Tier | Meaning | Lines |
|---|---|---|
| A | Pure Rust — no glib, no gio, no gtk | 6,134 |
| B | glib/GObject only (properties, signals, `MainContext`) — never links libgtk's widgets | 28,206 |
| C | GTK _data_ classes only (`FilterListModel`, `SortListModel`, `gdk::Texture`) | 15,291 |
| D | Real UI: widgets, libadwaita, Blueprint | 79,456 |

* Exactly **one file** under `session/` references `gtk::Widget`
  (`session/user_sessions_list/user_session.rs`). No `.blp` exists below
  `session/`, `secret/`, `session_list/`, or `utils/`.
* The models hold SDK types directly — `OnceCell<MatrixRoom>`,
  `Arc<EventTimelineItem>`, `Arc<SdkTimeline>` — so the GObject shell is a
  wrapper, not an entanglement. Removing it is mechanical-but-wide, not deep.
* The platform seams we need already exist and are already shaped right:
  `secret/` is a trait with five backends and zero GTK; notifications,
  system settings, TLS, and paths are all `cfg(target_os)` dispatch.

What the extraction actually consists of:

| Work item | Scope | Character |
|---|---|---|
| Lift `secret/` as-is | 2,200 lines | Trivial (drop one `glib::Boxed` derive) |
| Lift image-pack events, Matrix URI parsing, `tls.rs`, `http.rs`, URL previews | ~1,500 | Trivial |
| Replace 274 `#[glib::Properties]` with fields + a watch/notify abstraction | `session/`-wide | Mechanical, high line count |
| Replace `Filter/Sort/FlattenListModel` composition with plain collections / `VectorDiff` passthrough | `sidebar_data/`, `timeline/`, member lists | Delete more than write |
| Delete GTK list-model plumbing (`grouping_list_model/`, `expression*`, …) | ~6,500 | Deletion |
| Replace `spawn!`/`MainContext` with a dispatcher seam (keep `spawn_tokio!`) | 197 sites | Mechanical |
| Move `gettext` out of the models (semantic values out, strings rendered UI-side) | 156 sites | Medium, unpleasant |
| Extract state-event humanization out of `room_history/state/content.rs` | 864 lines | Misplaced today; worth doing regardless |
| Extract login/auth logic out of the `login/` widgets | ~800 of 3,290 | Medium; the SSO loopback server must be replaced on Android anyway |
| Re-point the JNI bridge from `gdk_android_display_get_env()` to `JNI_OnLoad` | ~40 lines | Small |
| The UniFFI facade | new code | The real design work |

Expected result: a **~25–30k-line `commune-core`**, of which 8–10k is
irreplaceable Commune logic — the secret storage format, the search merge
(server `/search` + local index), the push-rules vocabulary, notification
body construction, the verification state machine, permissions/roles, room
categorization, image-pack event types, the multi-account session list.

What is _not_ worth porting: everything that is a one-line call into
`matrix_sdk_ui` behind a property notification (pagination, receipts,
reactions, redactions, the failed-send retry). The core facade stays thin
over the SDK there; fattening it would just be a second SDK.

## Architecture

```text
┌────────────────────────┐   ┌──────────────────────────────┐
│  GTK UI (src/, .blp)   │   │  Kotlin UI (android-kotlin/) │
│  desktop platforms     │   │  Jetpack Compose, Material 3 │
└───────────┬────────────┘   └──────────────┬───────────────┘
            │ direct Rust use               │ UniFFI (Kotlin bindings)
┌───────────▼───────────────────────────────▼───────────────┐
│  commune-core  (commune-core/, headless)                  │
│  Client setup · sync loop · session list · rooms ·        │
│  timeline orchestration · secrets · search · push rules · │
│  notifications logic · verification · categorization      │
└───────────────────────────┬───────────────────────────────┘
                            │
              matrix-sdk / matrix-sdk-ui / ruma
              (same pinned revs as today)
```

Decisions:

* **`commune-core` is its own crate** at `commune-core/`, detached from the
  app's build (`[workspace]` tables kept separate) so main's Meson/Cargo
  world is untouched until the GTK app is ready to consume it. End state:
  the GTK app depends on `commune-core` too — that is what "shared" means —
  but the GTK migration trails the extraction rather than blocking it.
* **UniFFI 0.31.0 at the same mozilla git rev the SDK workspace pins**
  (`e5f4821410bea19e71984ea5e06a7bc8b11ed9e5`), so the facade and any use of
  `matrix-sdk-ffi` types stay on one FFI runtime.
* **Change propagation:** GObject `notify` becomes `tokio::sync::watch` (for
  scalar state) and `VectorDiff` streams passed through to the UI (for
  lists). Compose consumes them as `StateFlow`/`Flow` via UniFFI callback
  interfaces; GTK consumes them by rebuilding the thin GObject shells as
  pure _view-models_ over core types.
* **Threading:** the core owns a tokio runtime (as today). The UI-thread hop
  (`glib::MainContext` today) becomes a dispatcher seam; on Android the
  bindings already deliver callbacks off-thread and Compose state handles
  the rest.
* **Strings:** the core returns semantic values (enums, structured events),
  not English. The 156 `gettext` call sites in the model layer move to the
  UIs — GTK keeps gettext, Android uses string resources. This restores the
  translations that the GTK Android build lost entirely.
* **Android platform services stay on the Kotlin side:** Keystore crypto,
  notifications (`MessagingStyle`), foreground sync service, UnifiedPush
  receiver, OAuth redirect intent-filter, trust store. The Rust
  implementations written for the GTK port (`utils/android_*.rs`, JNI
  keystore backend) are the _specification_ — some carry over via JNI as-is,
  some are simpler re-done natively in Kotlin. Decide per module when its
  chunk comes up.
* **SQLite store:** `bundled` rusqlite, same as the GTK Android build. Same
  store schema, same pinned SDK — a debugging session on one variant
  reproduces on the other.
* **App identity: the same `io.github.steeb_k.commune`** (decided 27 Aug
  2026). Installing the Kotlin APK replaces the GTK build on a device, and
  **adopting the existing logged-in session in place is an explicit goal**:
  the store schema, the sealed-secrets format under
  `no_backup/commune/secrets.d/`, the Keystore alias
  (`commune.secrets.v1`) and the signing key must all stay compatible, so
  `install -r` upgrades from the GTK build straight into the Kotlin one
  without a login. On-device side-by-side comparison is traded away;
  compare via the emulator (which keeps the GTK build) against the device,
  or two AVDs.

  **Adoption-readiness, verified 27 Aug 2026** (statically, against the
  GTK build installed on the emulator): session discovery is a directory
  walk of `no_backup/commune/secrets.d/*.sealed` in both variants (the
  core's `secret/android` is the same module — GSettings only carries
  per-session preferences, with defaults on a miss); the Kotlin app's
  `noBackupFilesDir/commune` and `cacheDir/commune` equal the paths the
  GTK build demonstrably writes (`no_backup/commune/<id>/`,
  `cache/commune/<id>/`); the Keystore alias matches (`commune.secrets.v1`
  — the GTK build derives it from `CARGO_PKG_NAME` = `commune`); and both
  installed APKs carry the same signing certificate, so `install -r`
  is accepted. The live install-over is NOT run automatically: the GTK
  build on the emulator holds a session on a real homeserver
  (matrix.kzenjak.com), and replacing that install or syncing that
  account is the user's call. To run it: `adb install -r` the APK built
  without the `.skeleton` suffix, launch, and the sidebar should come up
  logged in.
* **`minSdk = 29`** (Android 10, decided 27 Aug 2026): scoped storage is
  the baseline, which simplifies every attachment/media path.
* **Material You dynamic color** (decided 27 Aug 2026): the Compose app
  derives its palette from the user's wallpaper rather than carrying the
  GTK accent — feeling native beats looking identical. Layout parity is
  unaffected; the bubble tint formulas from `doc/chat-bubbles.md` apply to
  whatever the scheme's accent is.
* **Chat bubbles on by default** (decided 27 Aug 2026): bubbles are the
  native messaging idiom on Android, so the Kotlin app inverts the GTK
  default; the flat style remains the setting's other value.

## The core stays transplantable

The point of the extraction is not one Kotlin app; it is a core that any
future front end — SwiftUI on iOS/macOS, WinUI, whatever comes — can pick
up whole. That is a contract, kept by construction:

* **No UI types cross the core's boundary.** No gtk/glib, no Android
  classes, no display assumptions. State is `eyeball` observables and
  `VectorDiff` streams; strings out are semantic values the UI words.
* **Every platform seam is already per-OS.** Secrets have five backends
  (Keystore, Keychain, Credential Manager, Secret Service, sealed files);
  paths, settings storage and connectivity notification are handed in by
  the embedder through `config::init()`; the JNI bridge is the one
  Android-only module and is `cfg`-gated.
* **The facade is generated per language from one definition.** uniffi
  produces the Kotlin bindings the Android app uses and — demonstrated
  27 Aug 2026 from the same built library — the Swift bindings
  (`commune_core.swift` + C header + modulemap) a SwiftUI app would
  import. C# for WinUI comes from the community `uniffi-bindgen-cs` on
  the same definitions; and a Rust UI (the GTK app, Track 3) skips the
  FFI and links the crate directly.
* **The GTK application is untouched until Track 3 chooses otherwise.**
  The extraction copies logic; `git diff main fractal-kotlin -- src po
  data meson.build build-aux hooks Cargo.toml Cargo.lock` is empty
  (verified 27 Aug 2026), and the app builds and tests green from this
  branch. Divergence risk runs the other way — main moving while copies
  age — which the merge cadence and Track 3 retire.

## The UI contract

The Kotlin app should _feel_ native (Material 3, predictive back, native
sheets and dialogs — no GTK-style modals) but _read_ like Commune: buttons
in the same places, messages laid out the same. The authority for "the same"
is the GTK app itself running on Android; reference screenshots (emulator,
seeded test session, 27 Aug 2026) are in `doc/kotlin-reference/`:

| Screenshot | Shows |
|---|---|
| `gtk-sidebar.png` | Sidebar: avatar/search/menu header, security banner, Explore row, sections (Server Notices, Spaces, Favorites, Rooms) with unread badges |
| `gtk-room-timeline.png`, `gtk-room-bubbles-clean.png` | Timeline: day divider, state events as dim inline text, sender header + timestamp, thread-reply chips, mention pills, **chat bubbles** (others accent-neutral left, own accent right with trailing avatar), two-row composer |
| `gtk-thread-view.png` | Thread mode: "Viewing a thread / Back to All Messages" banner over the same timeline |
| `gtk-primary-menu.png` | Primary menu: New Direct Chat / New Room / Join Room · Image Packs · Shortcuts / About |
| `gtk-room-details.png` | Room details as a sheet: avatar header, Edit Details, Members count row, Media/Files/Audio, notification radios |
| `gtk-composer-text.png` | Composer detail: attach · emoji · sticker · overflow on the action row; mic · entry · send on the entry row |

Layout translation rules (from the full widget inventory — 174 `.blp` files,
~20 destinations, ~16 structured dialogs, ~18 alerts, ~15 popovers):

* `Adw.NavigationSplitView` + the single 600sp `compact` breakpoint →
  Compose list-detail scaffold; phones get the back-stack behaviour the GTK
  app already has.
* `Adw.PreferencesDialog` (Room Details, Account Settings) → full-screen
  settings destinations with native subpage navigation.
* Popover menus → `DropdownMenu` where they are true menus, bottom sheets
  where they are pickers (sticker picker, reaction detail, context menus on
  long-press).
* `Adw.AlertDialog` confirmations (~18 sites, centralized in
  `message_dialogs.rs`) → Material `AlertDialog`, one-for-one.
* The composer's phone layout (auxiliary buttons re-parented to a second
  row — `MessageToolbar::constructed()`) is the _designed_ layout on
  Android; build it that way natively.
* Chat bubbles follow `doc/chat-bubbles.md` exactly: 12px radius,
  `currentColor` @ 8% for others / accent @ 25% for own, own bubbles keep
  name + avatar, timestamp outermost.

## Verifying the GTK application, every time

The extraction must never cost the desktop apps anything. The gates, run
against this branch (all green 27 Aug 2026):

1. **No source divergence.** `git diff main fractal-kotlin -- src po data
   meson.build meson.options build-aux hooks Cargo.toml Cargo.lock` must
   be empty until a Track 3 change deliberately says otherwise.
2. **The application builds.** `cargo check` from a UCRT64 shell in the
   worktree. Two generated files must be copied from the main checkout
   first, since Meson writes them and git ignores them:
   `src/config.rs` and `hooks/checks-bin.exe`.
3. **The application's lint gate.** `cargo clippy --all-targets -- -D
   warnings`, same shell.
4. **The pre-commit hook** runs on every commit here anyway — style,
   template checks, doc freshness, machete, deny, POTFILES, markdown.
   (`cargo test` is not one of the gates: on this machine the test
   binaries link against a mismatched libadwaita out of the gvsbuild and
   mingw64 prefixes, and they do so identically on main and on this
   branch — verified 27 Aug 2026 — so it measures the environment, not
   the code.)
5. When Track 3 begins moving the GTK app onto the core, the eyeball
   checklists (`doc/eyeball-tests.md`, `doc/eyeball-android.md`) become
   the acceptance suite per migrated module, exactly as they were for the
   ports.

## Chunks

Each chunk is a session-sized unit ending in something that compiles and is
demonstrable. Order matters only within a track; the two tracks interleave.

### Track 1 — the core extraction

0. ✅ _(this session)_ Branch, worktree, this plan, reference screenshots,
   `commune-core` skeleton crate pinned to the SDK's uniffi rev.
1. ✅ _(27 Aug)_ Lift the trivial tier: `secret/` (minus the Boxed derive),
   `image_packs/events.rs`, Matrix URI parsing, `tls.rs`, `http.rs`,
   `url_preview`, password validation. Bring their tests. `cargo test` in
   the core, msys2 toolchain as for SDK tests.
2. ✅ _(27 Aug)_ Foundations — the watch/notify abstraction turned out to
   already exist: the core adopted `eyeball`/`eyeball-im`, the SDK's own
   reactive primitives, at the workspace-pinned versions. Plus the
   `SettingsStore` seam (GSettings vs a JSON-file default), error
   types, `client_setup()` and the store layout, `StoredSession` +
   `session_list/` (multi-account discovery and ordering).
3. ✅ _(27 Aug)_ Sync: the sync loop with backoff and offline detection out of
   `session/mod.rs`; session-change/token watching; a headless `Session`.
4. ✅ _(27 Aug, v1)_ Rooms: `Room` minus GObject (43 properties → fields + watch), category
   and sidebar sectioning rules as pure functions over room state.
5. ✅ _(27 Aug: live timeline read/send/paginate, receipts and the
   read-state watcher, typing both ways, semantic membership sentences;
   27 Aug late: thread focus (`TimelineFocusKind`), thread reply counts,
   thread send, and `send_attachment` through the send queue;
   28 Aug: E2EE verified end-to-end against a matrix-nio client —
   both directions decrypt, `m.megolm.v1.aes-sha2` on the wire,
   `kotlin-e2ee-decrypted.png`; pinned timelines
   pending)_ Timeline: `matrix_sdk_ui::Timeline` orchestration with
   `VectorDiff` passthrough; the diff minimizer gets a generic sink trait
   instead of the GListModel one.
6. The rest of the logic tier as needed by UI chunks: permissions/roles,
   search merge, push-rules vocabulary, notification bodies, verification
   state machine, state-event humanization (extracted from
   `room_history/state/content.rs`).
7. UniFFI facade v1 (login, session, room list, timeline read + send) and
   the AAR build: `cargo ndk` for x86_64 + arm64, `uniffi-bindgen`
   generated Kotlin, packaged for Gradle.

### Track 2 — the Kotlin app

Starts after chunk 7; earlier screens can be built against fakes if a
session wants UI work sooner.

8. App skeleton: Gradle project at `android-kotlin/`, Compose + Material 3
   theme mapped to Commune's palette, list-detail navigation shell,
   login flow (greeter → homeserver → password/SSO → completed) —
   `local_server.rs` logic replaced by the intent-filter redirect the GTK
   port already registers.
9. Sidebar: section headers with collapse + badges, room rows (avatar,
   presence dot for DMs, unread dot / count), Explore row, account
   switcher, primary menu, search.
10. Timeline read path: message rows with the three-state header grouping,
    day dividers, collapsible state groups, typing row, read receipts,
    reactions display, thread chips, mention pills, chat bubbles.
11. Composer: two-row layout, reply/edit bar, send path, markdown toggle,
    @-mention completion, attach via Photo Picker/SAF.
12. Interactions: long-press context menu (bottom sheet) with the event
    action set, quick reactions, threads view, pinned view, in-room search.
13. Room details + account settings as native settings screens (the
    subpage inventory in the plan's companion report).
14. Verification (SAS emoji + QR via CameraX) and crypto/recovery setup.
    _(28 Aug: recovery done and round-trip verified; SAS emoji flows to
    the KeysExchanged stage against a scripted peer, the final confirm
    handshake awaits a real second client; QR scanning wired through
    zxing-android-embedded and the SDK's scan_qr_code. Full desktop
    feature parity is the standing requirement — no feature of the GTK
    app gets skipped, per the user, 28 Aug.)_
15. Notifications + background: UnifiedPush receiver, `MessagingStyle`
    notifications, foreground sync service fallback, Keystore-backed
    secret storage — porting the GTK port's Rust implementations or
    redoing them natively, per module.
16. ✅ _(27 Aug, v1: core media fetch to cache files, inline timeline
    images, room avatars with the direct-member fallback; 27 Aug late:
    fullscreen viewer with pinch-zoom/pan, attachment sending via the
    system picker; video/voice and history viewers
    pending)_ Media: images/blurhash placeholders, media viewer with
    zoom-from-thumbnail, video/voice playback (ExoPlayer — no GStreamer on
    this variant), attachments dialog, media history viewers.
17. The tail: spaces, explore/directory, invites and knocks, image packs,
    GIF search, URL previews, failed-send retry, report flows.

### Track 3 — convergence

**Underway since 29 August 2026. The plan of record is
`doc/track3-convergence.md`; read it rather than this section, which is kept
only for the history of how the work was priced.**

18. The GTK app consumes `commune-core` module by module, deleting its
    duplicated model layer; the doc ledgers get updated as each module
    moves (the per-feature docs are written against the current shape —
    budget for this, `AGENTS.md` treats them as load-bearing).

    What this chunk did not say, and what a measured pass over the crate
    found on 29 August: the application cannot consume the core while
    `facade.rs` holds the logic. `impl CoreApp` is 135 methods and about
    5,650 lines, and permissions, server ACLs, the upgrade rules, call
    signalling, image packs, verification and device management all live
    inside it rather than in `session/`. Decomposing it is a phase of its
    own and a hard prerequisite for every stateful module. The same pass
    settled four questions this section left open — full spine rather than
    the leaves alone, the GTK sources as the authority wherever the two
    disagree, the work staying on this branch, and convergence before the
    remaining parity gaps. All four are recorded in
    `doc/track3-convergence.md`.

## Risks and standing costs

* **Two UIs in lockstep** is the permanent tax this plan signs up for; the
  core makes behaviour shared, but every new feature now has two thin UI
  implementations. Accepted knowingly.
* **Merge cadence:** main keeps changing `session/` (gap-closing rounds).
  Until Track 3 lands, extraction copies logic while main edits the
  original. Merge main into `fractal-kotlin` often; extraction chunks
  should re-check their source modules against main at merge time.
* **The doc ledgers** describe the GObject shape; Track 3 invalidates parts
  of ~55 files. Update as modules move, not in one pass.
* **gettext extraction** is the ugliest mechanical step (156 sites) and
  changes GTK-side code; it lands in Track 3, not before.
* **APK size** should collapse versus the GTK build (no GTK/GStreamer/glib
  stack, no bundled asset tree), but measure once the AAR exists —
  matrix-sdk + crypto was ~81 MiB of the old `.text` on its own.
* **Emulator test bed:** the `seed_api35` AVD carries the seeded
  alice/bob homeserver session the eyeball runs use; it works unchanged
  for the Kotlin app, and `doc/eyeball-android.md`'s checklist is the
  eventual acceptance suite for parity.

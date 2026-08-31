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
designed against. The leaves go first. The `VectorDiff` half of the bridge
arrives with `session_list/`, the one leaf that holds state; the property half
arrives with `room_list/` and `Room`. What follows describes the bridge as a
whole; it is built in those two pieces.

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
`klipy.rs`, `password.rs`, the image-pack event types, `session_list/`. About
7,000 lines of duplication, at no behavioural risk: `src/secret/mod.rs` and its
core twin differed only by `pub(crate)`→`pub`, the `APP_ID`/`PROFILE` constants
becoming `config::app_id()`/`config::profile()`, and stripped `gettext`.
`commune_core::config::init()` is called from `src/application.rs` startup with
the Meson values, the GLib directories and a `GSettings`-backed
`SettingsStore`. `secret/` passes `None` for the settings store to begin
with, because nothing under it reads one; the `GSettings` implementation is
owed by the time `session_list/` moves.

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

**Phase 3 — decompose `facade.rs`.** The prerequisite that chunk 18 of
`doc/kotlin-plan.md` does not name. `impl CoreApp` runs from line 744 to 6391 —
135 methods, about 5,650 lines — and it holds permissions, server ACLs, the
upgrade rules, call signalling, image packs, verification and device management
_inside the UniFFI object_. The GTK application cannot consume that shape. Real
logic moves down into core modules with typed arguments and real error enums;
one-line SDK passthroughs (`kick_user`, `ban_user`, `set_member_power_level`,
`upgrade_room`) stay passthroughs on both sides, because a shared wrapper for
them would only be a second SDK. The Kotlin application is the regression suite
for this phase.

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

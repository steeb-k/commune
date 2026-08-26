# S5b — Real push on Android: context and a plan

Written 26 August 2026, before any of it is built. S5 established everything except arrival: a
channel, permission, a tap that survives process death, and a `dataSync` foreground service that
keeps the sync loop alive — for six hours a day, after which Android 15 calls `onTimeout()` and
Commune is deaf until it is next opened. This plan is about the remaining half: a message reaching
a frozen or dead Commune. Everything marked _measured_ was checked in the tree or on the emulator;
everything marked _unverified_ is a claim to test before relying on it.

## Contents

<!-- toc -->
* [The decision](#the-decision)
* [What push means for Matrix, precisely](#what-push-means-for-matrix-precisely)
* [What already exists that this builds on](#what-already-exists-that-this-builds-on)
* [The hard problem: the wake](#the-hard-problem-the-wake)
* [The step ladder](#the-step-ladder)
* [The Play store, later](#the-play-store-later)
* [Open questions](#open-questions)
<!-- /toc -->

## The decision

**UnifiedPush is the target, the foreground service stays as the fallback mode, and FCM waits for
the Play store.** The reasoning, so it does not have to be re-derived:

* **FCM** needs Play services on the device, a Firebase project, the `google-services` Gradle
  plugin — which pixiewood cannot express — and a push gateway (Sygnal) holding the Firebase
  credentials, running forever, on infrastructure of ours. The stated preference is less
  infrastructure until the app deserves it, distribution is sideload/F-Droid-shaped for now, and
  the development phone is a Pixel 9a on GrapheneOS, where Play services are at best sandboxed.
  Deferred, not rejected: see [the Play store section](#the-play-store-later).
* **UnifiedPush** is a small open spec: a distributor app on the device holds the one battery-cheap
  connection and hands every subscribed app an HTTPS endpoint. The decisive detail is that
  **ntfy's server embeds a Matrix push gateway** — the endpoint itself answers
  `/_matrix/push/v1/notify` — so with ntfy as the distributor there is no infrastructure for us to
  run at all. It is what Element X, FluffyChat and Molly do for the no-Google case, and it is the
  native answer on GrapheneOS.
* **The foreground service** already works, against any homeserver including one with no push
  gateway configured, and costs nothing to keep. It becomes the mode used when no distributor is
  installed or the user prefers it — the two-mode model Molly ships. If its six-hour cap ever
  needs lifting for the sideload build, `FOREGROUND_SERVICE_TYPE_SPECIAL_USE` has no timeout
  (_unverified_, and the Play store requires a justification declaration for it — a reason to not
  reach for it casually).

`android_notifications` takes no position on where a notification came from, which was the point
of building it first; nothing in it changes here.

## What push means for Matrix, precisely

The homeserver does not talk to distributors. The client registers an **HTTP pusher**
(`POST /_matrix/client/v3/pushers/set`) whose `data.url` names a **push gateway**, and on every
notifying event the homeserver POSTs to that gateway's `/_matrix/push/v1/notify`; the gateway
forwards to the device. With ntfy the endpoint and the gateway are the same URL. `matrix-sdk`
already wraps the API — `Client::pusher()` with `set()` and `delete()`
(`crates/matrix-sdk/src/pusher.rs`, _measured_ in the pinned checkout).

Because every room that matters is E2EE, the pusher is registered with
`data.format = "event_id_only"`: the push carries a room ID and an event ID and **no content**.
Content, decryption and the decision whether to show anything at all happen on the device, after
the wake. That is the hard problem, and it has [its own section](#the-hard-problem-the-wake).

Gateway discovery, per the UnifiedPush spec: probe the endpoint for
`/_matrix/push/v1/notify` answering `{"unifiedpush": {"gateway": "matrix"}}` (_measured_ against
ntfy.sh in step 0, byte for byte). If the user's distributor has no Matrix gateway behind it, we
do not silently route their metadata through a third-party public gateway; we say so and fall back
to the foreground service. ntfy-first UX makes that the rare case.

## What already exists that this builds on

_Measured_, all of it, in S3–S5:

| piece | where | what push reuses |
| --- | --- | --- |
| Posting, replacing, withdrawing | `utils/android_notifications.rs` | unchanged; tags are deterministic (`<id>//<room-uri>/e/<event>`), so the same event posted twice replaces itself rather than duplicating |
| Tap through process death | intent-filter + `onNewIntent` + `process_uri()` | unchanged |
| A Java class patched in beside GTK's glue | `patch-gtk-service.sh`, `SyncService.java` | the receiver arrives the same way |
| Manifest additions | `patch-manifest.sh` | grows a `<receiver>`, and `<queries>` for distributor visibility |
| JNI from Rust | `utils/android.rs` | grows a second init path (see the wake) |
| Session data out of GTK's reach | `no_backup/commune` | what the headless process restores from |
| The foreground service | `utils/android_sync_service.rs` | becomes mode two of two |
| Single-event fetch + decrypt | `matrix_sdk_ui::notification_client` | already compiled — the module is not feature-gated (_measured_ in the pinned checkout) |

## The hard problem: the wake

A push arrives as a broadcast to a process that is usually dead. Three layers of problem, in
order of certainty:

**Getting native code running without `main` — impossible, and unnecessary.** This plan first
assumed the glue loads `main` from the `Activity` and a receiver would need a JNI entry of its own
that bypasses it. Step 0 read the glue and the assumption is wrong: `RuntimeApplication.onCreate`
calls `startRuntime()` (`gdkandroidruntime.c`), which dlopens the application library, spawns the
GTK thread, **calls `main` on it, and blocks `onCreate` until the GLib event loop is running** —
on _every_ process start, an `Application.onCreate` for a broadcast included (_measured_ in the
source, not yet on the emulator). So by the time `onReceive` runs, the full application is up with
its main loop live and `android.rs` initialized the ordinary way; the receiver's only JNI job is
handing the message across, and `writeResources()` has already run (harmless since S5 moved
everything to `no_backup`, but it is a cost every wake pays).

What that trades away is lightness — a push wake boots all of Commune — and what it leaves open is
what a window-less start does: with no `Activity`, does anything fire `activate` and try to
present a window (Android blocks background `Activity` starts since 10, so the attempt would be
discarded — but what does GDK do with the refusal?), or does the application idle in its main loop
under the glue's `g_application_hold` with no session restored? The first measurement of step 3,
gated only on step 1's receiver existing. Budget-wise the spec helps: the distributor raises the
app to foreground importance for five seconds around delivery, and a connector may expose a
`RAISE_TO_FOREGROUND`-bindable service when it needs longer; if neither is enough for a cold boot
plus a `/context` query, the escalation path is a platform `JobService` (no dependency), not
WorkManager (an AAR pixiewood cannot add).

**Turning an event ID into a notification, headlessly.** Two candidate strategies, and this is
the real fork in the plan:

* **(A) One bounded classic sync.** Restore the session from the store, `sync_once` with a short
  timeout, and feed `response.notifications` through the existing pipeline. Server-agnostic, and
  the to-device messages carrying room keys arrive through the same sync that delivers the event —
  decryption needs no separate machinery. The cost: the existing formatting lives in
  `session::notifications::Notifications`, a GTK object hanging off `Session`; this strategy needs
  the title/body/avatar formatting extracted into something a headless process can call, and an
  incremental sync after hours frozen may be heavy.
* **(B) `NotificationClient`.** Purpose-built for exactly this — Element X ships on it. Restore a
  plain `Client`, `NotificationClient::new(client, …)`, `get_notification(room_id, event_id)`,
  and back comes a display-ready `NotificationItem` (sender display name, avatar, computed push
  actions, decrypted content). It tries sliding sync first and falls back to a `/context` query
  (_measured_ in the source); the open question is decryption when the room key has not arrived
  yet — its retry leans on the sliding-sync encryption extension (_unverified_ how it behaves
  against a homeserver without simplified sliding sync).

The plan bets on **(B) first**, measured against `matrix.org` (which has simplified sliding sync)
and against a homeserver that does not, with (A) as the fallback if decryption-on-wake fails where
sliding sync is absent. Either way the wake runs on its own tokio runtime, posts through
`android_notifications`, and exits; the deterministic tags mean a Commune that was actually
foreground and synced the same event merely replaces the notification.

`NotificationProcessSetup` wants `SingleProcess { sync_service }` on Android — an
`Arc<matrix_sdk_ui::sync_service::SyncService>` Commune does not have, since it runs its own sync
loop. Whether a throwaway `SyncService` that is never started satisfies it, or `MultipleProcesses`
(a cross-process store lock, designed for iOS but honest about our situation too) is the safer
declaration, is a step 3 measurement, not a guess to build on.

## The step ladder

Each step is independently measurable, and the first one needs no code at all.

**Step 0 — prove the server side with `curl`, and read the ground. Done, 26 August 2026.** The
whole server-side chain ran with `curl` alone, and no real account was touched: the throwaway
Synapse from `testing/local-homeserver.sh` (podman is now installed on the WSL Arch build host for
it), alice given an HTTP pusher via `pushers/set`, bob speaking in their DM, and the notify
arriving at a `curl` subscriber on the ntfy topic seconds later. What the wire actually carried:

```json
{"notification":{"counts":{"unread":3},"devices":[{"app_id":"io.github.steeb_k.commune.android",
"data":{"format":"event_id_only"},"pushkey":"https://ntfy.sh/upqBLho454KJ5w?up=1",
"pushkey_ts":1787770567}],"event_id":"$9N8Sge…","prio":"high","room_id":"!NULGBt…:localhost"}}
```

_Measured_, each previously a claim: gateway discovery on ntfy.sh answers exactly
`{"unifiedpush":{"gateway":"matrix"}}`; the pusher registers and lists back; `event_id_only` is
honored — event ID, room ID, counts and priority, no content; and **the payload reaches the
subscriber as plaintext JSON**, so the receiver parses `bytesMessage` directly and RFC 8291
decryption is not needed on the ntfy path (spec AND_3.1.0 allows encrypted WebPush in general —
handle "starts with `{`" and "aes128gcm" as two arms if another gateway ever matters).

Two ntfy server rules worth knowing, read out of `server.go` after the 507s they caused:
subscriber-based rate limiting only attaches to topics named `up` + 12 characters (exactly 14 — the
distributor names topics, so never our code's concern), and publishing to a UP topic with no
active subscriber is refused with 507 — which the Matrix gateway converts, once the topic has been
quiet long enough, into **rejecting the pushkey, so the homeserver deletes the pusher itself**. A
dead subscription self-cleans server-side; the client still handles `UNREGISTERED` for the local
half.

The spec reading (AND_3.1.0) pinned the handshake for step 1: the app broadcasts
`org.unifiedpush.android.distributor.REGISTER` / `…UNREGISTER` to the distributor with a `token`
(UUIDv4, unique per registration — multi-account is answered: one token per session); identity
travels via `FLAG_SHARE_IDENTITY` on SDK ≥ 34 and a `PendingIntent` extra below; the distributor
answers on the app's receiver with `org.unifiedpush.android.connector.NEW_ENDPOINT` (`token`,
`endpoint`), `MESSAGE` (`token`, `bytesMessage`, and an `id` that must be answered with
`MESSAGE_ACK`), `REGISTRATION_FAILED` (`reason`, including `VAPID_REQUIRED`), `TEMP_UNAVAILABLE`,
and `UNREGISTERED` (optionally naming a `useDistributor` to move to). Distributor _discovery_ is
not a receiver query: a distributor exposes an activity on the `unifiedpush://link` deep link, so
the `<queries>` entry matches that `VIEW` intent and selection can go through
`startActivityForResult`.

**Step 1 — the receiver, and an endpoint in the log.** `PushReceiver.java` copied in by a new
patch script exactly as `SyncService.java` is; `<receiver>` and `<queries>` added by
`patch-manifest.sh`; the registration handshake with the distributor (ntfy from F-Droid on the
emulator — no Play services needed). Measured when: the endpoint string reaches the Rust side and
survives a reinstall via `NEW_ENDPOINT`.

**Step 2 — the pusher, registered by Commune.** On receiving an endpoint: probe for the gateway,
`Client::pusher().set()` with `event_id_only`, re-register on `NEW_ENDPOINT`, delete on logout,
fall back to the service on `UNREGISTERED` or a failed probe. Measured when: the pusher is visible
in `GET /pushers`, and a message sent to a **frozen** Commune produces a broadcast in logcat —
even though nothing is posted yet.

**Step 3 — the wake.** First measurement: what a broadcast-only start does with no `Activity` —
whether anything fires `activate` or tries to present a window, and whether the application idles
usably in its main loop (step 0 established `main` runs regardless; see the wake section). Then:
a "started for push" path inside the ordinary application — session restore without a window,
strategy (B) with (A) held in reserve, the `NotificationProcessSetup` question answered, and the
receiver handing `room_id`/`event_id` across JNI to the running main loop. Measured when: with
Commune force-stopped, a real message posts a real notification with the sender's name and the
decrypted body, and tapping it opens the conversation — the S5 measurement, repeated with the
process dead the whole time.

**Step 4 — one mode at a time.** Push mode and service mode become an explicit setting: with a
working pusher the foreground service does not run; losing the pusher falls back. The setting
lives in GSettings, per the Android build.

**Step 5 — the first-time-setup screen.** A person installing Commune should not have to know any
of the above. An Android-only setup page, shown once after the first session exists (the natural
hook is where `present_main_window()` already calls `android_notifications::init()`), that does
three things in order:

1. Says why notifications matter for a chat app and lets the `POST_NOTIFICATIONS` prompt make
   sense instead of arriving cold — the request moves here from `init()`.
2. Detects a distributor (a `PackageManager` query for activities on the `unifiedpush://link`
   `VIEW` intent — this is what the `<queries>` entry earns; pinned by step 0). If one is present:
   offers to set push up, one tap. If none: **recommends installing ntfy**, with a link out
   through the existing `launch_uri()` — to Play or F-Droid, whichever is installed — and a plain
   explanation that without it, delivery pauses after six background hours a day.
3. Offers "just keep Commune running" (the foreground service) as the no-extra-app choice, so
   declining ntfy is a decision rather than a dead end.

The same choices live permanently in notification settings, because the person who taps through a
setup screen is not the person who later installs ntfy — re-detection on return to settings makes
switching modes a toggle, not a reinstall. Measured when: a fresh install with no distributor
recommends ntfy and lands in service mode; installing ntfy and revisiting settings flips to push
mode and step 3's measurement passes.

## The Play store, later

Written down now so the door stays open, deliberately not built now:

* **FCM needs a gateway of ours.** However it is wired on the client, the homeserver must POST to
  something holding the Firebase server credentials — that is Sygnal, small and stateless, but
  permanent infrastructure. There is no FCM without it.
* **The client-side shape worth keeping in mind** is Molly's: an _embedded_ FCM-backed
  distributor, so FCM is just another UnifiedPush distributor and steps 1–4 stay the only code
  path. The alternative — a parallel native FCM pusher — means a second registration path and the
  `google-services` Gradle problem in full.
* **Policy notes:** the `dataSync` service as shipped is fine for Play; `specialUse` would need a
  declaration; and the setup screen's ntfy recommendation is unremarkable there (FluffyChat does
  the same).

None of this changes steps 0–5, which is the point of the embedded-distributor shape.

## Open questions

Answered by step 0, kept for the record: the spec version is AND_3.1.0 and the handshake is pinned
in the step 0 notes; multi-account is one token (and so one pusher) per session, which is the
spec's multiple-registrations design. Still open:

* What a window-less application start actually does — `activate`, window attempts, or a quiet
  main loop — and where session restore hooks when no window ever appears (step 3's first
  measurement).
* Whether the five seconds of foreground importance around delivery cover a cold boot of the full
  application plus a `/context` query and a decryption retry, or step 3 needs the
  `RAISE_TO_FOREGROUND` service or a platform `JobService` (measure with the process force-stopped
  and the network slow).
* What `NotificationClient` does about an undecryptable event against a homeserver without
  simplified sliding sync — the (A)/(B) fork.
* Whether the six-hour service and push mode ever need to coexist (a distributor that flakes), or
  whether fallback-on-`UNREGISTERED` plus the gateway's own pushkey rejection is enough.
* Which spec versions the ntfy distributor app actually speaks on-device — the AND_3.1.0 `LINK`
  discovery against the shipping ntfy APK is a step 1 measurement.

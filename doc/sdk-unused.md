# What the pinned SDK carries that Commune does not surface

An audit taken on 26 August 2026 of the pinned matrix-rust-sdk (rev
`db02d6c`, `Cargo.toml:88-115`): every public feature surface of
`matrix-sdk` and `matrix-sdk-ui` was walked and each capability grepped for
in `src/`. This is the record of what the SDK already implements that this
client never calls — the shopping list future gap-closing rounds draw from,
and the reason some rounds are cheaper than the ledgers price them.

Things already planned, excluded on the stance, or done are not repeated
here; `doc/gap-closing-plan.md` carries those decisions. SDK paths are
relative to the checkout (`~/.cargo/git/checkouts/matrix-rust-sdk-*/db02d6c`).

## Corrections this audit forced

* **Push rules are more surfaced than the comparison page says.**
  `Client::notification_settings()` is substantially used already —
  `add_keyword` / `remove_keyword`, per-room modes, enabling and disabling
  rules, deleting room rules all have call sites. Before round 5 is scoped,
  check what the settings UI actually draws; the ◐ on
  `client-comparison.html` ("the underlying push rules are not exposed") may
  be underselling it. What is confirmed unreferenced: `unmute_room`,
  `create_custom_conditional_push_rule`, `set_underride_push_rule_actions`,
  `contains_keyword_rules`, `is_push_rule_enabled`, `ruleset()`,
  `subscribe_to_changes()` (`matrix-sdk/src/notification_settings/mod.rs`).
* **Invite by email is cheaper than the spec-gaps page prices it.**
  `Room::invite_user_by_3pid()` (`matrix-sdk/src/room/mod.rs:2019`) exists
  ready-made. The "identity server flow" cost is identity-server
  _configuration_ UI, not protocol work.
* **Answering a knock is implemented, and the first draft of this audit was
  wrong to say otherwise.** The members page lists knocking members and the
  user page accepts or denies them through the normal membership operations
  (`src/components/user_page.rs:359-405`), which is what the spec intends —
  the SDK's `KnockRequest` wrapper is a convenience over the same calls, not
  a different protocol. What is actually missing is the _surfacing_ half:
  nothing notifies about a pending knock or remembers which were seen
  (`Room::subscribe_to_knock_requests`, `KnockRequest::mark_as_seen`,
  `decline_and_ban` — `matrix-sdk/src/room/knock_requests.rs`). You find out
  a knock exists by opening the members page.

## Strong candidates — on spec, the SDK does the heavy lifting

* **Per-message encryption shields.** `EventTimelineItem::get_shield()`
  (`matrix-sdk-ui/src/timeline/event_item/mod.rs:377`) puts a trust verdict
  on every timeline item — unverified device, unsigned sender, unencrypted
  event in an encrypted room. The app has **no per-message trust indicator
  at all**, only the session-level `SessionVerificationState`. The verdict is
  already computed on every item we draw; the work is a small icon and its
  tooltip. Related and equally unread: `UtdCause`
  (`event_item/content/mod.rs:541`) says _why_ a message cannot be decrypted
  (sent before you joined, withheld, verification violation) — the app draws
  a generic placeholder and matches `MsgLikeKind::UnableToDecrypt(_)` with
  the payload bound to `_`
  (`src/session_view/room_history/message_row/content.rs:323`). The trust bar
  behind the shields is configurable via
  `ClientBuilder::with_decryption_settings` and never set.
* **Retry and discard for failed sends.** The send queue is enabled and
  errors are subscribed per room, but a failed message only _shows_ a state:
  `EventTimelineItem::local_echo_send_handle()` →
  `SendHandle::{unwedge, abort}` (`matrix-sdk/src/send_queue/mod.rs:2778,
  2902`) is the retry/discard affordance, and `SendQueue::local_echoes()`
  (`:310`) is an outbox listing. Offline sending exists but cannot be acted
  on.
* **Knock notifications** — the surfacing half described above.
* **Attachment captions, and upload progress.** `AttachmentConfig` carries
  `caption`, `mentions` and `in_reply_to`; the app sends
  `AttachmentConfig { info, thumbnail, ..Default }`
  (`src/session_view/room_history/message_toolbar/mod.rs:1079`). Progress is
  `SendAttachment::subscribe_to_send_progress`
  (`matrix-sdk-ui/src/timeline/futures.rs:53`) plus
  `SendQueue::enable_upload_progress`. Editing a caption afterwards is
  `EditedContent::MediaCaption` (`matrix-sdk/src/room/edit.rs:47`); only
  `EditedContent::RoomMessage` is used today.
* **Room privacy settings.** `Room::privacy_settings()`
  (`matrix-sdk/src/room/privacy_settings.rs`) does canonical-alias editing,
  publishing and removing aliases in the room directory, room visibility,
  and history-visibility editing. None is surfaced; alias handling is
  reimplemented by hand in `src/session/room/aliases.rs`.
* **Structured roles and permissions.** `RoomMemberRole`,
  `Room::apply_power_level_changes()`, `reset_power_levels()`,
  `get_suggested_user_role()`, `users_with_power_levels()`
  (`matrix-sdk/src/room/power_levels.rs`, `room/mod.rs:2909-2950`) — an
  Admin/Moderator/User permissions page instead of raw
  `update_power_levels`.
* **Cross-room search.** The `experimental-search` feature is enabled on
  both crates, but only the raw index is used, with a fixed result cap
  (`src/session/room/search.rs:409`). Unused on top of the same index:
  `Room::search_messages()` (paginated stream),
  `Client::search_messages()` (cross-room, relevance-ordered, DM filters —
  `matrix-sdk/src/message_search.rs:345`), and the reactive
  `matrix_sdk_ui::search_service::SearchService` built exactly for a UI.

## Smaller wins, each about a sitting

* `Recovery::is_last_device()` — warn before logging out the last device,
  which is a data-loss guard (`matrix-sdk/src/encryption/recovery/mod.rs:566`).
* `Client::get_dm_room()` — find the existing DM before creating another
  (`client/mod.rs:1963`).
* `Timeline::send_multiple_receipts(Receipts)` — the app sends read,
  private-read and fully-read markers as separate requests
  (`src/session_view/room_history/mod.rs:1586,1608`).
* `Client::load_or_fetch_max_upload_size()` — pre-flight a large attachment
  instead of letting the server refuse it after the upload.
* **A storage settings page**: `Client::get_store_sizes()` /
  `optimize_stores()` (`client/mod.rs:3760-3776`), media-cache
  `Media::clean()` and per-item eviction, and a configurable
  `MediaRetentionPolicy` — currently hard-coded to `default()`
  (`src/session/mod.rs:794-800`).
* `Media::get_media_file()` — decrypted media as a temp _file_ rather than
  `Vec<u8>` in memory (`matrix-sdk/src/media.rs:355`); relevant to large
  video playback and Save As.
* Tombstones: `predecessor_room` is never read, so an upgraded room offers
  no "continue reading in the old room" link
  (`matrix-sdk-base/src/room/tombstone.rs:69`). Note the SDK has no
  `Room::upgrade()`; upgrading a room is ruma-only (`upgrade_room::v3`) and
  also absent.
* `Room::set_own_member_display_name()` — per-room nickname
  (`room/mod.rs:1239`).
* `Timeline::retry_decryption()` plus
  `Encryption::room_keys_received_stream()` — retry UTDs when keys arrive
  instead of waiting for the next repaint.
* `EventTimelineItem::contains_only_emojis()` — jumbo-emoji rendering.
* Backup polish: `Backups::wait_for_steady_state` (upload progress),
  `download_room_key` (a manual "try again from backup"),
  `Recovery::recover_and_fix_backup`.

## Out on the stance, now inventoried

Off spec, none a tack-on; recorded so nobody re-audits to find them:

* Live location sharing (MSC3489) — `start_live_location_share`,
  `LiveLocationsObserver`; the app sends static `m.location` only.
* Dehydrated devices (MSC3814) — `encryption/dehydrated_devices.rs`.
* Key sharing on invite (MSC4268) — `with_enable_share_history_on_invite`.
* Galleries (MSC4274) — `Timeline::send_gallery`.
* Extended profiles (MSC4133), profile status (MSC4175), call status
  (MSC4426) — `account.rs:398-528`.
* Identity pinning and verification-violation banners —
  `subscribe_to_identity_status_changes`, `UserIdentity::{pin,
  withdraw_verification, has_verification_violation}`; the "invisible
  crypto" family.
* Server-synced recent emoji (`io.element.recent_emoji`,
  `account.rs:1393`) — vendor namespace; our recent-emoji feature is local
  on purpose (`doc/recent-emoji.md`).
* Recently visited rooms (`im.vector.setting.breadcrumbs`) — vendor.
* MatrixRTC discovery (`rtc_transports`, `rtc_foci`) and call-decline
  events — with MSC4143 on the watch list.
* Preallocated media URIs (MSC3870), `MediaFetcher` /
  `matrix-sdk-contentscanner` media proxying.
* `SpaceService` (`matrix-sdk-ui/src/spaces/`) — a reactive space graph
  that arrived after our spaces module was built by hand. Ours works and is
  seen; rework would be motion without progress. Worth a look only if a
  space bug ever traces to our own graph-walking.

## Housekeeping

* The `socks` cargo feature is enabled and `ClientBuilder::proxy()` is
  never called — proxy support is compiled in with no way to turn it on.
  Expose a proxy setting or drop the feature.
* `Client::pause()` / `resume()` exist for suspend/sleep; the app leaves
  sync running through laptop sleep and reconnects on error instead.
* `Client::get_or_upload_filter()` — server-side sync filters are the main
  lever on classic-sync payload size and are unused; moot once the 2.0
  sliding-sync round lands, relevant if it slips.

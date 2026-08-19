# Image packs (MSC2545) — downstream implementation notes

Downstream feature branch. Not intended for upstream (see CONTRIBUTING.md §
"Generative AI"). This file is the ledger for the work: design decisions,
every integration point into existing code, and the rebase guide for
re-applying the branch to new Fractal releases.

## Scope

* Send `m.sticker` from a picker in the message toolbar.
* Unblock replying and reacting to stickers.
* Render `<img data-mx-emoticon>` custom emoticons inline in messages.
* `:shortcode:` completion in the composer, sending emoticons in
  `formatted_body`.
* Pack management UI: the personal pack, the packs enabled globally, and the
  packs of a room.
* Pack authoring (create/edit packs, upload images) — after consumption
  works end to end.
* Space pack inheritance — last phase.

## What the specification actually says

Image packs landed in Matrix 1.19. Two events, and only two:

| Event                | Kind                | Content                          |
| -------------------- | ------------------- | -------------------------------- |
| `m.room.image_pack`  | state, key = pack id | `images` map + optional `pack`   |
| `m.image_pack.rooms` | global account data | `rooms: {room → {state key → {}}}` |

Points that shaped the code:

* **There is no personal pack event.** MSC2545 had `im.ponies.user_emotes`;
  it was not carried into the specification, which expects a personal pack to
  be a room pack enabled globally instead. Deployed clients still use it, so
  we support it, under the unstable name only.
* **`usage` is a property of a pack, not of an image.** An image has only
  `url`, `body` and `info`. An absent or empty `usage` means every usage.
* **The objects in `m.image_pack.rooms` are opaque.** Clients must preserve
  the properties they do not know about, so they are round-tripped.
* Shortcodes are `[A-Za-z0-9_-]{1,100}`, case-sensitive. Malformed ones are
  still rendered, so that users can fix them; the grammar is only enforced
  when editing a pack.
* Pack order: the packs enabled globally, then the packs of the room, then
  the packs of its canonical space hierarchy.
* A pack absent from a room the user has left must be reported, not hidden.

## Wire format

The specification uses `m.*`; deployed clients use `im.ponies.*`. We read both
and send `im.ponies.*`.

| Purpose            | Send (unstable)         | Also read (stable)   |
| ------------------ | ----------------------- | -------------------- |
| Personal pack      | `im.ponies.user_emotes` | _(does not exist)_   |
| Room pack          | `im.ponies.room_emotes` | `m.room.image_pack`  |
| Enabled room packs | `im.ponies.emote_rooms` | `m.image_pack.rooms` |

ruma ships the two stable events as ungated types (the MSC graduated, so
there is no `unstable-msc2545` feature on the pinned revision). They are not
used: they cover neither the unstable names nor the personal pack, and
`RoomImagePackMeta` drops the unknown properties that we must preserve. Only
`ImageInfo` is reused, because `m.sticker` is defined in terms of it.

`events.rs` therefore defines one content type per wire name, with
`#[derive(EventContent)]`, over a shared `PackContent` / `EnabledPacks` body.
**One type per name, not one type with an `alias`**: an alias only affects
deserialization, while both the state store and the event handlers of the SDK
key on the single `StaticEventContent::TYPE` of a content type. Reading a name
means looking it up, and watching a name means registering a handler for it.

Two notes on the derive: it resolves its paths through the `ruma` facade, so
it works downstream; and a state event content cannot use a bare
`#[serde(flatten)]`, because the generated possibly-redacted type rejects it —
`#[serde(default, flatten)]` is accepted.

## Architecture

New code lives in dedicated directories; edits to existing files are kept
small and listed in the ledger below.

* `src/session/image_packs/` — model layer.
  * `events.rs`: the five content types and the shared bodies, with the
    shortcode grammar and serde tests for both identifier sets.
  * `pack_image.rs`: `PackImage`, one image of a pack. Builds the
    `m.sticker` content, applying the `body` → shortcode and `info` → `{}`
    fallbacks.
  * `image_pack.rs`: `ImagePack`, a pack and where it came from
    (`ImagePackSource`), exposing its images as a `gio::ListStore` sorted by
    shortcode, and its name falling back to the name of its room.
  * `mod.rs`: `ImagePacks`, one per session. Loads the account data, watches
    it under both names, and assembles the packs for a room in specification
    order, dropping duplicates between the packs enabled globally and the
    packs of the room.
* `src/session_view/room_history/message_toolbar/sticker_picker/` — picker
  popover. Images through the existing pipeline (`ThumbnailDownloader` /
  `IMAGE_QUEUE`); small-image widget modeled on
  `src/components/avatar/image.rs`.
* `src/account_settings/image_packs_page/` — personal pack and packs enabled
  globally. Template: `safety_page/` and its `ignored_users_subpage/`.
* `src/session_view/room_details/image_packs_subpage/` — the packs of a room.
* `src/components/custom_emoticon.rs` — `CustomEmoticon`, an image sent
  inline in a message. Sized from the font metrics rather than from the
  `height` attribute, which the specification only requires for the clients
  that do not support image packs.

An emoticon in a received message is an arbitrary image from the homeserver,
so it is only loaded when
`GlobalAccountData::should_room_show_media_previews` allows it, and falls
back to its description otherwise. Only an `mxc:` source is ever loaded,
which ruma enforces by leaving `ImageData::src` unset for anything else.

## Phases

Each phase compiles, passes clippy/fmt/nextest, and is usable on its own.

1. **Model** — content types, `ImagePacks`, `ImagePack`, `PackImage`,
   account data watchers, serde tests. _(done)_
2. **Sticker basics** — `can_send_sticker`; unblock the reply and react
   gates. _(done)_
3. **Sticker picker** — toolbar button, popover, send
   `AnyMessageLikeEventContent::Sticker` through `matrix_timeline.send()`.
   _(done)_
4. **Emoticon rendering** — allow `img` in the sanitizer, inline widget via
   `LabelWithWidgets`, tests. _(done)_
5. **Emoticon sending** — `:shortcode:` completion, inline widget in the
   composer, serialization in `composer_parser.rs`. See the note below, a
   decision is needed first.
6. **Pack management** — account settings page, room details subpage,
   enabling and disabling packs globally.
7. **Pack authoring** — create and edit packs, upload images, edit
   shortcodes and usage; the packs of a room gated on the power level.
8. **Space packs** — canonical space hierarchy, recursive, with a depth
   limit and a cycle guard, slotted into the order in `packs_for_room`.

## Open decision: how to complete `:shortcode:`

`CompletionPopover` is 769 lines built around one abstraction: its rows are
`PillSourceRow`s bound to `PillSource`s, and activating one inserts a `Pill`
into the composer. It handles the buffer scanning, the word boundaries, the
key navigation and the popover placement, none of which is specific to
mentions.

Two ways to get emoticon completion out of it, and they pull in opposite
directions:

1. **Make an emoticon a `PillSource`.** A shortcode is the display name and
   the image is the avatar, so the rows, the activation and the insertion all
   work unchanged. Perhaps 350 new lines: the source, a list, a `:` sigil and
   a `SearchTermTarget`. The composer then holds a `Pill`, which
   `composer_parser.rs` tells apart by the type of its source and writes as
   an `img` instead of an anchor. The cost is conceptual and practical: an
   emoticon is not a mention, so this widens a shared abstraction and the
   edits land in the middle of a file that is likely to move between
   releases.
2. **A separate popover for emoticons.** Self-contained, no risk of
   regressing mention completion, rebases as a whole directory. The cost is
   duplicating the buffer scanning and placement logic, which is the part
   worth reusing.

The composer side is the same either way, and is small: a
`ComposerChunk::Emoticon`, `<img data-mx-emoticon src alt title height="32">`
in the formatted body, `:shortcode:` in the plain body, and the flag that
forces the message to be sent as HTML, next to `has_rich_mentions`.

Note that the specification asks clients **not** to resolve a shortcode to an
image on their own when several packs define it, and to present a picker
instead, so the completion must always be a choice and never a substitution.

## Integration-point ledger

Existing files touched. Keep this current — it is the rebase map.

| File | Change |
| ---- | ------ |
| `src/session/mod.rs` | declare and re-export `image_packs`; `image_packs` property, built in `prepare()` |
| `src/session/room/permissions.rs` | `can_send_sticker`, mirroring `can_send_message` with `MessageLikeEventType::Sticker` |
| `src/session/room/timeline/event/mod.rs` | allow reply and react for stickers, in `can_be_replied_to` and `can_be_reacted_to` |
| `src/session_view/room_history/message_toolbar/mod.blp` | the sticker button and its popover |
| `src/session_view/room_history/message_toolbar/mod.rs` | declare `sticker_picker`; the two template children; send a sticker on selection; the room of the picker in `set_timeline`; `update_sticker_button` and its permission handler |
| `data/resources/icons/scalable/actions/sticker-symbolic.svg` | new icon, there is none in Adwaita |
| `data/resources/resources.gresource.xml` | the icon |
| `data/resources/stylesheet/_room_history.scss` | `.sticker-picker` |
| `src/ui-blueprint-resources.in`, `po/POTFILES.in` | the new files |
| `src/components/mod.rs` | declare and re-export `custom_emoticon` |
| `src/session_view/room_history/message_row/text/mod.rs` | `img` in `SUPPORTED_INLINE_ELEMENTS`; allow the `data-mx-emoticon` attribute; `CUSTOM_EMOTICON_ATTRIBUTE` |
| `src/session_view/room_history/message_row/text/inline_html.rs` | one ordered `widgets: Vec<gtk::Widget>` in place of the pills of `MentionsMode`; `append_image`; `append_element_node` takes the whole `MatrixElementData`, for the attributes |
| `src/session_view/room_history/message_row/text/widgets.rs` | the inline widgets are no longer only pills |
| `src/session_view/room_history/message_row/text/tests.rs` | the custom emoticon cases |

Still to come, per phase: `message_row/text/{mod,inline_html,widgets}.rs`
(phase 4), `message_toolbar/{composer_parser,completion}` (phase 5),
`account_settings/mod.blp` and `room_details/mod.rs` (phase 6).

## Conventions

* Commit tags by area: `image-packs:`, `message-toolbar:`, `message-row:`,
  `room-details:`, `account-settings:`; GNOME commit-message style.
* GObject module layout (`mod.rs` + `mod.blp`), `spawn!`/`spawn_tokio!`,
  `toast!` for user-facing errors, gettext for all strings.
* Pre-commit checks: rustfmt, typos, rumdl, POTFILES and blueprint list
  consistency (`hooks/checks`). Clippy pedantic is warn-level.

## Building here

`meson setup _build` needs `sass` or `grass`, which is not installed. For
type-checking only, `src/config.rs` can be written by hand from
`src/config.rs.in` (it is gitignored) and then
`CARGO_TARGET_DIR=… cargo check` works. `/tmp` is small; point the target
directory somewhere under `$HOME`, and pass `-j 2` to `cargo test`, which
otherwise gets the compiler killed for memory.

Two consequences of not having a meson build:

* `login::local_server::tests::generate_local_server_landing_page` fails,
  because it loads the gresource file that meson would have built. Every
  other test passes.
* Blueprints are not compiled by the build, so check them by hand with
  `blueprint-compiler compile <file>.blp` from `src/`.

rustfmt is configured with nightly-only options. Stable rustfmt agrees with
the tree on everything else, so `cargo fmt` is still worth running.

## Testing against a homeserver

Nothing here needs another server or another client: a pack is a state event
and some account data on your own homeserver, and the images are in its own
media repository.

There is no authoring UI yet, so `image-pack-tool.py`, next to this file,
creates one. By default it renders emoji from the system emoji font, so it
needs no images and no network beyond the homeserver:

```sh
./doc/image-pack-tool.py --homeserver https://matrix.example.org \
    --user alice --password hunter2 --room '!abc:example.org'
```

`--images DIR` uses a directory of files instead, taking the shortcodes from
their names. The placements to cover, one flag each:

| Flag | What it writes |
| ---- | -------------- |
| `--room` | `im.ponies.room_emotes` in the state of the room |
| `--room --stable` | `m.room.image_pack`, to check that we read both names |
| `--personal` | `im.ponies.user_emotes`, the pack with no stable name |
| `--room --enable-globally` | also `im.ponies.emote_rooms`, so the pack appears in every room |

`--usage sticker` or `--usage emoticon` restricts where the pack shows up;
the default leaves `usage` unset, which means everywhere.

Two things that look like bugs but are not:

* Custom emoticons follow the media previews setting, which defaults to
  private rooms only. In a public room they render as their description.
  Account settings, Safety, Media Previews.
* The sticker picker loads the packs the first time it is opened for a room
  and keeps them until the room changes, so a pack edited from elsewhere only
  appears after switching rooms. `ImagePacks` already emits `changed`, but
  nothing listens to it yet, and the packs in the state of a room are not
  watched at all.

## Rebase guide

1. Rebase the branch onto the new release tag.
2. Whole-directory additions (`image_packs/`, `sticker_picker/`,
   `image_packs_page/`, `image_packs_subpage/`) rarely conflict.
3. Conflicts concentrate in the ledger files above; re-apply by intent, not
   by hunk.
4. Check for changes in: the `EventContent` derive, `Timeline::send`, the
   sanitizer configuration, `LabelWithWidgets`, and
   `Room::get_state_events`.
5. If ruma gains the unstable names and a personal pack type, `events.rs`
   can be replaced by them.
6. Run the `events.rs` tests and `message_row/text/tests.rs` first; they
   catch wire and renderer drift cheapest.

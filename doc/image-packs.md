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
* `src/account_settings/image_packs_page/` — the personal pack, and every
  pack enabled globally with a switch to stop using it. A pack whose room the
  user has left cannot be loaded, and is presented by its state key with a
  warning, which is the case the specification asks clients to handle.
* `src/session_view/room_details/image_packs_subpage/` — the packs of a room,
  each with a switch to use it in every room, and, for a user whose power
  level allows it, a button to edit one and a button to create one.
* `src/components/image_pack_editor/` — the editor, shared by the room
  subpage and the account settings page because a pack is the same thing in
  both places. It is an `AdwNavigationPage`, which both an
  `AdwPreferencesWindow` (room details) and an `AdwPreferencesDialog`
  (account settings) can push, so each caller pushes it itself.
* `src/components/custom_emoticon.rs` — `CustomEmoticon`, an image sent
  inline in a message. Among words it is sized from the font metrics, not
  from the `height` attribute, which the specification only requires for the
  clients that do not support image packs. Alone in a message it takes the
  size the timeline gives a sticker, from `THUMBNAIL_MAX_DIMENSIONS`.

A message that contains nothing but emoticons does not go through
`LabelWithWidgets` at all: `widgets.rs` returns a plain box of widgets
instead. Reserving a pango shape inside a line of text is only worth its
trouble for an emoticon sitting among words, and a message of one emoticon is
the same thing to the sender as a sticker, so it is presented the same way.

Only an `mxc:` source is ever loaded, which ruma enforces by leaving
`ImageData::src` unset for anything else, so a message cannot make us fetch
anything from outside the homeserver.

The SDK sanitizes the HTML of every message before we are given it, with
`HtmlSanitizerMode::Compat` in `Message::from_event`, and there is no way to
opt out. That allow-list only keeps `src`, `alt`, `title`, `width` and
`height` on an image, so `data-mx-emoticon` never survives. The
specification says an image is a custom emoticon **if and only if** that
attribute is present, which we therefore cannot honour: an inline image whose
source is on the homeserver is presented as an emoticon instead. The
attribute is still checked first, so this becomes exact again if the SDK ever
stops removing it. Reading the unsanitized HTML back from `Event::raw()` was
the alternative, and it was rejected because an edited message would need
`latest_edit_raw()` and its `m.new_content`, which is a lot of surface for
the same result.

Emoticons are deliberately **not** gated behind
`GlobalAccountData::should_room_show_media_previews`, which they were at
first. The setting exists so that the media of a message is not fetched until
it is clicked, and an emoticon has nothing to click: it is part of the text,
so hiding it leaves a message that cannot be read, with no way to get it
back. The residual difference from an ordinary image is that a sender can
learn roughly when a message was read, through their homeserver being asked
for the media. Restoring the gate is a two-line change in `append_image` if
that trade is not wanted.

## Traps

Three things cost a lot of time here and are easy to walk back into.

* **A widget whose class sets a layout manager is never measured through its
  own `measure`.** `CustomEmoticon` was an `AdwBin`, which sets
  `GtkBinLayout`, so GTK asked the layout manager and every size it computed
  was discarded; the label was told the size of the image instead, which is
  nothing before it loads and hundreds of pixels after. It subclasses
  `GtkWidget` and draws the image itself for that reason. Do not give it a
  child widget or a layout manager again.
* **`compute_concrete_size` with a specified size of zero returns the
  intrinsic size**, ignoring the allocation. Draw at the allocated size.
* **The SDK sanitizes the HTML of every message before we see it**, so
  `data-mx-emoticon` never arrives. See the wire format section.

None of these are visible to the compiler or to the tests, which do not build
widgets. A change to how something is drawn has to be looked at.

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
   composer, serialization in `composer_parser.rs`. _(done)_
   Verified in the app: an emoticon sent on its own is the same size as the
   same image sent as a sticker.
6. **Pack management** — room details subpage and account settings page,
   enabling and disabling packs globally. _(done)_
7. **Pack authoring** — create and edit packs, upload images, edit
   shortcodes and usage; the packs of a room gated on the power level.
   _(done)_ The avatar of a pack is not editable, only preserved; nothing
   presents it yet.
8. **Space packs** — canonical space hierarchy, recursive, with a depth
   limit and a cycle guard, slotted into the order in `packs_for_room`.

## How `:shortcode:` completion is done

`CompletionPopover` is 769 lines built around one abstraction: its rows are
`PillSourceRow`s bound to `PillSource`s, and activating one inserts a `Pill`
into the composer. It handles the buffer scanning, the word boundaries, the
key navigation and the popover placement, none of which is specific to
mentions.

Emoticons reuse it, by being `PillSource`s. `AtRoom` is the precedent: a
pill source that is not a user or a room, presented like a mention while the
message is composed without being one. A shortcode is the display name and
the pack image is the avatar, so the rows, the keyboard handling, the popover
placement and the insertion all work unchanged, and `row_activated` already
downcasts to `PillSource` rather than to anything narrower.

What that leaves is small: a `:` sigil, a `SearchTermTarget`, a list of
sources, and a separate scan for the word boundaries. The scan is separate
because `:` is also the separator of a Matrix ID, and the existing parser
reads localparts, server names, IPv6 addresses and ports. A `:` only counts
as the sigil at the beginning of a word, which is unambiguous: in
`@user:server` the scan starts at `@`. At least one character has to follow
it before anything is proposed, so that `:` in ordinary text is quiet.

The alternative that was rejected, for the record:

a separate popover for emoticons, which would have rebased as a whole
directory, but would have duplicated the buffer scanning and the placement,
which is the part worth reusing.

The composer side is a `ComposerChunk::Emoticon`, written as
`<img data-mx-emoticon src alt title height="32">` in the formatted body and
`:shortcode:` in the plain body, with a flag next to `has_rich_mentions` that
forces the message to be sent as HTML. A draft cannot store the image, so it
keeps the shortcode as text and the emoticon has to be picked again.

The completion is always a choice and never a substitution, which is what the
specification asks for when several packs define the same shortcode.

## Authoring

Writing a pack is the mirror of reading one, with three decisions worth
keeping.

* **A pack is written back under the event type it was read from.** The
  source of a pack carries that type, so editing a pack that another client
  created under `m.room.image_pack` does not leave a second copy of it behind
  under `im.ponies.room_emotes`, shadowing the first. Only a new pack picks
  the type, and it picks the unstable one.
* **Deleting is saving a pack with no images**, which is what the reader
  already treats as absent, and is also what a redacted pack looks like. The
  pack is removed from the packs enabled everywhere at the same time.
* **Images are uploaded when they are chosen, not when the pack is saved**,
  so that the editor can present them. Leaving without saving therefore
  leaves the media on the homeserver with nothing pointing at it. Saving on
  the way out instead would mean either holding every file in memory or
  presenting the images from disk and re-resolving them later, and neither is
  worth avoiding an orphaned upload.

The state key of a new room pack is the empty one when it is free, which is
what the clients in the wild use for the pack of a room, and `pack-2`,
`pack-3` … after that. `PackMeta` and the data of an image keep the
properties we do not know about, so an edit does not drop what another client
put there.

Permission is one property, `Permissions::can_change_image_packs`, checked
against the event type we create packs under. A room that gave the stable and
the unstable type different power levels would need it to be per pack; there
the request fails and the error is reported, which is enough.

Room state is now watched under both names, so a pack saved from the editor —
or by another client, in the room that is open — appears without switching
rooms.

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
| `src/session_view/room_history/message_row/text/widgets.rs` | `is_emoticons_only` and `emoticons_widget`, so a message of only emoticons skips the label |
| `src/session_view/room_details/mod.rs` | declare `image_packs_subpage`; the `ImagePacks` subpage name and its construction |
| `src/session_view/room_details/general_page.blp` | the row that opens the subpage |
| `src/account_settings/mod.rs`, `mod.blp` | declare and present `image_packs_page` |
| `src/session_view/room_history/message_toolbar/composer_parser.rs` | `ComposerChunk::Emoticon`, its serialization, and the flag forcing HTML |
| `src/session_view/room_history/message_toolbar/completion/mod.rs` | declare `emoticon_list` |
| `src/session_view/room_history/message_toolbar/completion/completion_popover.rs` | the `:` sigil, `SearchTermTarget::Emoticon`, the shortcode boundary scan, the list and the accessible label |
| `src/session/room/permissions.rs` | `can_change_image_packs`, from the event type of the packs we create |
| `src/components/mod.rs` | declare and re-export `image_pack_editor` |
| `src/session_view/room_details/image_packs_subpage/{mod.rs,mod.blp}` | the buttons to create and edit a pack, and a reload when the packs change |
| `src/account_settings/image_packs_page/{mod.rs,mod.blp}` | the personal pack row opens the editor |

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
* Editing a pack in a room needs the power level to send its state event.
  Without it the pack list has no buttons to create or edit, which is not an
  error.

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

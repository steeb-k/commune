# GIF search (KLIPY) — downstream implementation notes

This file is the ledger for the GIF search: design decisions, every
integration point into existing code, and what to check when rebasing onto a
new Fractal release. See `fork.md` for why none of this goes upstream.

## Scope

A second tab in the sticker picker where the user can search for a GIF and
send it. Nothing else: no GIF in the composer, no GIF in the completion, no
stickers or clips or memes from the same API.

## The service

[KLIPY](https://klipy.com/) is a GIF API. It was chosen because a key was to
hand; nothing in the code is specific to it beyond `src/utils/klipy.rs`, which
is the only file that knows the wire format.

Endpoints used, all under `https://api.klipy.com/api/v1/{key}/`:

| Path                | Method | Purpose                        |
| ------------------- | ------ | ------------------------------ |
| `gifs/search`       | GET    | `q`, `page`, `per_page`        |
| `gifs/share/{slug}` | POST   | Report that a GIF was sent     |

`gifs/trending`, `gifs/categories` and `gifs/recent` exist and are not used —
see "Nothing is requested until the user searches", below.

A result item carries `hd`/`md`/`sm`/`xs` sizes, each in `gif`, `webp`, `mp4`,
`webm` and `jpg`, plus a `blur_preview`: a blurred thumbnail as a `data:` URI,
inline with the results. The picker presents `xs.webp`, with `blur_preview` as
the placeholder until it arrives; what is sent is the largest `gif` variant
under 4 MB.

**`static.klipy.com` is slow, and that is the thing to design around.** The
API itself answers in about 130 ms, but a page of 24 `sm.webp` previews is
3.9 MB and took 12.6 s to fetch, twenty at a time — every request in the image
queue passed its ten-second stall threshold. The same page of `xs.webp` is
905 KB and 3.1 s. `xs` is also 87×90, which is what the 90 px-tall buttons
present at, so the small variant is both the fast choice and the correct one.
Pages are 16 items for the same reason, previews are downloaded four at a time
so that the grid fills in a row at a time rather than all at the end, and every
button shows its `blur_preview` in the meantime so the results never look
empty. If previews ever feel slow again, measure the CDN before suspecting the
API or the decoder.

The same host is why the send cap is 4 MB. The median `hd.gif` in a page of
results is 3.1 MB and the largest was 15.7 MB; at 300 KB/s that is the whole
delay between choosing a GIF and seeing it in the room.

The `slug` of an item is only valid for the response it arrived in — it embeds
a per-response token — so a share can only be reported with the slug that was
presented, never a stored one.

### Points that shaped the code

* **The API mixes advertising into its results.** Items carry a `type`, which
  is `gif` for a GIF and `ad` for a sponsored item, and `meta` carries
  `ad_max_resize_percent`. Everything that is not a `gif` is dropped in
  `GifPage`'s deserialization, so no advertising reaches the composer. A page
  that held nothing else is not reported as empty; the next one is requested
  instead.
* **`customer_id` is optional and is not sent.** It is the identifier the API
  uses to tie a user's requests together. Searching works without it.
* **The terms of use require a visible "Powered by KLIPY".** It is a link at
  the bottom of the GIF tab. The terms ask for the logo as well, which we do
  not ship; the text-only attribution is what is there.
* **The terms forbid caching results.** Nothing is stored: the results live in
  the widget and are dropped when the query changes.
* The `query` and `json` features of `reqwest` are not enabled by the SDK, so
  the query string is built with `url::form_urlencoded` and the body is parsed
  with `serde_json`.

## Design decisions

### The key is a build option, not a constant

`meson.options` has `klipy-api-key`, empty by default, which reaches the code
as `config::KLIPY_API_KEY` in a generated, git-ignored `src/config.rs`. The
repository is public, and a key committed to it would be a published key. With
no key, `klipy::is_available()` is `false`, the GIF tab is never shown and the
settings row is hidden — the feature is simply not in the build.

Set it with `meson configure _build -Dklipy-api-key=…`.

**This rule holds for the core and for the Kotlin build too, and it did not
always.** `commune-core` is where the client now lives, and its first version
hard-coded the key as a `const` in `commune-core/src/klipy.rs` — a tracked
file — because the core has no build system of its own to take an option
from. That published the key for four days on `origin/fractal-kotlin`; see the
divergence ledger in `doc/track3-convergence.md`. The key is not the core's to
know: the embedder passes it to `commune_core::config::init()`, the desktop
from the Meson option above and the Kotlin build from a
`communeKlipyApiKey` Gradle property that reaches it as
`BuildConfig.KLIPY_API_KEY`. Both default to empty.

For an Android build, put the key in your own `local.properties` or
`~/.gradle/gradle.properties`:

```properties
communeKlipyApiKey=…
```

or pass `-PcommuneKlipyApiKey=…`. Neither file is tracked. The Kotlin side
asks `gifSearchAvailable()` over the FFI for the same answer
`klipy::is_available()` gives the desktop.

### Nothing is requested until the user searches

Opening the GIF tab shows a prompt, not a page of results. Loading trending
GIFs on open meant a multi-megabyte download every time the picker was opened,
which was slow enough to look broken, and it sent a request to a third party
for merely opening the sticker picker. Clearing the search box returns to the
prompt. `klipy::trending` was removed rather than left unused; the endpoint is
still there if it is ever wanted.

### The feature is opt-in

`gif-search-enabled` in the GSettings schema, `false` by default, with a switch
in the account settings under "Composer". Until it is turned on, nothing is
requested from KLIPY. The reason is that a search sends what the user types
and the IP address of their device to a party that is not their homeserver and
not part of Matrix, which is not what anyone expects of a Matrix client.

The cost is discoverability: nobody finds the tab without going to the
settings first. An in-tab explanation with an "Enable" button would be more
discoverable and just as honest about the data; it is the obvious next change
if the switch turns out to be too well hidden.

### A GIF is sent as an `m.sticker`, uploaded to the homeserver

Not as a link to `static.klipy.com`: a link would give KLIPY the IP address of
everyone in the room, would rot when the file moves, and could not be
end-to-end encrypted.

`m.sticker` rather than `m.image` because that is what the picker is: it
renders borderless and without a filename. This client animates stickers — an
`m.sticker` takes the same `build_image` path as an image, with `animated:
true` — but only if the `ImageInfo` we build says so. `mimetype` must be
`image/gif` and `is_animated` must be `Some(true)`, or `ImageSource::
should_thumbnail` lets the receiving client ask its homeserver for a
thumbnail, which is a still frame. That is the one non-obvious requirement in
the whole feature.

In an encrypted room the GIF is encrypted like any other media, with
`Client::upload_encrypted_file`, and referred to by
`StickerMediaSource::Encrypted`. That variant is behind ruma's
`compat-encrypted-stickers` feature, which is enabled transitively by
matrix-sdk — it is not named in our `Cargo.toml`, so if encrypted stickers stop
compiling after an SDK bump, that is why.

Sending needs the same power level as any other sticker, so the picker button
already gates it and there was nothing to change there.

### Two things about presenting a preview that are easy to get wrong

**An animated preview does not animate on its own.** An
`AnimatedImagePaintable` only advances while something holds a `CountedRef`
from its `animation_ref()`. `GifButton` takes one when it is mapped and drops
it when it is not, the same as `MessageVisualMedia` and the media viewer do.
Without it every preview sits on its first frame, which looks exactly like a
still image and not at all like a bug.

**The size of the child must not decide the size of the button.**
`set_width_request` is a minimum, not a maximum, and a decoded preview is
whatever pixel size the source happened to be, so letting `GtkPicture` report
its natural size gave rows of different heights with previews far larger than
the 90 px asked for. `GifButton` overrides `measure()` to report exactly the
size it presents at, and sets `halign`/`valign` to `Center` so the flow box
does not stretch it to its column either.

### The image queue learned to speak HTTP

Previews come from a third-party host, which the image pipeline had no way to
reach: `ImageRequestSource` knew only Matrix downloads and local files. It has
a third variant, `Http`, so previews get the existing glycin decoding,
animation, request de-duplication, priority and stall handling for free.

`src/utils/http.rs` holds the client that both this and `klipy` use. It
enforces a size limit as it reads, because unlike Matrix media there is no
homeserver in the way to enforce one.

## Files

New:

* `src/utils/http.rs` — the shared non-Matrix HTTP client and a size-limited
  `fetch`.
* `src/utils/klipy.rs` — the API: `search`, `report_share`, the wire types,
  variant selection, and the tests for all of it.
* `src/session_view/room_history/message_toolbar/sticker_picker/gif_page.{rs,blp}`
  — the tab: search entry, debounce, paging, the results flow box, the
  attribution.
* `src/session_view/room_history/message_toolbar/sticker_picker/gif_button.rs`
  — one GIF in the results.
* `doc/gif-search.md` — this file.

Edited:

* `meson.options`, `src/meson.build`, `src/config.rs.in` — the
  `klipy-api-key` option.
* `data/io.github.steeb_k.Commune.gschema.xml.in` — `gif-search-enabled`.
* `src/utils/mod.rs` — the two new modules.
* `src/utils/media/image/queue.rs` — `ImageRequestSource::Http`,
  `ImageRequestId::Http`, `HttpRequest`, `add_http_request`.
* `src/session_view/room_history/message_toolbar/sticker_picker/mod.blp` — the
  view stack and switcher around the existing sticker page.
* `src/session_view/room_history/message_toolbar/sticker_picker/mod.rs` — the
  `gif-selected` signal, and showing the tab when the setting is on.
* `src/session_view/room_history/message_toolbar/mod.rs` — `send_gif`,
  `upload_gif`, `SendGifError`.
* `src/account_settings/general_page/mod.{rs,blp}` — the "Composer" group with
  the switch.
* `data/resources/stylesheet/_room_history.scss` — `.gif-button`.
* `src/ui-blueprint-resources.in`, `po/POTFILES.in` — the new files.

## Rebase guide

The picker's `mod.blp` is the only existing file whose shape changed: its
`Gtk.Stack` is now inside an `Adw.ViewStackPage`, with an
`Adw.InlineViewSwitcher` below the stack. If upstream touches the sticker
picker, take their `Gtk.Stack` wholesale and put it back inside the view
stack.

`queue.rs` gained one variant in each of three enums and two methods; a
conflict there is mechanical.

Everything else is either a new file or a few lines appended to a list.

## Not done

* No `customer_id`, so the results are not personalised. Deliberate.
* The attribution is text; the terms also ask for the KLIPY logo.
* No trending or categories: the tab is empty until searched. See above.
* Sending shows no progress. The GIF is downloaded from KLIPY and uploaded to
  the homeserver _before_ the event is queued, so for several seconds nothing
  appears in the room at all — the spinner only shows up once the local echo
  is created. Lowering the send cap to 4 MB shortened it; only routing this
  through the send queue with a local echo would actually fix it, and that
  means teaching `Timeline::send_attachment` about stickers.

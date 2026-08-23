# URL previews — downstream implementation notes

This file is the ledger for link previews: what the fork added, the decisions
behind it, and what to check when rebasing onto a new Fractal release. See
`fork.md` for why none of this goes upstream.

## Scope

* Ask the homeserver about the first link of a text or notice message, through
  `GET /_matrix/client/v1/media/preview_url`.
* Show what comes back as a card under the message: site name, title,
  description and image. Clicking it opens the link.
* A switch in Account Settings ▸ General ▸ Messages turns the whole thing off.

Never in an encrypted room, never for more than one link per message, and
nothing is ever added to what this client _sends_.

## The spec has no opt-out, so ours is local

This is the thing most likely to be got wrong by someone reading other
clients. The Client-Server API v1.19 defines the endpoint and nothing else:
there is no `m.url_preview` account data, and no room state event for
switching previews off. Element's `org.matrix.room.preview_urls` state event
and its `im.vector.web.settings` account data are its own inventions, not
part of the standard.

So the switch is a GSettings key, `url-previews-enabled`, exactly like
`gif-search-enabled`. It is per-device and it is never written to anybody's
room. Adding a per-room toggle would mean inventing an event name and putting
it in other people's rooms, which is what the image pack work spent a day
undoing.

It defaults to **on**. In an unencrypted room the homeserver already stored
the message that carries the link, so asking it about that link tells it
almost nothing new. That is not true of GIF search, which leaves Matrix
entirely, which is why that one defaults to off and this one does not.

## Encrypted rooms are not negotiable

The spec says plainly:

> Clients should consider avoiding this endpoint for URLs posted in encrypted
> rooms. Encrypted rooms often contain more sensitive information the users do
> not want to share with the homeserver, and this can mean that the URLs being
> shared should also not be shared with the homeserver.

So the check is in `previewable_message_url()`, above the settings check, and
the switch cannot reach it. Turning previews on does not turn them on there.

**Use the SDK's tri-state, not `Room::is_encrypted()`.** That property is a
`Cell<bool>` that starts `false` and is only set to `true` after
`update_is_encrypted()` has awaited the SDK, so there is a window on startup
where an encrypted room reports itself unencrypted. `encryption_state()` on
the SDK's room is synchronous and returns `Encrypted` / `NotEncrypted` /
`Unknown`, and only `NotEncrypted` gets a preview. `Unknown` is treated as
encrypted, which is the safe direction of the two.

## MSC4095 is deliberately absent

There is a proposal for putting preview data _inside_ the message event, so
that a sender's client does the lookup once and every reader sees the same
card without asking their own homeserver. ruma models it, behind the
`unstable-msc4095` feature, and it serialises as `com.beeper.linkpreviews`.

We do not enable it. It is not in v1.19, and using it would put a
non-standard property in other people's rooms — the same trade that
`im.ponies.*` made and that has since been undone. The consequence is worth
naming: a message sent from Commune carries no preview data, so a client that
only reads bundled previews shows none for it.

Nothing about a message we _send_ changes at all. This feature is read-only
against the protocol.

## Only the stable endpoint, so old homeservers get nothing

`preview_url` moved under `/_matrix/client/v1/` in Matrix 1.11; before that it
was MSC3916, at `/_matrix/client/unstable/org.matrix.msc3916/media/preview_url`.
ruma's metadata knows both, and would fall back to the unstable path on a
homeserver that advertises the feature flag.

`is_supported()` prevents that by checking `client.server_versions()` for 1.11
or later before the first request, and switching the feature off for the
session when the answer is no. The SDK caches `/versions`, and the answer is
cached again in `UrlPreviewSupport`, so this costs one request per session at
most.

The same flag is set when a request comes back `M_UNRECOGNIZED`, which is what
a homeserver that has turned previews off answers with. Without it, every link
in the timeline would earn its own doomed request. Any other error — including
`M_FORBIDDEN`, which a homeserver returns for a URL it refuses to fetch, and
`M_LIMIT_EXCEEDED` — is per-URL and does not switch anything off.

## Which link, and why only one

`previewable_url()` in `src/utils/matrix/url_preview.rs` picks it, and it has
to agree with what the message actually renders as a link, or the card would be
for something the reader cannot see.

* **HTML wins over the plain body.** When there is a formatted body, the
  anchors in it are the links; the plain body is a fallback whose text can be
  something the HTML never linked. The two paths are separate for that reason.
* **`code` and `pre` are skipped.** A URL written inside one was meant to be
  read, not followed.
* **Mentions are not links.** `matrix:` and `matrix.to` URIs parse into their
  own `AnchorUri` variants, so the HTML path never sees them. The plain path
  has to work harder — see below.
* **`ftp:`, `mailto:` and `magnet:` are allowed in an anchor** by the spec and
  arrive as `AnchorUri::Other`, so the scheme is checked explicitly.
* **The reply fallback is sanitised away** before the search. The SDK already
  strips it, so this is belt and braces; without it, a reply would carry a
  second card for the link in the message it was replying to.

**The plain-text path has to mirror the linkifier.** `LinkFinder` is run with
`url_must_have_scheme(false)` so that `example.org/post` counts, because the
linkifier makes that a link too — and then a scheme-less match only counts if
its top-level domain is real, or `1.5` and `e.g.` would both be sent to the
homeserver. It also has the linkifier's `prev_span` guard: the finder detects
`example.org` inside `@alice:example.org`, and the `:` in front of it is what
tells the two apart. All of this is what `url_preview/tests.rs` is for; the
Matrix identifier case in particular was a live bug the tests caught.

One link per message, always the first. Two cards under one message is a lot of
timeline for one person's link dump, and each one is a rate-limited request.

## The response cannot be trusted to have the shape we expect

The endpoint returns free-form OpenGraph data. The spec names exactly two
things: `og:image` is an `mxc:` URI rather than a URL, and `matrix:image:size`
is the byte size. Everything else is "additional properties as per the
OpenGraph protocol".

So the body is deserialized into a `serde_json::Map` and read key by key,
rather than into a struct. A struct would throw away a good title because
some homeserver sent `og:image:width` as a string, which they do —
`uint_property()` accepts both spellings for that reason.

`og:image` is checked to actually be an `mxc:` URI before it is used. A
homeserver that sent an ordinary URL would have us fetch the image straight
from whoever the page links to, which is the one thing a preview exists to
avoid.

A response with no title, no description and no image gets no card. So does an
empty body, which is a valid answer meaning the homeserver looked and found
nothing.

## The card appears or it does not

There is no spinner and no placeholder. `update_card()` keeps the card hidden
until the preview reaches `LoadingState::Ready`, because most of the ways this
can end — no data, a refusal, an old homeserver — end with nothing to show, and
a placeholder that is then removed makes the timeline jump for no reason.

`RemoteUrlPreview` objects are cached in `RemoteCache` by URL, 100 of them, so
scrolling back over a message does not ask again. Unlike `RemoteRoom` and
`RemoteUser` there is no staleness check: a page changing under a message that
linked to it does not change what the message said.

The image is downloaded through the same `ThumbnailDownloader` as everything
else, so it goes through the texture cache and the request queue. The widget is
reused as the timeline scrolls, so the download checks that the preview it
started for is still the one being shown before it touches the image.

**Two traps, both of which cropped the image on screen, and neither of which
the compiler can see.**

* **Ask the homeserver to `Scale`, never to `Crop`.** A preview image is
  usually a wide banner with words on it. `Method::Crop` makes the homeserver
  cut it to a square before sending it, so `matrix.org` arrives as `natrix`.
  The cropping happens on the server, so nothing the widget does afterwards can
  undo it.
* **`GtkImage` with `pixel-size`, not `GtkPicture`.** A `GtkPicture` takes the
  natural size of its paintable, and a homeserver answers a request for 72
  pixels with whichever thumbnail preset it feels like — 320 for Synapse. The
  card then grew to fit the image instead of the image shrinking to fit the
  card. `pixel-size` pins the box and scales the image inside it.

Together those are why the card is always the same width and shows the whole
image. `custom_emoticon.rs` solves the same problem with a custom `measure()`,
which is the heavier answer for when the size has to be computed rather than
fixed.

## Files

Integration points, which are where a rebase will conflict:

| File | Change |
| --- | --- |
| `src/utils/matrix/url_preview.rs` | `previewable_url()`, which link gets picked |
| `src/utils/matrix/url_preview/tests.rs` | The cases above, including the ones that must _not_ be previewed |
| `src/session/remote/url_preview.rs` | `RemoteUrlPreview`, the request and the `OpenGraph` parsing |
| `src/session/remote/cache.rs` | The preview cache and the shared support flag |
| `src/session_view/room_history/message_row/url_preview.rs` | The card, and the container the message sits in |
| `src/session_view/room_history/message_row/url_preview.blp` | Its template |
| `src/session_view/room_history/message_row/content.rs` | `build_text_message_content()` and the three gates |
| `src/account_settings/general_page/mod.rs` | Binding the switch |
| `src/account_settings/general_page/mod.blp` | The Messages group |
| `data/io.github.steeb_k.Commune.gschema.xml.in` | `url-previews-enabled` |
| `data/resources/stylesheet/_room_history.scss` | `message-url-preview` |

## Rebase guide

1. `MessageType::Text` and `MessageType::Notice` in `content.rs` used to build a
   `MessageText` directly. They now go through `build_text_message_content()`,
   which chooses between a bare `MessageText` and one wrapped in a
   `MessageUrlPreview`. A merge that restores the direct call silently drops
   previews without breaking the build.
2. `MessageUrlPreview` implements `ChildPropertyExt` and
   `MessageContentContainer` the same way `MessageCaption` does, so that
   `child_or_default::<MessageText>()` reaches the message inside it rather
   than replacing the container. If upstream reshapes either trait, both move
   together.
3. The three gates in `previewable_message_url()` are ordered: format, then
   encryption, then the setting. Encryption above the setting is the point —
   see above.
4. `is_supported()` and the `M_UNRECOGNIZED` branch share one
   `Rc<Cell<Option<bool>>>` per session, handed out by `RemoteCache`. It is a
   plain cell rather than something holding the cache, which is what keeps it
   out of a reference cycle with the previews the cache owns.
5. If the pinned `matrix-sdk` grows a wrapper for this endpoint, the raw
   `client.send()` can go — but check what it does about the unstable path
   before taking it.

## Not done

* **Only the first link.** Deliberate; see above.
* **Only `m.text` and `m.notice`.** An emote can carry a link and does not get
  a card. So does the caption of an image, where the card would compete with
  the image above it.
* **Nothing is cached across restarts.** The 100 entries live in memory, so
  reopening the app asks again for the links on screen.
* **The card cannot be dismissed** for one message, and there is no per-room
  switch. Both would need somewhere to remember the decision, and the only
  in-spec place is this device.
* **No `ts` parameter.** The endpoint accepts a point in time so that a link
  can be previewed as it looked when it was posted. Homeserver support for it
  is thin and Synapse ignores it, so the card shows the page as it is now.

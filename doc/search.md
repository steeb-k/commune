# Message search — downstream implementation notes

This file is the ledger for searching the messages of a room: design
decisions, every integration point into existing code, and what to check when
rebasing onto a new Fractal release. See `fork.md` for why none of this goes
upstream.

## Scope

* Search the messages of the open room, from a pane over the timeline.
* Go to a result, which means a timeline centred on one message.
* Event permalinks and notifications open on the message rather than the room,
  which falls out of the same machinery.

Not the sidebar's room-name filter, which already existed and is unrelated.
Not searching across rooms, not searching state events, not searching
attachments by filename.

## Two backends, because one is not enough

Searching a room means two different things depending on the room, and neither
mechanism covers both cases:

| | Server `/search` | Local index |
| --- | --- | --- |
| Unencrypted room | whole history | only what this device saw |
| Encrypted room | **nothing at all** | whole history it has seen |

A homeserver indexes the messages it can read, so `/search` covers the whole
history of an unencrypted room and returns nothing for an encrypted one —
which is most direct chats. The SDK carries a per-room tantivy index for
exactly that case, behind its `experimental-search` feature: the event cache
hands it every event it stores, after decryption.

So the room decides, in `RoomSearch::load()`:

```rust
if self.room().is_encrypted() {
    self.load_local(search_term).await;
} else {
    self.load_from_server(search_term).await;
}
```

This is the same shape as the media history viewer deciding how to filter, and
it is deliberate that the user is not asked to choose. The trade-off is not one
they can act on: in an encrypted room there is no server option, and in an
unencrypted one the server strictly dominates.

The consequence worth knowing: in an encrypted room, search only finds what
this device has received. A message the device never saw is not findable here
at all. A message it saw but the SDK never indexed is handed to the index by
the search itself — see the next two sections, which are the subtler half of
the same problem.

## The index is encrypted, and lives in the cache

Two settings on the client builder, in `utils/matrix/mod.rs`:

```rust
.search_index_store(SearchIndexStoreKind::EncryptedDirectory(
    search_index_path,
    passphrase.to_string(),
))
```

* **Encrypted**, with the passphrase that already protects the databases. The
  index holds message bodies in the clear otherwise, which would put the
  plaintext of every encrypted room on disk — the one thing the encryption is
  there to prevent.
* **In the cache directory**, not with the data, because the event cache can
  rebuild it. It is `cache_path.join("search_index")`.

Left at its default the store is in memory, which means reindexing from
nothing on every launch. That is the failure mode to watch for if this line is
ever lost in a rebase: search still works, it is just silently useless on a
cold start.

**On Windows it is in memory on purpose**, which is now `commune-core`'s
`matrix/mod.rs`, not this tree's. That build gets an empty index on every
launch, and what the search-time sweep below gives back is whatever the room
has loaded since — no more.

## The index only knows what it was handed

The index is fed by the event cache, as it stores an event, and by nothing
else. An event that was **already stored when the index was created** was never
handed over, so no search finds it however many times it is read.

That is not an edge case. It is every message on any device that received it
before the index existed — which is every device upgrading into the feature
rather than installing fresh. The room looks fully populated and searching it
returns nothing.

What makes it hard to recognise is that paginating hides how complete the hole
is. Fetching a chunk the cache does not have indexes it on the way in, so
scrolling back makes old messages findable while the recent ones stay missing.
The index looks like it is working at random, or like it only has old history,
which is the opposite of the truth.

### Decrypted late, never indexed

The upgrade hole is not the only way an event misses the index, and it is not
the common one. The SDK's indexing task skips an event it cannot decrypt
(`parse_timeline_event()` returns nothing for a UTD), and when the redecryptor
later decrypts it, the event cache swaps it in place with
`replace_in_memory_utds()` — which drains the linked-chunk updates rather than
broadcasting them, so the indexing task never hears of it. The client asks the
key backup only `AfterDecryptionFailure`, which means old history in an
encrypted room routinely arrives undecryptable and is decrypted a moment
later. Every such message was unsearchable, and scrolling back to it did not
help: it was the user report "search doesn't find old messages in encrypted
chats, even if we scroll back and load them".

So **the first page of every local search re-offers the room's loaded events
to the index** before querying it — `RoomSearch::index_loaded_events()`, the
same body `reindex()` always had, called from `load_local()` inside the task
that runs the query. Later pages come from `pending` and do not repeat it. The
SDK's add is idempotent (the index checks `contains` before writing a
document), so a sweep over events that are already indexed costs a parse, one
relation lookup per message for its latest edit, and a commit. A failure is
logged and the query runs against whatever the index already had: a search
that could have half-worked is not turned into an error page.

The other way to close this was to index at the moment of decryption, from the
timeline's diff stream in `timeline.rs`. It was not taken: a `Set` diff does
not say what the item was before, so telling "decrypted" from "a reaction or a
receipt changed" means indexing on every one of them; the item is the UI's
`EventTimelineItem`, not the `TimelineEvent` the index takes, so each would be
a cache lookup; every one would be a tantivy commit; and the stream only runs
while a view is subscribed, so it would not cover more than the sweep does.
The sweep is one pass, at the one moment the index is read.

### Re-index This Room

**Re-index This Room** is the remedy for the upgrade hole, and now the explicit
form of what every search does: `RoomSearch::reindex()` takes the events
the room has loaded from the event cache and hands them to the index through
`bulk_handle_timeline_event`, then restarts the search so the results appear
without the term being retyped. Load more history and press it again to cover
more — the event cache only offers the chunks it has, and nothing is fetched
from the server for a message this device already holds.

Three things about where the button appears:

* **Only on the two pages with nothing else to show** — before a term is typed,
  and when a search came up empty. Those are exactly the states this problem
  produces.
* **Only in an encrypted room.** `update_reindex_buttons()` returns
  `Room::is_encrypted` and gates on it. Every other room is searched on the
  server, which knows the whole history and wants nothing from this device, so
  the button there would do nothing at all.
* **The spinner runs even before a term is typed**, where the view would
  otherwise sit on the empty page giving no sign that anything is happening. A
  rebuild that fails leaves the error page rather than a spinner that never
  stops — see the two `LoadingState::Error` arms in `reindex()`, one for the
  task dying and one for the index refusing.

`doc/macos.md` carries this as row 40 of its by-hand list, with a note that it
is not a macOS problem, so that nobody tests a bundle and goes looking for a
port bug.

## The query parser has a syntax, and a search field does not

The index's query parser has a syntax of its own — quoting, `+`/`-`, field
prefixes like `title:`, grouping — and it returns _an error_, not an empty
result, for anything that does not respect it. An unbalanced quote or a stray
colon is enough. But a search field is plain text as far as the person typing
in it is concerned; `who's there?` is a search term, not a malformed query.

`sanitize_local_query()` therefore strips every character the parser treats as
syntax from each whitespace-separated token, then quotes it. A token left empty
by that is dropped, and a query left empty is treated as no results rather than
as a failure.

There is a trap in testing this. The obvious test asserts against
`QUERY_SPECIAL_CHARS`, the constant the implementation strips — and that test
passes for exactly the mistake it exists to catch, a character quietly dropped
from the list. So `PARSER_SYNTAX` in the test module is a deliberate second
copy of the list, and `sanitize_query_never_emits_syntax_the_parser_rejects()`
checks the _shape_ of the output against it over a table of terms a person
could reasonably type. **Do not deduplicate these two constants.** The comment
in the test says so; this says so as well, because the temptation on a rebase
is real.

## Results are newest first

The server sorts that way on request (`OrderBy::Recent`). The index sorts by
relevance only, so all of its results are fetched in one go — `LOCAL_MAX_RESULTS`,
500 — and ordered here before being handed out a page at a time from `pending`.

In a conversation the recent match is usually the one being looked for. This
is why the two paths present the same way despite paginating differently: the
server path follows `next_batch`, the local path slices a list it already has.

Two guards against stale results, both in `RoomSearch`:

* `generation`, bumped on every new search term, so a response that arrives
  after the term changed is dropped instead of appended to the wrong list.
* `abort_handle`, so the in-flight request is actually cancelled, and aborted
  on `dispose()`.

## Focused timelines

Going to a result needs a timeline that is not the live one. The live timeline
of a room ends at the present and only grows at that end, so it cannot be
centred on a message from last year without loading everything in between. The
SDK builds a timeline focused on a single event and paginates it in both
directions, so `Timeline` grew that mode — `Timeline::new_focused()` — and
`RoomHistory` swaps between the two.

A focused timeline is a different object with different rules, and every one of
them is a guard on `is_focused()` somewhere:

| Rule | Why |
| --- | --- |
| No sync events, no local echoes | The SDK gives it none; it is a snapshot |
| Can only be left by returning to the live timeline | There is no path forward to the present |
| Composer stays bound to the **live** timeline | So a message sent while reading old history still appears |
| Sticky scrolling off | Its bottom is not the present |
| No read receipt sent | Would move the read marker backwards, or mark the room fully read |
| No typing status | Not a live view of the room |
| `can_paginate_forwards()` is true only here | The live timeline is already at the end |

`focused_event_id` is a `OnceCell` set only from `Timeline::new_focused()`, so
the `expect()` in `set_focused_event_id()` is unreachable by construction. If a
second caller ever appears, that is the thing that breaks.

### A focused timeline is the last resort, not the first

`RoomHistory::focus_on_event()` used to build one unconditionally. It is the
single entry point for going to a message — a search result, a pinned message, a
`matrix.to` permalink, and **a click on a desktop notification** — and the first
rule in the table above is what makes that wrong for the last of them: a focused
timeline receives no sync events. Clicking the notification for a message that
had just arrived opened the room in a snapshot of it, announced _Back to Latest_
for a place you had never left, and then quietly showed nothing new until you
pressed it. Arriving in a chat room and not seeing the next message is a worse
failure than the one it was solving.

So the method asks first whether the room's **live** timeline already holds the
event, with `Timeline::has_event()` — a lookup in the timeline's map of events,
and the only "is this loaded" answer either the SDK or this tree offers.
`Timeline::find_event_position()` walks the items for the position and is for
the one-off scroll, once the event is known to be there. If it is loaded, the
live timeline stays and the event is scrolled to and highlighted in place. Only
an event that is genuinely not loaded gets a timeline of its own, which is the
case that mode exists for.

### The view fills itself only once the jump has landed

`RoomHistory` keeps the view full: while the viewport is within two pages of
the top it paginates backwards, batch after batch, and on a focused timeline it
does the same at the bottom. That rule read the scroll position as soon as the
timeline was ready, which on a focused timeline is before the scroll to its
event has been laid out — the view sat at row zero, every batch prepended
above it, and the position it read between batches was the one before the last
batch's layout. A search result from months back loaded the whole history up
to itself, and froze the app on a busy room, which is exactly what a focused
timeline exists not to do.

Two guards, both in `RoomHistory`, since 3e35dc0c:

* `focused_scroll_landed` — `needs_more_events_at_the_start()` and
  `..._at_the_end()` say no on a focused timeline until it is set. The scroll
  to the focused event is issued from an idle; a tick callback marks the frame
  it lands in, and an idle after that frame sets the flag and runs the fill
  check once, against a laid-out position.
* `MAX_AUTO_LOAD_BATCHES` — one automatic walk loads three batches at most.
  The list only reflects a batch in its next frame, so a walk that never
  yields reads the same position every time; between two walks a frame goes
  by, and the change of height it brings starts the next walk when more is
  still needed. This one applies to the live timeline too.

The same change fixed a spinner that the walk used to hide: the timeline's
initial events were applied without notifying `is-empty`, and the core reports
a focused timeline ready before its events reach the view, so without the
walk's state changes nothing ever swapped the spinner for the list. The
initial append goes through `update_with_diff_list()` now, which notifies.

The awkward part is that the answer is not available at the moment of the click:
the live timeline of a room that has never been opened is still being built, so
"not found" is not yet an answer. The highlight is therefore a _pending_ target —
`highlighted_event_id` on `RoomHistory`, alongside the existing
`focused_scroll_done` machinery — retried from the same `is-empty` and `state`
handlers that already drive `scroll_to_focused_event_if_needed()`. It gives up
and builds a focused timeline the moment the timeline is `Ready` and non-empty
without the event in it; a two-second timeout covers a timeline that never
becomes ready at all.

The highlight reuses the `focused-event` CSS class rather than inventing a
second one, and clears itself after three seconds — a permanent highlight on a
live timeline reads as a selection. Rows built after it is set pick it up in
`bind_list_item_to_item` like the focused one; rows already on screen are walked
and updated, which is what `update_target_event_rows()` is for.

## Two bugs fixed on the way

Both were pre-existing, and both are why the feature is bigger than it looks.

* **Event permalinks dropped the event.** `SessionView::show_matrix_uri()`
  destructured `MatrixIdUri::Event` down to its `room_uri` and handed that to
  the room preview, so a link to a message and a link to its room did the same
  thing. Now both `matrix.to` links and notifications open on the message.
* **`scroll_to_event()` converted a position wrong.** The position came from
  the flat list of timeline items but was handed to a list view backed by the
  grouping model, which merges runs of state events into one row — so it was
  off by however many rows had been merged above the target. It had one caller
  until now, the chip above the composer while replying, which is why nobody
  noticed.

## The pane was painted with the accent colour

The search pane came up tinted edge to edge in the accent colour. The cause
was not in the pane: `_room_details.scss` styled the "Drop Here to Send" page
with a descendant selector, `dragoverlay statuspage`, giving it an accent
background at `--dim-opacity` and accent text. But the room history's
`DragOverlay` wraps the whole room content, and the search pane's empty,
no-results and error states are `Adw.StatusPage` too — so they all matched.

The rule is now scoped to the overlay's own tree, from
`src/components/drag_overlay.rs`: `dragoverlay > overlay > revealer >
statuspage`. Anything inside the wrapped child is a descendant of the
`dragoverlay` but not on that path. The rule is upstream's, which is the thing
to know on a rebase.

## Esc closes the search before it closes the room

The session view binds Esc to `session.close-room`, as a class shortcut on
`SessionView`, which sits above the room history. The search entry answers Esc
itself (`Gtk.SearchEntry`'s `stop-search`, which the bar is connected to), but
only while it has the focus. With the focus on a result, or anywhere else in the
room, Esc went straight to the session view: the room closed and the search
stayed open — and since closing a room only hides the room history and keeps
its timeline, opening the same room again landed back in the search.

Fixed on 16 October 2026, in two places:

* `RoomHistory::init_search()` adds an `EventControllerKey` on the room history
  that, while search mode is on, turns it off, gives the focus back to the room
  (`grab_focus()`, which is the composer) and stops the event. It is in the
  **bubble** phase, not the capture phase, deliberately: a popover, the
  completion or the composer's own Esc (cancelling a reply) is nearer the focus
  and still wins. The session view only ever sees an Esc that arrived with the
  search already closed, so the second Esc closes the room as before. The
  entry's own `stop-search` now also hands the focus back, rather than leaving
  it on an entry that is about to be hidden.
* `Content::set_item()` calls `RoomHistory::close_search()` whenever the shown
  item changes. Switching to another room was already covered by
  `set_timeline()`; closing the room and reopening the same one was not,
  because `set_timeline()` returns early for the timeline it already has.

## Files

New:

| File | What |
| --- | --- |
| `src/session/room/search.rs` | `RoomSearch`, `RoomSearchResult`, query sanitisation, both backends, `reindex()` |
| `src/session_view/room_history/search/mod.rs`, `.blp` | The search pane, its empty/no-results pages and the two re-index buttons |
| `src/session_view/room_history/search/row.rs`, `.blp` | One result row |

Integration points, which are where a rebase will conflict:

| File | Change |
| --- | --- |
| `Cargo.toml` | `experimental-search` on both `matrix-sdk` and `matrix-sdk-ui` |
| `src/utils/matrix/mod.rs` | `search_index_store()` on the client builder; `original_message_event_from_raw()` |
| `src/session/room/mod.rs` | `mod search;` and its re-exports |
| `src/session/room/timeline/mod.rs` | Focused mode: `new_focused()`, `is_focused()`, forward pagination, the guards |
| `src/session/room/timeline/virtual_item.rs` | The end-of-timeline spinner |
| `src/session_view/room_history/mod.rs`, `.blp` | Swapping timelines, the search pane, `<primary>F`, the Esc controller in `init_search()`, `close_search()`, and `focus_on_event()` preferring the live timeline with a pending highlight |
| `src/session_view/mod.rs` | Permalinks open on the event |
| `src/session_view/content.rs` | Passing the focused event through; closing the search when the shown item changes |
| `src/session_view/room_details/history_viewer/event.rs` | Uses the event's own timeline, which may be focused |
| `src/session_view/room_history/event_row.rs` | Highlighting the focused event, and now the highlighted one too |
| `src/utils/grouping_list_model/mod.rs` | The position conversion fix |
| `data/resources/stylesheet/_room_history.scss` | Focused-event highlight |
| `src/shortcuts-dialog.blp` | Search Messages, `<primary>F` |
| `data/resources/stylesheet/_room_details.scss` | The drop target's `statuspage` rule scoped to `dragoverlay > overlay > revealer > statuspage`. Upstream owns the rule |
| `commune-core/src/session/room/search.rs` | Where both backends, `index_loaded_events()` and `reindex()` live now; `src/session/room/search.rs` wraps it |

## Rebase guide

1. `experimental-search` in `Cargo.toml` is the first thing to check. Losing it
   does not fail to compile in an obvious place — the local path simply stops
   existing, and encrypted rooms return nothing with no error.
2. `search_index_store()` on the client builder, same reason. Losing it is
   worse than losing the feature: the index falls back to memory and the app
   still works.
3. Upstream is active in `room_history/mod.rs` and `timeline/mod.rs`. Take
   theirs for anything not about focus, and re-apply the `is_focused()` guards
   one at a time against the table above rather than resolving the hunks
   mechanically. A dropped guard is a read receipt sent from a timeline
   showing last year, which is a data-losing bug and not a visible one.
4. `<primary>F`, not `<ctrl>F`. The binding in `room_history/mod.rs` uses
   `key_bindings::PRIMARY_MASK`. This was wrong in the first version of the
   branch and is easy to reintroduce.
5. `reindex()` reaches past the SDK's stable surface —
   `client.search_index().lock().await.bulk_handle_timeline_event(...)`, plus
   `room_version_rules_or_default().redaction` for the redaction rules. All of
   it is behind `experimental-search` and is the most likely thing here to be
   renamed or reshaped by an SDK bump. Its error type belongs to a crate that
   is not a direct dependency, which is why the result is mapped to `String`
   rather than named.
6. The search-time sweep is `index_loaded_events()` in
   `commune-core/src/session/room/search.rs`, called from `load_local()` and
   from `reindex()`. It exists because the SDK does not index an event that is
   decrypted after it is stored. **On an SDK bump, check whether that is still
   true** — `replace_in_memory_utds()` in the event cache's room state, and
   whether `search_indexing_task` now hears about redecryptions. If it does,
   the sweep is redundant but harmless; do not remove it until a test shows a
   message decrypted late being found without it.
7. `dragoverlay > overlay > revealer > statuspage` in `_room_details.scss`.
   The rule is upstream's, and a rebase that takes their version of that file
   brings back the descendant selector and the accent-purple search pane. The
   compiler says nothing; only looking at the pane does.
8. Any new `.rs` file here calls `gettext`, so it belongs in `po/POTFILES.in`,
   alphabetically. `hooks/checks-bin` catches it, but read its message with
   care: the two for that check are swapped, so "Found N file(s) in
   POTFILES.in without translatable strings" in fact means those files _have_
   translatable strings and are _missing_ from POTFILES.in.
9. The Esc controller and the `stop-search` handler sit at the end of
   `init_search()` in `room_history/mod.rs`, and `close_search()` next to
   `header_bar()` in the public impl; `search_bar` is `pub(super)` for it. The
   conflict will be there and in `Content::set_item()`, both upstream's. If
   upstream changes how Esc closes a room — the `session.close-room` binding
   in `session_view/mod.rs` — check the controller is still nearer the focus
   than whatever does it now: a capture-phase handler above the room history
   would take Esc back from the search, and nothing but pressing it shows that.

## Not done

* Searching across all rooms rather than the open one.
* Presenting the context lines around a result. `EventContext` is requested as
  zero on both sides; going to the message in the timeline is the answer
  instead.
* Searching state events, membership changes or attachment filenames.
* Any explanation, in an encrypted room, of _why_ results are limited to what
  this device has seen.
* Searching what was never loaded. Every search now indexes what the room has
  loaded, decrypted late or not — so scrolling back and searching works, with
  no button. But a room that has never been scrolled back still cannot find
  what it never loaded; nothing paginates on the search's behalf.
* Re-indexing every room at once, or on upgrade. The sweep is per room, at
  search time, so a device coming from a build without an index stays
  unsearchable in a room until that room is searched — which is now the same
  moment it is needed, rather than a second visit to press a button.
* A persistent index on Windows. It is in memory there, so every launch starts
  from nothing and only the history loaded since is searchable.
* Highlighting the matched terms within a result row.

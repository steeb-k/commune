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
this device has received and indexed. A message the device never saw is not
findable here at all. A message it saw but never indexed can be recovered —
see the next section, which is the subtler half of the same problem.

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

**Re-index This Room** is the remedy: `RoomSearch::reindex()` takes the events
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
event, with `Timeline::find_event_position()` — the same lookup `scroll_to_event`
does, and the only "is this loaded" answer either the SDK or this tree offers.
If it does, the live timeline stays and the event is scrolled to and highlighted
in place. Only an event that is genuinely not loaded gets a timeline of its own,
which is the case that mode exists for.

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
| `src/session_view/room_history/mod.rs`, `.blp` | Swapping timelines, the search pane, `<primary>F`, and `focus_on_event()` preferring the live timeline with a pending highlight |
| `src/session_view/mod.rs` | Permalinks open on the event |
| `src/session_view/content.rs` | Passing the focused event through |
| `src/session_view/room_details/history_viewer/event.rs` | Uses the event's own timeline, which may be focused |
| `src/session_view/room_history/event_row.rs` | Highlighting the focused event, and now the highlighted one too |
| `src/utils/grouping_list_model/mod.rs` | The position conversion fix |
| `data/resources/stylesheet/_room_history.scss` | Focused-event highlight |
| `src/shortcuts-dialog.blp` | Search Messages, `<primary>F` |

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
6. Any new `.rs` file here calls `gettext`, so it belongs in `po/POTFILES.in`,
   alphabetically. `hooks/checks-bin` catches it, but read its message with
   care: the two for that check are swapped, so "Found N file(s) in
   POTFILES.in without translatable strings" in fact means those files _have_
   translatable strings and are _missing_ from POTFILES.in.

## Not done

* Searching across all rooms rather than the open one.
* Presenting the context lines around a result. `EventContext` is requested as
  zero on both sides; going to the message in the timeline is the answer
  instead.
* Searching state events, membership changes or attachment filenames.
* Any explanation, in an encrypted room, of _why_ results are limited to what
  this device has seen. "Re-index This Room" offers the remedy but says nothing
  about the cause, and a room that has never been scrolled back still cannot
  find what it never loaded.
* Re-indexing every room at once, or on upgrade. It is per-room and manual, so
  a device coming from a build without an index stays mostly unsearchable until
  each room is visited.
* Highlighting the matched terms within a result row.

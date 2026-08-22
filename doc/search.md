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
this device has received and indexed. A message from before this session was
created is not findable here, and there is nothing the client can do about it.

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
| `src/session/room/search.rs` | `RoomSearch`, `RoomSearchResult`, query sanitisation, both backends |
| `src/session_view/room_history/search/mod.rs`, `.blp` | The search pane over the timeline |
| `src/session_view/room_history/search/row.rs`, `.blp` | One result row |

Integration points, which are where a rebase will conflict:

| File | Change |
| --- | --- |
| `Cargo.toml` | `experimental-search` on both `matrix-sdk` and `matrix-sdk-ui` |
| `src/utils/matrix/mod.rs` | `search_index_store()` on the client builder; `original_message_event_from_raw()` |
| `src/session/room/mod.rs` | `mod search;` and its re-exports |
| `src/session/room/timeline/mod.rs` | Focused mode: `new_focused()`, `is_focused()`, forward pagination, the guards |
| `src/session/room/timeline/virtual_item.rs` | The end-of-timeline spinner |
| `src/session_view/room_history/mod.rs`, `.blp` | Swapping timelines, the search pane, `<primary>F` |
| `src/session_view/mod.rs` | Permalinks open on the event |
| `src/session_view/content.rs` | Passing the focused event through |
| `src/session_view/room_details/history_viewer/event.rs` | Uses the event's own timeline, which may be focused |
| `src/session_view/room_history/event_row.rs` | Highlighting the focused event |
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
5. Any new `.rs` file here calls `gettext`, so it belongs in `po/POTFILES.in`,
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
* Any indication in an encrypted room that results are limited to what this
  device has seen. The limitation is real and currently invisible.
* Highlighting the matched terms within a result row.

# Guardrails — downstream implementation notes

Three small guards from the 26 August SDK audit's "smaller wins" list,
built 26 August 2026 in one sitting, plus two entries the audit priced
and this round declined. None of them is a feature; each is a mistake
the app used to let happen.

## A duplicate direct chat is not created

`User::get_or_create_direct_chat` checked only the local heuristic — a
joined room whose computed direct member matches — before creating a new
room. The SDK's `Client::get_dm_room()` reads `m.direct` itself, so it
still finds the direct chat whose membership does not currently look
like one (the other side left, the room not yet recomputed), which is
exactly the case that used to end in a duplicate. It is asked second,
after the local check and before creating.

## Logging out the last device says what it costs

The logout page hedged: "if this is your last connected session…".
`Recovery::is_last_device()` answers the question, so when verification
or recovery is not set up the warning now says plainly that this _is_
the last session and encrypted messages will be lost for good. An error
from the check keeps the hedged sentence — guessing "not last" would
soften a warning that exists to be heard.

## A too-large file is refused before the upload

`Client::load_or_fetch_max_upload_size()` is asked before an attachment
is sent — the answer is cached after the first ask — and a file over the
homeserver's limit gets a toast naming the limit instead of a full
upload that ends in a refusal. When the limit cannot be had, the upload
proceeds and the server stays the judge.

## Declined, and why

* **Merging the receipt requests** (`send_multiple_receipts`): the read
  receipt and the fully-read marker fire on different clocks on purpose
  — the receipt promptly, the marker after a timeout and never from a
  thread. Folding them into one request would change when each moves to
  save one HTTP round trip.
* **The storage settings page** (`get_store_sizes`, `optimize_stores`,
  a configurable `MediaRetentionPolicy`): a page of its own, deferred
  whole rather than shipped as a stub.

## Rebase guide

Three point edits: one early-return block in `user.rs`, one sentence
fork in `log_out_subpage.rs` (the warning update became async for it),
one pre-flight block at the top of the toolbar's `send_attachment`.

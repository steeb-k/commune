# Message shields — downstream implementation notes

Round 8 of `doc/gap-closing-plan.md`, first item, built 26 August 2026:
the per-message trust verdict, and the real reason a message cannot be
decrypted. Both verdicts were computed by the SDK on every timeline item
all along — the fork's work is saying them.

## The shield

`EventTimelineItem::get_shield(false)` — the non-strict bar, the one the
other clients present by default — answers for every item of an encrypted
room: nothing for an ordinary well-attested message, a grey shield for a
caveat (authenticity not guaranteed), a red one for a warning (unverified
or mismatched sender, an unsigned or unknown device, a message sent in
the clear in a room that encrypts). `Event::shield()` exposes it and the
message row draws it: a small icon beside the delivery state, red in the
error color, grey dimmed, with a tooltip sentence per code
(`shield_message` in `message_row/mod.rs`). Most messages draw nothing,
which is what keeps the two that do readable.

The strict bar and `ClientBuilder::with_decryption_settings` stay
untouched: the default trust requirement is the field's, and a setting
for it can come the day somebody asks.

## The undecryptable message says why

`MsgLikeKind::UnableToDecrypt` used to bind its payload to `_` and draw
one sentence for every failure. The payload carries `UtdCause`, and each
cause now reads differently (`could_not_decrypt_message` in
`message_row/content.rs`): sent before the account joined the room;
withheld by the sender, with or without the reason being this device's
verification; the sender's identity changed since verification; history
unavailable because key backup is off, or because this session is not
verified yet — the one case where the sentence can say what to do. Only
the unknown cause still promises a retry, because only there is one
coming. The `matrix-sdk-base` crate joined the dependencies for the
`UtdCause` type; it was in the tree all along.

## Rebase guide

* `Event::shield()` is one method; the icon is one template child in the
  message row's grid, updated beside the thread chip on item changes.
* The cause sentences live in one function each in `content.rs` and
  `mod.rs`; if the SDK grows a new `UtdCause` or shield code, the match
  breaks loudly and the new arm writes itself.

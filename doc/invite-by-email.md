# Invite by email — downstream implementation notes

Round 7 of `doc/gap-closing-plan.md`, built 26 August 2026: the invite
subpage takes an email address now, not only a Matrix ID — the gap the spec
board ranked highest after threads, because the person deciding whether to
try Matrix at all was the one person the page could not reach.

## The shape: a third slice count, in one round

Three pieces, each a commit: the identity server module, the affordance on
the invite subpage, and the configuration row in the account settings.

## The identity server module

`src/session/identity_server.rs`. The homeserver forwards an email
invitation, but only a client can pick the identity server and register
with it, so the module does, lazily and cached per run:

* **Resolution.** The `m.identity_server` account data is asked first —
  its tri-state matters, and an explicit `null` means "none, and stop
  asking", which surfaces as its own error rather than a fallback. With no
  preference, the `.well-known` of the account's server name (not the
  homeserver's host — on matrix.org the two differ) suggests one. Whatever
  is found must answer `GET /_matrix/identity/v2` before it is believed.
* **Registration.** `POST /user/{id}/openid/request_token` on the
  homeserver, whose response is exactly the body of
  `POST /_matrix/identity/v2/account/register`; the identity server
  answers with the token every later call carries.
* **Terms.** `GET /terms`, filtered against the `m.accepted_terms`
  account data (any accepted translation of a policy accepts the policy),
  each pending policy picked in the interface's language with gettext's
  fallback order. Accepting posts to the identity server **and** records
  the URLs in `m.accepted_terms`, so no client asks about the same
  documents twice. Nothing talks to any identity server until somebody
  asks to invite by email — registering is telling a third party who the
  account is, and that is not a call the module makes on its own.

The requests are ruma's own types — the pin carries the whole Identity
Service API behind `identity-service-api-c` — driven over the SDK's
reqwest client with `try_into_http_request`, because `client.send()` only
targets the homeserver. Defining endpoints in-app panics (the
`mutual_rooms` lesson); using ruma's is fine.

## The invite subpage

When the search text is an email address — the account settings' rule: an
`@` between two non-empty halves — a card appears under the search entry:
"Invite {email} by email". Activating it runs the whole flow: readiness,
the terms dialog when the identity server has unaccepted policies (links
to each document, Agree/Cancel, asking again at most once), then
`invite_user_by_3pid` with the server's host and the registered token. The
address never enters the checkbox list: that list, its pills and its
failure handling are all keyed on Matrix user IDs, and an email invitee is
gone from the room's point of view the moment the homeserver accepts it —
the toast is the whole lifecycle.

## The settings row

_Identity Server_ under Account Settings ▸ General ▸ Privacy names the
server that would be used and on whose word — set on this account,
suggested by the homeserver, declined, or none. Editing offers the
tri-state the account data has: a URL (validated against the status
endpoint before it is written), an empty field for "none at all", or the
homeserver's suggestion. Saving resets the module's caches, so the next
invite resolves and registers afresh.

## Rebase guide

* `identity_server.rs` is new; its only session coupling is the
  `IdentityServer` cell on `Session` and `spawn_tokio`.
* The invite subpage changes are additive: three template children, the
  text-notify hook, and the flow methods. The checkbox list is untouched.
* The settings row is one ActionRow plus two methods on the general page.
* If a later SDK grows identity-server support, the module shrinks from
  the transport up; the flow and the UI stay.

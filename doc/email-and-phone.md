# Email and phone on the account — downstream implementation notes

The second half of round 5 in `doc/gap-closing-plan.md`: the third-party
identifiers of the account, which upstream Fractal never showed at all — its
account settings deferred to "Manage Account" in a browser, which only exists
on a homeserver with the OAuth 2.0 API. On a password-auth homeserver there
was nothing.

## Scope

_Email and Phone Numbers_, a subpage off Account Settings ▸ General, beside
_Change Password_ and behind the same kind of gate:

* **Every identifier on the account is listed** — emails and phone numbers —
  from `GET /account/3pid`.
* **An email address can be added.** The request-token flow the password
  reset built the pattern for: `POST /account/3pid/email/requestToken` with a
  generated client secret, a dialog saying a link was sent, and
  `POST /account/3pid/add` once the link was opened — routed through
  `AuthDialog`, since adding an identifier is a sensitive operation and the
  homeserver asks for the password through UIAA.
* **Any identifier can be removed**, behind a confirmation that says what is
  lost: an address the homeserver could have used to reset the password
  cannot be after this. `POST /account/3pid/delete`, no UIAA.
* **A phone number cannot be added, on purpose.** Validating one takes a
  text message, and almost no homeserver runs SMS infrastructure — the
  stance recorded in the plan. The phone group says so in its description
  and only appears when the account has a number from somewhere else.

## The two failure modes of adding are told apart

Pressing _Continue_ before opening the link is not an error, it is a "not
yet": the homeserver answers `M_THREEPID_AUTH_FAILED`, the page toasts
"Open the link in the email first, then try again", and the dialog comes
back. Cancelling the `AuthDialog` also returns to the dialog rather than
throwing the validation session away — the link may simply not have been
opened yet. Only a real error, or _Cancel_ on the dialog itself, gives up;
the validation session then just expires on the server.

A second press of _Add_ for the same address keeps the client secret and
bumps `send_attempt`, which is the spec's way of saying "send the email
again" rather than "start over" — the same bookkeeping as the reset page,
for the same reason.

## Reachability

The row is hidden when the homeserver's account management lives in the
browser (`account_management_uri` in the OAuth 2.0 server metadata), because
that page manages identifiers too — so on matrix.org, the default homeserver,
this UI never appears, the same reachability as _Change Password_ and the
_Forgot Password?_ link. Where the row does appear, the `m.3pid_changes`
capability decides whether the add and remove affordances draw; a homeserver
that forbids changes still gets the list.

## Files

* `src/account_settings/general_page/third_party_ids_subpage.rs` + `.blp` —
  the subpage.
* `src/account_settings/general_page/mod.rs` + `.blp` — the row and its
  visibility.
* `src/account_settings/mod.rs` — the `ThirdPartyIds` subpage variant.

## Rebase guide

* The subpage is entirely ours; the conflict surface is the two additive
  blocks in the general page and the subpage enum in
  `account_settings/mod.rs`.
* The add flow leans on `AuthDialog::authenticate` handling the UIAA loop
  and on `error.client_api_error_kind()` naming `ThreepidAuthFailed`; an SDK
  bump that reshapes either fails loudly here.

## Not done

* Adding a phone number — see above, a stance rather than a gap.
* Binding an identifier to an identity server (`/account/3pid/bind`), which
  is what makes an address _discoverable_ by other users. Nothing here
  touches identity servers; the identifiers are only between the account and
  its homeserver. Round 7 (invite by email) is where identity-server
  configuration arrives, and binding can be revisited with it.
* `submit_url` handling: a homeserver that wants the token submitted by the
  client rather than by opening a link. Synapse sends a link; the flow here
  requires one.

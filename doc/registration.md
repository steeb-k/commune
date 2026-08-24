# Signing up, and resetting a password — downstream implementation notes

This file is the ledger for getting into an account without leaving the app:
creating one, getting back into one whose password is gone, and — since both
flows share it — the page that asks which homeserver any of it happens on. It
carries what the fork added, the decisions behind it, and what to check when
rebasing onto a new Fractal release. See `fork.md` for why none of this goes
upstream.

## Scope

* A _Create Account_ button on the greeter that is no longer dead.
* On a homeserver with the native API: a page for a username and a password,
  with the username checked against the homeserver as it is typed, and the
  account created through the user-interactive authentication the server asks
  for.
* On a homeserver with the OAuth 2.0 API: the same in-browser flow the log-in
  path already uses, with `prompt=create` so the server opens its sign-up form
  rather than its sign-in form.
* Pages for the two authentication stages a homeserver actually asks a new
  account for — a registration token, and agreeing to its policy documents —
  drawn here rather than in the browser.
* Straight into the session once it exists — the same path a password login
  takes, including the encryption setup pages.
* A _Forgot Password?_ link on the password login page, which asks the
  homeserver to email a link and then takes a new password.
* A homeserver page that offers `matrix.org` first, for logging in as well as
  signing up, with the empty entry it used to be behind a second choice.

Upstream has none of it: the greeter's _Create Account_ button was hidden and
pointed at an `app.create-account` action that existed nowhere in the tree, and
the word "reset" appears in its source only for cross-signing keys.

## The dialog had to stop needing a session

`AuthDialog` runs the whole UIAA stage loop and is the reason registration is
cheap here — it already answered `m.login.password` and `m.login.dummy` natively
and sent every other stage to the spec's own fallback page,
`GET /_matrix/client/v3/auth/{type}/fallback/web`, which is the standard's
answer to a captcha, a terms checkbox or an emailed token.

It was `construct_only`-bound to a logged-in `Session`, because until now every
flow that used it was something an account did to itself. Registration is the
one flow where the account is the thing being created, so the dialog now holds
a `matrix_sdk::Client` and an `Option<OwnedUserId>`:

* `AuthDialog::new(session)` is unchanged for the four existing callers.
* `AuthDialog::for_client(client, None)` is the registration path.

Only one stage needs the user ID — `m.login.password`, which identifies the
user to the homeserver — and it cannot appear in a registration flow for that
same reason. It gets `AuthError::MissingUserId` rather than the catch-all, so a
server that asks for it anyway leaves something legible in the log.

## Two stages are drawn here, and the rest still go to the web page

`m.login.registration_token` and `m.login.terms` are the two stages a
homeserver actually asks a new account for, so they became pages of their own
rather than trips to the fallback page:

* **The token page** is the password page with a plain entry: the token is not
  a secret to be hidden from the person typing it, and it usually arrives by
  message from whoever runs the server. A wrong one comes back as the same
  stage again, and the dialog now says "The registration token is invalid"
  instead of "An unexpected error occurred".
* **The terms page** is a check button per policy document, each with a link
  that opens it, and the _Agree_ button stays insensitive until every one is
  checked. The policies come from the flow's `params`, not the stage's, which
  is why `page()` now takes the whole `UiaaInfo` — `AuthState` only carries the
  stage.

`AuthState::next` prefers both of them the way it already preferred password,
SSO and dummy, so a flow that offers a native stage and a web-only one takes
the native one.

Three things are deliberately not native. `m.login.recaptcha` and
`m.login.email.identity` keep going to
`GET /auth/{type}/fallback/web`, because a captcha is a Google widget and an
emailed token needs a 3PID layer this tree does not have. And a terms stage
whose params do not parse, or carry no policies at all, falls back to that same
page rather than showing an empty list — a server that asks for agreement is
owed an answer it can recognise.

**Policy documents come in translations**, keyed by language tag, and the spec
notes that servers write both `en-US` and `en_US`. `preferred_translation()`
normalises both spellings, walks `glib::language_names()` in order, tries the
bare language before moving on, then falls back to English and finally to
whatever is there. A policy with no translation at all is dropped from the list:
agreeing to a document nobody can read is worse than not offering it.

## The greeter's action is `login.create-account`, not `app.create-account`

The dead button named an application action. It is a widget action on `Login`
instead, beside `login.sso` and `login.open-advanced`, because everything it
touches is inside that widget: the flow's purpose, the navigation view and the
client. An application action would have had to find the window, then the
login view, to set a flag on it.

## One homeserver page, two purposes

`Login` carries a `LoginPurpose` — `LogIn` or `CreateAccount` — set by the
button that started the flow and reset to `LogIn` in `clean()`, which runs
whenever the greeter becomes visible again. The homeserver page is untouched
and unaware: it builds the client and asks `discover_login_api()` which API the
server speaks, and the two `init_*_login()` methods on `Login` branch on the
purpose at the end.

That is why _Create Account_ goes through the same domain-name entry, the same
autodiscovery switch and the same "is this even a homeserver" check as logging
in, with no second copy of any of it.

## The homeserver page offers one now

Upstream's homeserver page is an empty entry, a help line saying "for example
gnome.org", and nothing else. That asks a question most people cannot answer:
somebody who has never used Matrix does not have a homeserver in mind, and the
page will not let them past until they invent one.

So the page is a choice of two rows in a boxed list, with the entry behind the
second:

* **matrix.org**, checked by default, described as the biggest public
  homeserver. Choosing it needs no typing and _Next_ takes the focus, so the
  whole page is one keystroke.
* **Another Homeserver**, which reveals exactly the page that used to be there —
  the same entry, the same help line, the same validation.

Both flows get it, because both go through this page. Somebody logging in to an
account elsewhere picks the second row, the same as somebody signing up
elsewhere.

Three things fall out of it:

* **`homeserver()` answers for the choice**, not for the entry. When the first
  row is picked it returns `matrix.org` and the entry is not consulted at all,
  so a stale value left in it cannot leak into a login.
* **Auto-discovery is forced on for the default.** The advanced dialog's switch
  exists so somebody can give a URL instead of a domain name; `matrix.org` is a
  domain name and is always looked up as one. `use_autodiscovery()` says so in
  one place, and the _Advanced…_ button is hidden while the default is chosen
  rather than left there as a control that changes nothing.
* **`server_name()` moved onto the page.** `Login` used to derive it by asking
  its own auto-discovery property and sanitising the entry text; the page is the
  only thing that knows which of the two rows is picked, so it answers instead.
  That is what puts "Log in to matrix.org" on the next page.

The default is a constant, `DEFAULT_SERVER_NAME`. There is no setting for it and
no list of suggestions: a second name in that list is a recommendation this fork
would be making on somebody's behalf, and one default that is obvious to replace
is a smaller claim than a curated list.

## `prompt=create` is the whole of the OAuth path

On a server with the OAuth 2.0 API there is no `POST /register` to call — the
server owns the account lifecycle. OpenID Connect's
`prompt=create` (`Prompt::Create`) is what says "this person has no account
yet", and the SDK's authorization URL builder takes it directly.

We check `prompt_values_supported` in the authorization server metadata first
and refuse with a toast if `create` is not there. A server is allowed to ignore
a prompt it does not support, and ignoring this one would silently show a
sign-in page to somebody who came to sign up.

## The availability check is advisory, deliberately

`GET /register/available` is asked 500 ms after the last keystroke, with a
generation counter so a late answer about an older username is discarded — the
same shape as the GIF search debounce.

Its answers are grouped into four states, and only two of them block the
button: `Free` and `Unknown` allow the attempt, `Refused` does not, `Pending`
means the answer has not arrived. **An error that is not `M_USER_IN_USE`,
`M_INVALID_USERNAME` or `M_EXCLUSIVE` lands on `Unknown` and lets the attempt
through**, because the endpoint is rate-limited, a homeserver may not answer it
at all, and `POST /register` is the authority either way. Blocking on a
question the server declined to answer would make the button permanently
insensitive on such a server.

## `M_FORBIDDEN` means something else here

`UserFacingError` maps `M_FORBIDDEN` to "Invalid credentials", which is right
almost everywhere and wrong on this page: a homeserver with registration
switched off answers `POST /register` with `M_FORBIDDEN`. So the register page
intercepts that one kind and says the homeserver does not allow creating an
account; everything else goes through `to_user_facing()`.

Four kinds were added to `user_facing_error.rs` on the way —
`M_USER_IN_USE`, `M_INVALID_USERNAME`, `M_EXCLUSIVE` and `M_WEAK_PASSWORD`.
The last was already handled at one call site, in the change-password page,
and is now a message everywhere.

We do not ask whether registration is enabled before showing the page. There is
no endpoint for that question: `POST /register` either returns the flows or
refuses, and the refusal is the answer. So the page is shown, and the failure
arrives when the account is attempted.

## The password rules are ours, not the server's

The strength meter and its five rules — length, lower case, upper case, digit,
symbol — come from `validate_password()`, the same function the change-password
page uses, and the same level bar with the same five offsets. They are a client
opinion: the spec has no password policy and a homeserver's own rules can be
stricter or looser. A server that refuses a password we accepted answers
`M_WEAK_PASSWORD`, which is now a sentence rather than a raw message.

The offsets are added from Rust rather than the template, because Blueprint has
no syntax for `<offsets>` — that is why `change_password_subpage.ui` is still
XML and this page is `.blp`.

## Integration points

| What | Where |
| --- | --- |
| The purpose of the flow | `LoginPurpose`, `src/login/mod.rs` |
| The default homeserver | `DEFAULT_SERVER_NAME`, `src/login/homeserver_page.rs` |
| The action behind the button | `login.create-account`, `Login::class_init` |
| The page | `src/login/register_page.rs`, `.blp` |
| Where the native path forks | `Login::init_matrix_login` |
| Where the OAuth path forks | `Login::init_oauth_login` |
| The session-free dialog | `AuthDialog::for_client`, `src/components/dialogs/auth/mod.rs` |
| The token stage page | `src/components/dialogs/auth/registration_token_page.rs` |
| The terms stage page | `src/components/dialogs/auth/terms_page.rs` |
| Which stages are drawn natively | `AuthState::next`, and `AuthDialog::page` |
| The reset page | `src/login/reset_password_page.rs`, `.blp` |
| The password meter, shared | `src/utils/password.rs` |
| Error messages | `src/user_facing_error.rs` |

## Rebase guide

* **`AuthDialog` lost its `session` property.** If upstream adds a caller that
  passes `session` as a construct property, it will fail; the constructors to
  use are `new(session)` and `for_client(client, user_id)`.
* **The greeter button.** Upstream keeps it `visible: false` with an
  `app.create-account` action. A rebase that takes their version of
  `greeter.blp` puts the dead button back.
* **`homeserver_page.blp` is substantially ours** from the choice list down, and
  `build_client()` lost its `autodiscovery` argument — the page works it out.
  Taking upstream's version of either brings back the empty entry.
* **`Login::init_matrix_login` and `init_oauth_login`** each grew a branch at
  the top. Upstream changing what happens after the homeserver page is the
  thing to watch.
* **`AuthDialog::page()` takes a `UiaaInfo`** now, not just an `AuthState`. If
  upstream adds a stage page, it gets the params for free; if it changes the
  signature back, the terms policies are what breaks.
* **`Prompt`** comes from
  `ruma::api::client::discovery::get_authorization_server_metadata::v1`. If the
  SDK grows a `prompt` argument on `OAuth::login()` itself, prefer that over
  the builder method.

## What is not built

* **A native captcha or emailed-token stage.** Both go to the spec's fallback
  page, which is the correct standard answer and not a shortcut.
* **Email or phone on the account.** The registration request supports 3PID
  binding, and account settings still cannot show, add or remove an identifier.
  Reset needs only the request-token half of that layer, and that is all this
  builds; the row for the rest is still open.
* **Resetting by phone number.** `requestToken` has an msisdn twin and the same
  `AuthData` shape covers it. It needs a phone number entry with country codes
  to be worth anything, and nobody has asked.
* **A generated username.** `POST /register` will invent a localpart if the
  request omits one. The page always sends what was typed.

## Resetting a password is two requests and one secret

`POST /account/password/email/requestToken` and then `POST /account/password`,
with a `client_secret` that ties them together. Both are unauthenticated — the
whole point is that the person cannot log in — so both go through
`client.send()` rather than through `Account`, which needs a session.

The page is one navigation page with a two-page stack: the address, then the new
password. It does not move on until the homeserver has answered with a session
ID, because without one there is nothing to send the second request with.

Three decisions:

* **The address is not validated here.** Anything non-empty is sent. The
  homeserver is the only thing that knows which addresses are on which
  accounts, and a client that refuses an address the server would have accepted
  is worse than one that asks.
* **`send_attempt` goes up only when the user asks again.** That is what the
  field is for — it tells the homeserver "send another email" apart from "this
  is a retry of a request that may have been lost". The secret is kept across a
  resend, since it is what identifies the session, and a new address starts a
  new one.
* **`logout_devices` is left at `true`**, which is the endpoint's default, and
  is written out anyway rather than left implicit. Somebody who could not log in
  has had a password they did not control for as long as that lasted; every
  other session going is the point rather than a side effect.

**The "not yet" case is the interesting one.** Until the link in the email is
opened, `POST /account/password` answers 401 with a UIAA body rather than
succeeding. That is not a failure and is not shown as one: the toast says to
open the link and try again, and the button comes back. Only a non-UIAA error is
reported as an error.

`AuthData::EmailIdentity` cannot be built from its fields — `EmailIdentity` is
`non_exhaustive` outside ruma — so it goes through `AuthData::new()` with the
`threepid_creds` object the spec describes. `ThirdpartyIdCredentials::new()`
does have a constructor, and is what gets serialized into it.

## Nothing about reset is drawn for an OAuth homeserver

`deactivate_account_subpage.rs` establishes the split this was going to mirror:
on a server with the OAuth 2.0 API, send the user to
`account_management_url_with_action(...)` instead of doing it in-app. It turns
out there is nothing to mirror. The _Forgot Password?_ link lives on the
password login page, and that page is only ever shown by
`Login::init_matrix_login()` — a homeserver with the OAuth API goes to the
in-browser page and never sees it. The server's own sign-in page is where such a
person resets a password, which is the same place we would have sent them.

So the OAuth branch is unreachable rather than unwritten. If the method page
ever appears on an OAuth server, this is the thing that has to grow a branch.

**This is not a hypothetical, and it applies to the default.** `matrix.org`
answers `GET /_matrix/client/v1/auth_metadata` with 200 — checked
24 August 2026, issuer `https://account.matrix.org/`, with `login` and `create`
among its `prompt_values_supported`. So on the homeserver this client now offers
first, logging in and signing up both go to the browser, the password login page
never appears, and neither does the link to reset a password. That is correct:
`account.matrix.org` owns those accounts and its own sign-in page carries its own
"forgot password". It does mean **the reset page can only be reached on a
homeserver that does not delegate authentication** — the throwaway Synapse in
`testing/local-homeserver.sh` is one, and is where to look at it.

The thing this leaves undone, should it ever be wanted: the in-browser login page
could offer `account_management_uri` from that same metadata as a "manage this
account" link. Today it says nothing, and the server's own page is one click away
inside the browser it opens.

## One password meter, three pages

Changing a password, signing up and resetting a password all ask somebody to
invent a password, and all three say the same five things about it in the same
widgets. The third copy was the one too many: `utils::password` now holds
`draw_password_validity()` and `draw_password_confirmation()`, and all three
pages call them. `validate_password()` in `utils::matrix` is unchanged — it is
about the specification's advice on passwords, where this is about GTK.

The level bar's offsets are added from Rust in both new pages, because Blueprint
has no syntax for `<offsets>`; that is also why `change_password_subpage.ui` is
still XML.

## Testing

`testing/local-homeserver.sh` runs with `enable_registration` and
`enable_registration_without_verification`, so Synapse asks for
`m.login.dummy` and the account is created without a stage the user can see.
That is the happy path, and it hides the dialog completely.

`./testing/local-homeserver.sh signup token` makes the server require a
registration token and prints one, which is the only way to see the token page
without a public homeserver. `signup off` refuses registration, which is the
`M_FORBIDDEN` path. `signup open` puts it back.

**The terms page has no harness.** Synapse asks for `m.login.terms` only with a
`user_consent` block pointing at template files it renders itself, and that is
more homeserver configuration than the rest of this harness needs. The page is
written and has never been drawn.

Do not try registration variants against a public homeserver — it leaves junk
accounts behind. `matrix.org` has registration behind a captcha, which is worth
one run through the fallback page and no more.

**Password reset has no local harness either.** Synapse only sends email with an
SMTP server configured, and the harness has none — and it is also the only place
the page can be reached at all, since the default homeserver delegates
authentication (above). Against `matrix.org` the first
half is safe to exercise on an account you own — asking for the email — and the
second half changes a real password and logs out every other session, so do that
knowing it.

# Signing up — downstream implementation notes

This file is the ledger for creating an account from inside the app: what the
fork added, the decisions behind it, and what to check when rebasing onto a new
Fractal release. See `fork.md` for why none of this goes upstream.

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

Password reset is _not_ here yet; it is the other half of this row and is
tracked in the plan. Upstream has neither half: the greeter's
_Create Account_ button was hidden and pointed at an `app.create-account`
action that existed nowhere in the tree.

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
| The action behind the button | `login.create-account`, `Login::class_init` |
| The page | `src/login/register_page.rs`, `.blp` |
| Where the native path forks | `Login::init_matrix_login` |
| Where the OAuth path forks | `Login::init_oauth_login` |
| The session-free dialog | `AuthDialog::for_client`, `src/components/dialogs/auth/mod.rs` |
| The token stage page | `src/components/dialogs/auth/registration_token_page.rs` |
| The terms stage page | `src/components/dialogs/auth/terms_page.rs` |
| Which stages are drawn natively | `AuthState::next`, and `AuthDialog::page` |
| Error messages | `src/user_facing_error.rs` |

## Rebase guide

* **`AuthDialog` lost its `session` property.** If upstream adds a caller that
  passes `session` as a construct property, it will fail; the constructors to
  use are `new(session)` and `for_client(client, user_id)`.
* **The greeter button.** Upstream keeps it `visible: false` with an
  `app.create-account` action. A rebase that takes their version of
  `greeter.blp` puts the dead button back.
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

* **Password reset.** The other half of the row.
* **A native captcha or emailed-token stage.** Both go to the spec's fallback
  page, which is the correct standard answer and not a shortcut.
* **Email or phone at sign-up.** The registration request supports 3PID
  binding; nothing in the tree has a 3PID layer, and reset needs only half of
  one.
* **A generated username.** `POST /register` will invent a localpart if the
  request omits one. The page always sends what was typed.

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

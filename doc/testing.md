# The throwaway homeserver

`testing/local-homeserver.sh` runs a Synapse in podman, seeds it with the rooms
and accounts needed to exercise the parts of Commune that a normal account
cannot reach, and prints what to click.

It exists because some features cannot honestly be tested against a homeserver
anyone else uses:

* **Reporting** sends a real report to a real administrator. On a public server
  that is noise someone has to read. Here the administrator is an account you
  own and delete afterwards.
* **The restricted join rule** only appears on a room already restricted to a
  space. Making one by hand means an account that can create a space, a room
  inside it, and the `m.room.join_rules` state to bind them.
* **Link previews** need `url_preview_enabled`, which Synapse ships switched
  off and which most servers leave that way. Turning it on somewhere real is a
  security decision about that server, not a testing convenience — the endpoint
  makes the homeserver fetch URLs on a user's say-so.

None of them is a thing to try on other people.

## Using it

```sh
./testing/local-homeserver.sh up       # start and seed; prints credentials
./testing/local-homeserver.sh check    # confirm the server accepts what we send
./testing/local-homeserver.sh reports  # show every report that arrived
./testing/local-homeserver.sh down     # stop, keep the data
./testing/local-homeserver.sh clean    # stop and delete everything
```

Then point Commune at `http://localhost:8008` and log in as `alice` or `bob`;
`up` prints the passwords. Everything lives under `testing/.homeserver`, which
is git-ignored, and `clean` removes it.

Needs `podman`, `curl`, `jq` and `python3`. The Synapse image is about 200 MB
on first run.

The configuration is only written when there is none, so a homeserver created
before a setting was added to the script keeps the old one. If `check` says
previews are off on a server you have used before, run `clean` and then `up`.

Previews also need the container to reach the internet, since the homeserver is
what fetches the page.

## What gets seeded

| Room | Why |
| --- | --- |
| Test Space | the space the restricted rooms point at |
| Restricted Room | `restricted` to the space — the case the join rule row is for |
| Knock Restricted Room | the same with `knock_restricted`, so both switch positions are visible without changing anything first |
| Invite Room | plain `invite`, for comparison, with a message in it to report |
| Knock Room | plain `knock` |
| Public Room | plain `public` |
| Link Room | seven messages covering what does and does not get a preview card |
| Encrypted Room | where a link must never get one, whatever the setting says |

Three accounts: `alice` owns the rooms, `bob` is a second member to report and
be reported, `admin` is a Synapse admin so reports can be read back.

## `check` versus the app

`check` drives the endpoints with `curl`, not through Commune. It answers one
question only — _does this homeserver accept what we intend to send_ — so that
a failure in the app is known to be the app's. It covers the room report
endpoint, the user report endpoint, a `knock_restricted` round trip that
asserts the allow list survives unchanged, an `m.room.server_acl` round trip,
and the URL preview endpoint — which also prints whether the image really came
back as an `mxc:` URI, since that is the one difference from OpenGraph the spec
names and the one thing the card refuses to render without.

It is not a test of Commune. The GUI still has to be driven by hand, which is
what `up` prints a checklist for.

## `reports`

Synapse has an admin API for event reports and none for the other two, so
`reports` reads all three tables — `room_reports`, `user_reports`,
`event_reports` — straight out of the SQLite database, read-only, with the
server still running.

This is where to confirm that a report Commune sent actually arrived, and that
the reason travelled with it.

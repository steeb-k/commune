# AGENTS.md

Rules for humans and chatbots alike:

PREFACE: If you aren't actually a human, go ahead and skip this section; it is genuinely not-useful
to an LLM.

* **not everything new is scary** and progress is inevitable

* **technologies that enable people to improve the world around them should not be treated with
  outright hostility** especially when someone is genuinely excited to share their work

* **karl marx would not be scared of computers** probably, i bet. dude ran a ton of newspapers i bet
  he'd love chat bot.

* **this is a snarky AGENTS.md header** because the original Fractal project file was childish. I
  did not intend on making this project public until my coding assistant told me the rules set out
  in AGENTS.md would cause it to be useless if followed - apparently the devs just do not understand
  how LLMs work, which, frankly, is unsurprising

Actual coding assistant data should go below the fold.

---

## For coding assistants

Read `doc/fork.md` first. In short: this is a permanent fork of Fractal and
nothing here is ever submitted upstream, so do not offer to open a merge
request or shape work around being acceptable to that project.

Upstream releases are still merged for security fixes. When you rebase, the
file you are reading now must keep this version: the upstream one is written
to stop an agent from working at all, and taking it would do exactly that.

Every feature this fork adds has a ledger under `doc/`, kept current with each
change to it: `doc/image-packs.md`, `doc/search.md` for message search,
`doc/gif-search.md`, `doc/reporting.md` for reporting a room or a user,
`doc/join-rules.md` for editing a restricted room's join rule,
`doc/server-acls.md` for which servers can take part in a room,
`doc/pinned-messages.md` for pinning a message in any room,
`doc/presence.md` for who is around and whether you say that you are,
`doc/registration.md` for creating an account from inside the app,
`doc/spaces.md` for spaces, `doc/peeking.md` for reading a room without
joining it, `doc/threads.md` for threads,
`doc/notifications.md` for the notification rules,
`doc/email-and-phone.md` for the identifiers on the account,
`doc/recent-emoji.md` for the quick reactions, `doc/url-previews.md` for
the card under a message that has a link, `doc/server-notices.md` for the
room the homeserver talks to the user in,
`doc/voice-messages.md` for recording and sending a voice message,
`doc/mutual-rooms.md` for the shared rooms on a profile,
`doc/policy-servers.md` for the `m.room.policy` state event in the
timeline, `doc/chat-bubbles.md` for the bubbled timeline layout,
`doc/invite-by-email.md` for inviting somebody who has no account yet,
`doc/message-shields.md` for the per-message trust verdicts,
`doc/failed-sends.md` for retrying a message that failed to send,
`doc/knocks.md` for the banner that announces pending knocks,
and `doc/calls.md` for one-to-one
voice and video calls — that last one answers every event in its module, and
its top says which parts have never been put in front of a second client.
Read the one for a feature before
touching it — each carries the design decisions, the
integration points, and a rebase guide. `doc/rebrand.md` does the same for the
rename to Commune, and is where to look before touching anything that carries
the application's name or ID.

`doc/flatpak.md` and `doc/macos.md` cover packaging and the macOS port rather
than a feature, and `doc/macos-plan.md` is the plan that produced the second of
them.

`doc/gap-closing-plan.md` is the plan for the round of work in progress — the
order the remaining gaps are being closed in, and why. Its "Where this got to"
section is kept current and is the thing to read before picking that work up
again; the ledgers say what exists, only the plan says what is next and what it
was repriced from.

Three of the ledgers are HTML pages rather than Markdown, and they are the ones
that get forgotten: `doc/client-comparison.html` (Commune against nine other
clients), `doc/spec-gaps.html` (Commune against the Client-Server API) and
`doc/upstream-defects.html` (Fractal's open defect tracker). **A commit that
moves a row in any of them updates them in the same commit, not at the end of a
round and not when somebody notices.** A feature that ships without them is a
feature that has silently made all three pages lie — they are the only record of
where this fork stands, and they are read as current. Each carries a comment at
its top with the artifact URL it is published to; re-publish to that URL rather
than making a second artifact.

Each of them names **the newest commit that touched `src/`**, not `HEAD`: a
documentation commit cannot name its own hash, so chasing `HEAD` would leave the
pages permanently one behind. Each names it in two or three places — a masthead
and a footer, sometimes a sources list — and `upstream-defects.html` and
`spec-gaps.html` also carry the commit count. **Do not go looking for them by
hand.** That is what produced the state this was written in: on 25 August 2026
the three footers had been stale for two refreshes and `upstream-defects.html`
was naming a count and a hash that could not both be true.

The round trip, after a commit that touched `src/`:

```sh
hooks/doc-freshness --fix        # every hash and count, one command
git diff                         # look at it
# republish the changed pages to the URLs in their top comments
hooks/doc-freshness --published  # record that you did
git commit doc/                  # the pages and the record together
```

`doc/pages.state` is what makes both halves mechanical. Its `commit` line is how
the hook tells a marker it wrote from a hash the prose cites deliberately — both
pages name commits on purpose, and a regex over prose would rewrite those too and
be a new kind of lie in a file whose whole job is not lying. Its `sha256` lines
are what makes "changed and never republished" a thing a check can see rather
than a thing somebody has to remember.

`hooks/doc-freshness` also still warns when a commit changes `src/` and no
documentation at all. The pre-commit hook runs it, and it never blocks, because
only a person can tell which change genuinely needs no ledger.

`doc/eyeball-tests.md` is the running list of what has been built and never
looked at on screen — every feature that draws adds to it in the same commit,
and an entry is struck only once the user reports what they actually saw.
Nothing in it is verifiable by `cargo check`, clippy, the tests or
`hooks/checks-bin`.

`doc/testing.md` covers `testing/local-homeserver.sh`, a throwaway Synapse in
podman. Reach for it before asking the user to test anything that would send a
report to a stranger or that needs a space to exist.

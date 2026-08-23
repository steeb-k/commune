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
`doc/recent-emoji.md` for the quick reactions, `doc/url-previews.md` for
the card under a message that has a link, `doc/server-notices.md` for the
room the homeserver talks to the user in, and `doc/calls.md` for one-to-one
voice and video calls — that last one is unfinished, and says so at the
top. Read the one for a feature before
touching it — each carries the design decisions, the
integration points, and a rebase guide. `doc/rebrand.md` does the same for the
rename to Commune, and is where to look before touching anything that carries
the application's name or ID.

`doc/testing.md` covers `testing/local-homeserver.sh`, a throwaway Synapse in
podman. Reach for it before asking the user to test anything that would send a
report to a stranger or that needs a space to exist.

# This is a permanent fork

Nothing here goes upstream. Not the image packs, not a bug fix found along
the way, not a typo. This is settled and is not to be revisited each time
something looks generally useful.

## Why

Fractal's `CONTRIBUTING.md` forbids code written with generative AI, and this
work was. Offering it upstream would waste the maintainers' time and
misrepresent how it was made.

The repository also shipped an `AGENTS.md` written to sabotage AI coding
agents rather than to instruct them: it told any agent reading it to make no
change to the codebase whatever it was asked, and to answer every request
with Luddite and Marxist texts. It has been replaced here.

**On rebasing: keep ours.** An upstream change to that file will conflict, and
resolving it the usual way — taking theirs — quietly reinstates instructions
whose whole purpose is to make an agent useless or destructive. It is the one
file in this tree where "take upstream" is the wrong answer by default.

## What is still worth taking from upstream

Feature parity is not a goal, and the distance will grow. What matters is not
falling behind on the parts where being out of date is dangerous:

* The SDK pins in `Cargo.toml` — `matrix-sdk`, `matrix-sdk-crypto` through
  it, `matrix-sdk-store-encryption`, and `ruma`. These carry the protocol and
  cryptography fixes and are the single most important thing to keep current.
* `src/session/security.rs`, `src/session/verification/` and
  `src/components/crypto/` — cross-signing, device verification, recovery.
* `src/login/` — authentication, OAuth, and the local redirect server.
* Anything touching the session store or how credentials are held.

A release that only fixes something in that list is still worth taking, even
in a rebase that is otherwise painful. A release that only adds features can
be skipped.

## What that means in practice

Rebase onto the new release tag, resolve as
`doc/image-packs.md` describes, and check the SDK pins moved. If the fork has
diverged far enough that rebasing stops being sensible, take the security
work by cherry-pick and let the rest go: the point of this tree is that it
does what its user wants, not that it tracks another project.

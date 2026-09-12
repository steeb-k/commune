# The Commune update feed

This branch is written only by the release workflow. It holds one manifest
per release channel and a detached signature for each, and nothing else: no
source, no history shared with `main`.

An installed Commune reads `<channel>.json` and `<channel>.json.sig` from
here to find out whether a newer release exists. It checks the signature
against a public key compiled into the application before it reads a single
field, so editing a manifest by hand only breaks update checks — it cannot
make anybody install anything.

`stable` is tagged releases. `rc` is release candidates and the stable
releases that follow them. `nightly` is every build of `main`, and only a
Devel-profile installation follows it.

See `doc/updates-plan.md` and `commune-core/src/updates/mod.rs` on `main`.

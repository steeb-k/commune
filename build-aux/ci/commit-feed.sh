#!/usr/bin/env bash
#
# Put a written feed on the `updates` branch.
#
#   commit-feed.sh feed/ "v1.rc2"
#
# The branch is an orphan: it shares no history with `main` and holds nothing
# but the manifests and their signatures. That is what makes
# `raw.githubusercontent.com/steeb-k/commune/updates/stable.json` a stable URL
# to serve them from without a Pages site, an object store or an API quota —
# see `doc/updates-plan.md`.
#
# It is written only by this script. Nothing reads it back into a build, so a
# mistake here breaks update checks and nothing else, and is fixed by running
# the release workflow again.

set -euo pipefail

feed_dir="${1:-}"
label="${2:-manual}"

die() {
    printf 'commit-feed: %s\n' "$1" >&2
    exit 1
}

[ -d "$feed_dir" ] || die "not a directory: $feed_dir"

# Every manifest must have its signature beside it: half a pair published is
# an update check that fails for everybody until the next release.
for manifest in "$feed_dir"/*.json; do
    [ -f "$manifest" ] || die "no manifests in $feed_dir"
    [ -f "$manifest.sig" ] || die "$manifest has no signature"
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

git config --global user.name "${GIT_AUTHOR_NAME:-commune release}"
git config --global user.email "${GIT_AUTHOR_EMAIL:-noreply@github.com}"

# A worktree rather than a checkout, so this cannot disturb the tree the
# release was built from.
if git ls-remote --exit-code --heads origin updates >/dev/null 2>&1; then
    git fetch origin updates
    git worktree add "$work/updates" origin/updates
    git -C "$work/updates" checkout -B updates
else
    git worktree add --detach "$work/updates"
    git -C "$work/updates" checkout --orphan updates
    git -C "$work/updates" rm -rf . >/dev/null 2>&1 || true
fi

cp "$feed_dir"/*.json "$feed_dir"/*.json.sig "$work/updates/"

cat > "$work/updates/README.md" <<'EOF'
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
EOF

git -C "$work/updates" add -A

if git -C "$work/updates" diff --cached --quiet; then
    printf 'commit-feed: the feed is unchanged; nothing to push\n' >&2
    exit 0
fi

git -C "$work/updates" commit -m "feed: $label"
git -C "$work/updates" push origin updates

printf 'commit-feed: published %s to the updates branch\n' "$label" >&2

#!/usr/bin/env bash
#
# Print the version the project releases under.
#
# One place asks, so one place answers. `RELEASING.md` records that the same
# version wears three spellings — `1.rc1` in Meson and the app, `1.0.0-rc1` in
# `Cargo.toml`, `1~rc1` in the metainfo — and this prints the semver one,
# because that is the spelling the release feed compares and the one every
# artifact is named after.
#
# The `[package]` section, not the first `version =` in the file: the
# workspace table comes first and would one day grow one.

set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"

sed -n '/^\[package\]/,/^\[[^p]/p' "$root/Cargo.toml" \
    | sed -n 's/^version = "\(.*\)"/\1/p' \
    | head -1

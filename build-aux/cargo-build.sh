#!/usr/bin/env bash
#
# Build the binary with cargo and put it where meson expects it.
#
# Usage: ./cargo-build.sh CARGO BUILT_BINARY OUTPUT [CARGO_OPTION…]
#
# This exists because the two steps cannot be written as one meson command.
# Meson does not run a `custom_target` command through a shell, so a `&&`
# between them is passed to cargo as an argument rather than acting as a
# separator — which cargo rejects outright on Windows. It happens to be
# tolerated elsewhere, but a shell operator in an argument list is not something
# to keep relying on.
#
# The environment — `CARGO_HOME`, `CARGO_TARGET_DIR`, and the gettext variables
# on macOS and Windows — is set by meson on the target and inherited here.

set -e

cargo="$1"
shift
built_binary="$1"
shift
output="$1"
shift

"$cargo" build "$@"

# `cp` rather than `mv`: the built binary stays in the cargo target directory so
# that the next build is incremental.
cp "$built_binary" "$output"

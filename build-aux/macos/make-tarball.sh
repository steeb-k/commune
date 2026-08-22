#!/usr/bin/env bash
# Wrap a built Commune.app in a gzipped tarball.
#
#   build-aux/macos/make-tarball.sh "_build/macos/Commune.app" [out.tar.gz]
#
# This is the artefact to hand to somebody while the app is signed with an
# ad-hoc or self-signed identity, and the reason is Gatekeeper rather than
# taste. A browser tags anything it downloads with `com.apple.quarantine`, and
# a quarantined app whose signature is not from a Developer ID is refused on
# first launch -- so a .dmg, which is the familiar shape, is exactly the shape
# that will not open. Files extracted from a tarball on the command line are
# never quarantined in the first place. The sibling SEED Sync project ships a
# `curl | sh` tarball for the same reason.
#
# The signature survives the round trip: it lives inside the Mach-O files and in
# `Contents/_CodeSignature`, both of which are ordinary files.
set -euo pipefail

APP="${1:-}"
OUT="${2:-}"

[ -n "$APP" ] || {
    echo "usage: make-tarball.sh <Commune.app> [out.tar.gz]" >&2
    exit 2
}
[ -d "$APP" ] || {
    echo "make-tarball: no such bundle: $APP" >&2
    exit 1
}

APP="$(cd "$APP" && pwd)"
APP_NAME="$(basename "$APP" .app)"

VERSION="$(defaults read "$APP/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo 'unknown')"
ARCH="$(uname -m)"

if [ -z "$OUT" ]; then
    slug="$(printf '%s' "$APP_NAME" | tr '[:upper:] ' '[:lower:]-')"
    OUT="$(dirname "$APP")/$slug-$VERSION-$ARCH.tar.gz"
fi

rm -f "$OUT"

# `COPYFILE_DISABLE` keeps bsdtar from writing an AppleDouble `._name` file
# beside every entry that carries extended attributes. Those files are noise
# when the archive is unpacked anywhere else, and the bundle does not need them.
COPYFILE_DISABLE=1 tar \
    --create --gzip \
    --file "$OUT" \
    --directory "$(dirname "$APP")" \
    "$(basename "$APP")"

echo "make-tarball: wrote $OUT ($(du -sh "$OUT" | awk '{print $1}'))"

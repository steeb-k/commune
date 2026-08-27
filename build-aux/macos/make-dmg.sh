#!/usr/bin/env bash
# Wrap a built Commune.app in a compressed disk image.
#
#   build-aux/macos/make-dmg.sh "_build/macos/Commune.app" [out.dmg]
#
# The image holds the app and a symlink to /Applications, which is the gesture
# every Mac user already knows: open the image, drag the icon across.
#
# Read `doc/macos.md` before handing one of these to anybody. An unsigned or
# ad-hoc-signed app inside a .dmg is the *worse* of the two things this
# directory can produce: a disk image downloaded by a browser is tagged with
# `com.apple.quarantine`, and Gatekeeper will not accept an ad-hoc signature for
# a quarantined app, so the first launch is refused outright. `make-tarball.sh`
# produces the artefact that actually launches. This one exists because it is
# the right shape once there is a Developer ID to sign with, and because a .dmg
# built and copied locally is a fine way to test the install itself.
set -euo pipefail

APP="${1:-}"
OUT="${2:-}"

[ -n "$APP" ] || {
    echo "usage: make-dmg.sh <Commune.app> [out.dmg]" >&2
    exit 2
}
[ -d "$APP" ] || {
    echo "make-dmg: no such bundle: $APP" >&2
    exit 1
}

APP="$(cd "$APP" && pwd)"
APP_NAME="$(basename "$APP" .app)"
VOLNAME="$APP_NAME"

# The version the bundle was built with, so the file name says what it holds.
# CommuneVersion is the full string, rc suffix included; the CFBundle key only
# carries the numeric prefix, and is the fallback for a bundle from before the
# split.
VERSION="$(defaults read "$APP/Contents/Info" CommuneVersion 2>/dev/null \
    || defaults read "$APP/Contents/Info" CFBundleShortVersionString 2>/dev/null \
    || echo 'unknown')"
ARCH="$(uname -m)"

if [ -z "$OUT" ]; then
    slug="$(printf '%s' "$APP_NAME" | tr '[:upper:] ' '[:lower:]-')"
    OUT="$(dirname "$APP")/$slug-$VERSION-$ARCH.dmg"
fi

STAGE="$(dirname "$OUT")/.dmg-stage"
rm -rf "$STAGE"
mkdir -p "$STAGE"

# `cp -R` on a bundle preserves the symlinks and the signature; `ditto` is used
# instead because it also carries the extended attributes across, and a bundle
# that loses them can fail to validate.
ditto "$APP" "$STAGE/$(basename "$APP")"
ln -s /Applications "$STAGE/Applications"

rm -f "$OUT"
hdiutil create \
    -volname "$VOLNAME" \
    -srcfolder "$STAGE" \
    -ov \
    -format UDZO \
    "$OUT" >/dev/null

rm -rf "$STAGE"

echo "make-dmg: wrote $OUT ($(du -sh "$OUT" | awk '{print $1}'))"

# Notarization is opt-in because this target is also the dev-iteration one:
# a submission uploads the image to Apple, wants the network, and takes some
# minutes -- none of which a local test build should pay. Set NOTARIZE_PROFILE
# to a `notarytool store-credentials` profile name to get a .dmg that opens
# straight off a download. It only makes sense over a real signature, so an
# ad-hoc bundle is refused before anything leaves the machine.
if [ -n "${NOTARIZE_PROFILE:-}" ]; then
    if codesign -dv "$APP" 2>&1 | grep -q 'flags=.*adhoc'; then
        echo "make-dmg: NOTARIZE_PROFILE is set but the bundle is ad-hoc signed;" >&2
        echo "  rebuild with CODESIGN_IDENTITY first -- Apple rejects ad-hoc submissions." >&2
        exit 1
    fi
    echo "make-dmg: notarizing (profile '$NOTARIZE_PROFILE'; takes a few minutes)"
    if ! xcrun notarytool submit "$OUT" --keychain-profile "$NOTARIZE_PROFILE" --wait; then
        echo "make-dmg: notarization failed; the reasons are in:" >&2
        echo "  xcrun notarytool log <submission id> --keychain-profile $NOTARIZE_PROFILE" >&2
        exit 1
    fi
    # The staple pins the ticket to the image so Gatekeeper can verify a
    # download with no network. Without it the .dmg still opens -- the ticket
    # is looked up online -- but stapling costs nothing and works offline.
    xcrun stapler staple "$OUT"
    echo "make-dmg: notarized and stapled"
fi

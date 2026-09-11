#!/usr/bin/env bash
#
# Sign a bundle with a Developer ID, notarize it, and staple the ticket.
#
#   sign-notarize.sh path/to/Commune.app
#
# `bundle.sh` already signs whatever it builds, but with an ad-hoc identity
# unless `CODESIGN_IDENTITY` names a real one, and it does not notarize. On a
# developer's Mac the real identity lives in the login keychain and
# `make-dmg.sh` handles the rest; on a runner there is no keychain and no
# stored notarytool profile, so both have to be built from secrets first.
# That is all this script is.
#
# # Why a second certificate exists
#
# The certificate on the maintainer's Mac is Xcode's cloud-managed kind: its
# private key syncs through iCloud Keychain and cannot be exported as a
# `.p12`, so CI cannot use it. A second Developer ID Application certificate
# under the same team, issued from a CSR generated anywhere, is what this
# takes. The team is what matters, not the certificate: an installed copy
# checks the Team ID of an update against its own, so a second certificate
# under the same team updates silently and one under a different team would
# not update at all.
#
# # Why the ticket is stapled to the .app
#
# `make-dmg.sh` staples the disk image, which is right for a disk image. The
# updater downloads a tarball, and a ticket stapled to a `.dmg` does not
# travel inside one — so the ticket goes on the bundle itself, before it is
# ever wrapped, and Gatekeeper can then check it with no network at all.
#
# Environment:
#
#   CODESIGN_IDENTITY     The identity's common name, e.g.
#                         'Developer ID Application: Someone (TEAMID)'.
#   CERTIFICATE_P12       The certificate and its key, base64 of a .p12.
#   CERTIFICATE_PASSWORD  The .p12's password.
#
# Then either set, for the notary service:
#
#   NOTARY_KEY            The .p8 private key contents.
#   NOTARY_KEY_ID         App Store Connect API key id.
#   NOTARY_ISSUER_ID      App Store Connect API issuer UUID.
#
#   NOTARY_PASSWORD       An app-specific password from appleid.apple.com.
#   NOTARY_APPLE_ID       The Apple ID it belongs to.
#   NOTARY_TEAM_ID        The team, e.g. VLC2KZKNBH.
#
# Or ALLOW_UNNOTARIZED=1 to sign without notarizing, which the updater can
# install and a browser download cannot.

set -euo pipefail

bundle="${1:-}"

die() {
    printf 'sign-notarize: %s\n' "$1" >&2
    exit 1
}

[ -n "$bundle" ] || die "usage: sign-notarize.sh path/to/Commune.app"
[ -d "$bundle" ] || die "not a bundle: $bundle"

for required in CODESIGN_IDENTITY CERTIFICATE_P12 CERTIFICATE_PASSWORD; do
    [ -n "${!required:-}" ] || die "$required is not set"
done

# How to authenticate to the notary service, or whether to at all.
#
# Notarization is not the App Store. It is the other half of Developer ID —
# the arrangement Apple provides for software distributed outside the store —
# and it involves no listing, no review and no app record. Worth stating
# plainly because the credential is called an "App Store Connect API key",
# and this project could not go in the store in any case: it is GPL-3 with
# many copyright holders, which the store terms do not permit.
#
# Two ways in, and for this purpose neither is better than the other:
#
#   * An App Store Connect team API key — a `.p8`, a key id, an issuer id.
#     Creating one needs a role on the developer account.
#   * An app-specific password from appleid.apple.com, with the Apple ID and
#     the team id. No portal, no roles.
if [ -n "${NOTARY_KEY:-}" ]; then
    notarize=key
    for required in NOTARY_ISSUER_ID NOTARY_KEY_ID; do
        [ -n "${!required:-}" ] || die "$required is not set, and NOTARY_KEY is"
    done
elif [ -n "${NOTARY_PASSWORD:-}" ]; then
    notarize=password
    for required in NOTARY_APPLE_ID NOTARY_TEAM_ID; do
        [ -n "${!required:-}" ] || die "$required is not set, and NOTARY_PASSWORD is"
    done
elif [ "${ALLOW_UNNOTARIZED:-0}" = "1" ]; then
    notarize=no
else
    die "no notary credentials: set NOTARY_KEY or NOTARY_PASSWORD, or ALLOW_UNNOTARIZED=1"
fi

work="$(mktemp -d)"
keychain="$work/build.keychain-db"
keychain_password="$(openssl rand -base64 24)"

cleanup() {
    security delete-keychain "$keychain" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

# A keychain of its own, deleted on the way out. The runner is thrown away
# anyway; this is so that a local run of the same script cannot leave a
# certificate behind in the login keychain.
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"

printf '%s' "$CERTIFICATE_P12" | base64 --decode > "$work/certificate.p12"
security import "$work/certificate.p12" \
    -k "$keychain" \
    -P "$CERTIFICATE_PASSWORD" \
    -T /usr/bin/codesign \
    -T /usr/bin/security

# Without this, every `codesign` call stops on a GUI prompt that nothing on a
# runner will ever answer.
security set-key-partition-list -S apple-tool:,apple:,codesign: \
    -s -k "$keychain_password" "$keychain" >/dev/null

# Put it in front of the search list rather than replacing it, so the system
# roots the chain is validated against are still reachable.
security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')

# Re-sign the whole bundle with the real identity. `bundle.sh` did this once
# already with an ad-hoc one and, on the universal path, `lipo` replaced every
# binary afterwards — so what is on disk now is unsigned as far as Gatekeeper
# is concerned.
#
# Nested first and the bundle last, and never `--deep`, which Apple
# deprecated: the same order `bundle.sh` uses.
find "$bundle" -type f \( -name '*.dylib' -o -name '*.so' \) -print0 \
    | xargs -0 -n1 codesign --force --timestamp --options runtime \
        --sign "$CODESIGN_IDENTITY"

find "$bundle/Contents/MacOS" "$bundle/Contents/Resources/libexec" -type f -perm -u+x 2>/dev/null \
    | while IFS= read -r executable; do
        codesign --force --timestamp --options runtime \
            --sign "$CODESIGN_IDENTITY" "$executable"
    done

entitlements="$(dirname "$0")/entitlements.plist"
codesign --force --timestamp --options runtime \
    --entitlements "$entitlements" \
    --sign "$CODESIGN_IDENTITY" "$bundle"

codesign --verify --deep --strict --verbose=2 "$bundle"

if [ "$notarize" = "no" ]; then
    # Deliberate, and worth saying loudly. A signed but un-notarized build
    # still installs through the updater, which extracts the tarball itself
    # and so never sets the quarantine attribute — but a person who downloads
    # the same tarball in a browser and unpacks it in Finder is refused on
    # first launch, with no way forward that does not involve System Settings.
    printf 'sign-notarize: WARNING: %s is signed but NOT notarized.\n' "$bundle" >&2
    printf 'sign-notarize: the updater can install it; a browser download cannot.\n' >&2
    exit 0
fi

# Notarization takes a zip, not a directory. `ditto -c -k` is the only
# archiver Apple documents for this.
ditto -c -k --keepParent "$bundle" "$work/notarize.zip"

if [ "$notarize" = "key" ]; then
    printf '%s' "$NOTARY_KEY" > "$work/notary.p8"
    xcrun notarytool submit "$work/notarize.zip" \
        --key "$work/notary.p8" \
        --key-id "$NOTARY_KEY_ID" \
        --issuer "$NOTARY_ISSUER_ID" \
        --wait
else
    xcrun notarytool submit "$work/notarize.zip" \
        --apple-id "$NOTARY_APPLE_ID" \
        --password "$NOTARY_PASSWORD" \
        --team-id "$NOTARY_TEAM_ID" \
        --wait
fi

# On the bundle, so that the ticket survives being put in a tarball.
xcrun stapler staple "$bundle"
xcrun stapler validate "$bundle"

printf 'sign-notarize: %s is signed, notarized and stapled\n' "$bundle" >&2

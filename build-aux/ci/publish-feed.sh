#!/usr/bin/env bash
#
# Write and sign a release feed manifest.
#
# The manifest is the one thing an installed Commune reads to find out that a
# newer release exists — see `commune-core/src/updates/mod.rs` for what it is
# and `doc/updates-plan.md` for why it is a static file rather than a call to
# the GitHub API. This script is what produces it, out of the artifacts a
# release has just built.
#
# It signs with an Ed25519 key whose public half is compiled into every build.
# That signature is not what stops a hostile artifact from being installed —
# Authenticode, Developer ID and the APK signing key do that, and the platform
# enforces them. It is what stops a rewritten feed from choosing *which*
# correctly signed build an installation ends up on, which is the attack an
# unsigned feed leaves open: pinning everybody to a release whose bugs the
# attacker knows.
#
# Usage:
#
#   UPDATE_FEED_PRIVATE_KEY="$(cat key.pem)" \
#   publish-feed.sh \
#       --channel stable \
#       --version 1.0.0 \
#       --build 4021 \
#       --notes-url https://github.com/steeb-k/commune/releases/tag/v1 \
#       --out-dir feed \
#       --asset 'windows-x86_64-msi|dist/Commune-1.0.0-x64.msi|https://…/Commune.msi' \
#       --asset 'macos-universal-tar|dist/commune-universal.tar.gz|https://…/commune.tgz'
#
# An `--asset` is `key|path|url`, separated by a vertical bar because both of
# the other two fields can contain a colon — every URL does, and so does a
# Windows path. The path is read to get the size and the digest; the URL is
# what goes in the manifest. A key with no artifact is simply left out, and an
# installation on that platform is told there is nothing to install rather
# than being sent at a URL that 404s.

set -euo pipefail

channel=""
version=""
build=""
notes_url=""
out_dir="feed"
published="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
assets=()

die() {
    printf 'publish-feed: %s\n' "$1" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --channel) channel="$2"; shift 2 ;;
        --version) version="$2"; shift 2 ;;
        --build) build="$2"; shift 2 ;;
        --notes-url) notes_url="$2"; shift 2 ;;
        --published) published="$2"; shift 2 ;;
        --out-dir) out_dir="$2"; shift 2 ;;
        --asset) assets+=("$2"); shift 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done

[ -n "$channel" ] || die "--channel is required"
[ -n "$version" ] || die "--version is required"
[ -n "$build" ] || die "--build is required"
[ -n "$notes_url" ] || die "--notes-url is required"
[ -n "${UPDATE_FEED_PRIVATE_KEY:-}" ] || die "UPDATE_FEED_PRIVATE_KEY is not set"

case "$channel" in
    stable|rc|nightly) ;;
    *) die "unknown channel: $channel (expected stable, rc or nightly)" ;;
esac

# The core parses this with serde and refuses a version it cannot read as
# semver, so a malformed one must fail here rather than reach a manifest.
printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' \
    || die "version is not semver: $version"
printf '%s' "$build" | grep -Eq '^[0-9]+$' || die "build is not a number: $build"

mkdir -p "$out_dir"

manifest="$out_dir/$channel.json"
signature="$out_dir/$channel.json.sig"

sha256_of() {
    # Both spellings exist across the runners this has to work on.
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

size_of() {
    # BSD and GNU `stat` disagree about everything, so ask `wc` instead.
    wc -c < "$1" | tr -d '[:space:]'
}

{
    printf '{\n'
    printf '  "channel": "%s",\n' "$channel"
    printf '  "version": "%s",\n' "$version"
    printf '  "build": %s,\n' "$build"
    printf '  "published": "%s",\n' "$published"
    printf '  "notes_url": "%s",\n' "$notes_url"
    printf '  "assets": {'

    first=1
    for asset in ${assets+"${assets[@]}"}; do
        key="${asset%%|*}"
        rest="${asset#*|}"
        path="${rest%%|*}"
        url="${rest#*|}"

        [ -n "$key" ] || die "asset has no key: $asset"
        [ -n "$path" ] || die "asset $key has no path"
        [ -n "$url" ] || die "asset $key has no URL"
        [ -f "$path" ] || die "asset $key is not a file: $path"

        if [ "$first" -eq 1 ]; then
            printf '\n'
            first=0
        else
            printf ',\n'
        fi

        printf '    "%s": {\n' "$key"
        printf '      "url": "%s",\n' "$url"
        printf '      "sha256": "%s",\n' "$(sha256_of "$path")"
        printf '      "size": %s\n' "$(size_of "$path")"
        printf '    }'
    done

    if [ "$first" -eq 0 ]; then
        printf '\n  '
    fi

    printf '}\n'
    printf '}\n'
} > "$manifest"

# The signature covers the bytes of the file exactly as written, which is what
# the core verifies before it parses a single field.
key_file="$(mktemp)"
raw_signature="$(mktemp)"
trap 'rm -f "$key_file" "$raw_signature"' EXIT
chmod 600 "$key_file"
printf '%s\n' "$UPDATE_FEED_PRIVATE_KEY" > "$key_file"

# `-rawin` is how Ed25519 is signed: it takes the whole message rather than a
# digest of it, because the algorithm hashes internally.
openssl pkeyutl -sign -rawin -inkey "$key_file" -in "$manifest" -out "$raw_signature"

# Hex rather than base64 or raw bytes, so that the branch this is committed to
# stays readable text and a bad signature can be eyeballed.
od -An -v -tx1 < "$raw_signature" | tr -d ' \n' > "$signature"
printf '\n' >> "$signature"

printf 'publish-feed: wrote %s (%s bytes) and its signature\n' \
    "$manifest" "$(size_of "$manifest")" >&2

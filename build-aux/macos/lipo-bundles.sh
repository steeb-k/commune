#!/usr/bin/env bash
#
# Merge an arm64 `Commune.app` and an x86_64 one into a universal bundle.
#
# Two runners build the same bundle from the same recipe, one on each
# architecture, and neither can build the other's half: the conda-forge
# environment `setup-conda-macos.sh` creates is per-subdir, and the three
# projects built from source are built by the machine they run on. So the
# merge happens afterwards, here.
#
# What actually differs between the two bundles is only the Mach-O files.
# Everything else — the resources, the schemas, the icon, the plist — is
# architecture-neutral and identical, and this checks that rather than
# assuming it: a file that differs and is *not* a Mach-O means the two builds
# diverged somewhere they should not have, and that is worth failing on rather
# than silently taking one side.
#
# Signing happens after this, never before. A signature covers all of a
# binary's slices at once, so signing the halves and then merging them
# produces a bundle whose signature does not check out.
#
# Usage:
#
#   lipo-bundles.sh --arm64 arm64/Commune.app \
#                   --x86_64 x86_64/Commune.app \
#                   --output universal/Commune.app

set -euo pipefail

arm64_bundle=""
x86_64_bundle=""
output=""

die() {
    printf 'lipo-bundles: %s\n' "$1" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --arm64) arm64_bundle="$2"; shift 2 ;;
        --x86_64) x86_64_bundle="$2"; shift 2 ;;
        --output) output="$2"; shift 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done

[ -d "$arm64_bundle" ] || die "--arm64 is not a bundle: $arm64_bundle"
[ -d "$x86_64_bundle" ] || die "--x86_64 is not a bundle: $x86_64_bundle"
[ -n "$output" ] || die "--output is required"

rm -rf "$output"
mkdir -p "$(dirname "$output")"

# `ditto` rather than `cp -R`, for the same reason `make-dmg.sh` uses it: it
# carries extended attributes, and a bundle that loses them can fail to
# validate.
ditto "$arm64_bundle" "$output"

# The signatures of the halves are ad-hoc and about to be replaced; leaving
# them in place would make `codesign` complain about signing over them.
find "$output" -name '_CodeSignature' -type d -prune -exec rm -rf {} +

merged=0
copied=0

while IFS= read -r -d '' file; do
    relative="${file#"$output"/}"
    other="$x86_64_bundle/$relative"

    case "$(file -b "$file")" in
        *Mach-O*) ;;
        *)
            # Not a binary. It must be byte-for-byte what the other build
            # produced, or the two builds are not the same build.
            if [ -f "$other" ] && ! cmp -s "$file" "$other"; then
                die "the two builds disagree about $relative, which is not a binary"
            fi

            copied=$((copied + 1))
            continue
            ;;
    esac

    if [ ! -f "$other" ]; then
        die "$relative is in the arm64 bundle but not the x86_64 one"
    fi

    lipo -create "$file" "$other" -output "$file.universal"
    # Keep the mode: `lipo` writes a plain file, and an executable that comes
    # out non-executable is a bundle that will not launch.
    chmod --reference="$file" "$file.universal" 2>/dev/null \
        || chmod "$(stat -f '%Lp' "$file")" "$file.universal"
    mv "$file.universal" "$file"

    merged=$((merged + 1))
done < <(find "$output" -type f -print0)

# Anything the x86_64 bundle has and the arm64 one does not is just as much a
# divergence as the other way round.
while IFS= read -r -d '' file; do
    relative="${file#"$x86_64_bundle"/}"

    if [ ! -f "$output/$relative" ]; then
        die "$relative is in the x86_64 bundle but not the arm64 one"
    fi
done < <(find "$x86_64_bundle" -type f -print0)

printf 'lipo-bundles: merged %s binaries, carried %s other files\n' "$merged" "$copied" >&2

# Worth stating in the log: this is the number that decides
# `LSMinimumSystemVersion`, and the whole reason the bundle can run on macOS
# 11 is that conda-forge builds against an old SDK whatever machine is used.
printf 'lipo-bundles: architectures in the executable: %s\n' \
    "$(lipo -archs "$output/Contents/MacOS/"* 2>/dev/null || echo unknown)" >&2

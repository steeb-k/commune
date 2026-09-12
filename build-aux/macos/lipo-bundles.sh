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
one_sided=0

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
        # A library that only one architecture's binaries link. The walk in
        # `bundle.sh` copies what `otool -L` actually names, and
        # conda-forge's two platforms do not always produce the same graph:
        # its osx-arm64 `libsqlite3` links `libicui18n` and its osx-64 build
        # of the same package does not.
        #
        # Carrying it through as a single-architecture file is correct and
        # not merely tolerable. dyld resolves by path, per slice: the arm64
        # slice that asks for this library finds it, and nothing in the
        # x86_64 slice ever asks. A universal bundle needs every file each
        # slice names, not every file fat.
        one_sided=$((one_sided + 1))
        printf 'lipo-bundles: arm64 only, kept single-arch: %s\n' "$relative" >&2
        continue
    fi

    lipo -create "$file" "$other" -output "$file.universal"
    # Keep the mode: `lipo` writes a plain file, and an executable that comes
    # out non-executable is a bundle that will not launch.
    chmod --reference="$file" "$file.universal" 2>/dev/null \
        || chmod "$(stat -f '%Lp' "$file")" "$file.universal"
    mv "$file.universal" "$file"

    merged=$((merged + 1))
done < <(find "$output" -type f -print0)

# The same in the other direction. `ditto` seeded the output from the arm64
# bundle, so a file only the x86_64 one has is not in the output at all yet
# and has to be copied in — otherwise the x86_64 slice would be missing a
# library it names, which is the one way this can produce a bundle that
# launches on one architecture and not the other.
while IFS= read -r -d '' file; do
    relative="${file#"$x86_64_bundle"/}"
    [ -f "$output/$relative" ] && continue

    # The output's own ad-hoc signature was deleted above, so the x86_64
    # bundle still having one is not a divergence between the builds — it is
    # the thing that was just thrown away on purpose, and signing the merged
    # bundle writes a new one. Without this the script reports
    # `Contents/_CodeSignature/CodeResources is in the x86_64 bundle but not
    # the arm64 one`, which is true and means nothing.
    case "$relative" in
    Contents/_CodeSignature/*) continue ;;
    esac

    case "$(file -b "$file")" in
    *Mach-O*)
        mkdir -p "$(dirname "$output/$relative")"
        cp -p "$file" "$output/$relative"
        one_sided=$((one_sided + 1))
        printf 'lipo-bundles: x86_64 only, kept single-arch: %s\n' "$relative" >&2
        ;;
    *)
        # Not a binary, so there is no architecture to explain it away: the
        # two builds produced different files.
        die "$relative is in the x86_64 bundle but not the arm64 one, and is not a binary"
        ;;
    esac
done < <(find "$x86_64_bundle" -type f -print0)

printf 'lipo-bundles: merged %s binaries, carried %s other files, %s single-arch\n' \
    "$merged" "$copied" "$one_sided" >&2

# Worth stating in the log: this is the number that decides
# `LSMinimumSystemVersion`, and the whole reason the bundle can run on macOS
# 11 is that conda-forge builds against an old SDK whatever machine is used.
printf 'lipo-bundles: architectures in the executable: %s\n' \
    "$(lipo -archs "$output/Contents/MacOS/"* 2>/dev/null || echo unknown)" >&2

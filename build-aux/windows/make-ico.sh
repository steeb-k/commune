#!/usr/bin/env bash
# Render an SVG application icon into a Windows .ico.
#
#   build-aux/windows/make-ico.sh assets/appicon.svg _build/windows/commune.ico
#
# The result is a derived artifact and is not committed; `build.rs` embeds it
# into the executable, which is the only place Windows looks for the icon
# Explorer, the taskbar and the Alt-Tab switcher show. There is no metadata file
# on Windows to carry it instead, the way `Info.plist` does on macOS.
#
# The sizes are the ones Windows actually asks for. 16 and 32 are the shell's
# small and large icons, 48 is the "medium icons" view, 256 is what the modern
# taskbar and the Start menu scale from, and the rest fill in the views between
# so that Windows never has to resample a smaller one upwards.
set -euo pipefail

SVG=${1:-}
OUT=${2:-}

if [ -z "$SVG" ] || [ -z "$OUT" ]; then
    echo "usage: make-ico.sh <input.svg> <output.ico>" >&2
    exit 2
fi

[ -f "$SVG" ] || {
    echo "make-ico: no such file: $SVG" >&2
    exit 1
}

for tool in rsvg-convert icotool; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "make-ico: $tool is not installed (pacman -S mingw-w64-ucrt-x86_64-librsvg mingw-w64-ucrt-x86_64-icoutils)" >&2
        exit 1
    }
done

SIZES='16 20 24 32 40 48 64 128 256'

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pngs=''
for size in $SIZES; do
    rsvg-convert --width "$size" --height "$size" --keep-aspect-ratio \
        --output "$work/$size.png" "$SVG"
    pngs="$pngs $work/$size.png"
done

mkdir -p "$(dirname "$OUT")"
# shellcheck disable=SC2086 # The list is ours and has no spaces in it.
icotool --create --output "$OUT" $pngs

echo "make-ico: $OUT"

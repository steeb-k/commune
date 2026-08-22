#!/usr/bin/env bash
# Render one of the app icons into the .icns that Info.plist names.
#
#   build-aux/macos/make-icns.sh assets/appicon.svg _build/commune.icns
#
# The .icns is a derived artefact and is not committed. `bundle.sh` chooses the
# source: assets/macos-tahoe-flat.svg or assets/macos-legacy-bevel.svg for a
# stable build, assets/appicon-devel.svg for a development one. `.iconset/` is
# gitignored for the intermediate directory this leaves behind if it fails
# partway.
#
# Nothing here masks the artwork into a rounded rectangle. The macOS plates are
# already square and full-bleed, which is what the system wants to enclose, and
# the GNOME icon is drawn with a silhouette of its own that a squircle would
# crop. macOS is happy to show an icon of any shape.
set -euo pipefail

SRC="${1:-}"
OUT="${2:-}"

if [ -z "$SRC" ] || [ -z "$OUT" ]; then
    echo "usage: make-icns.sh <icon.svg> <out.icns>" >&2
    exit 2
fi

[ -f "$SRC" ] || {
    echo "make-icns: no such file: $SRC" >&2
    exit 1
}

for tool in rsvg-convert iconutil; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "make-icns: $tool not found on PATH." >&2
        echo "  rsvg-convert comes with librsvg, iconutil with the Xcode command line tools." >&2
        exit 1
    }
done

ICONSET="${OUT%.icns}.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"

# iconutil wants exactly these names, and refuses the set if one it expects for
# a size it finds is absent. The @2x of one size and the 1x of the next are the
# same pixel dimensions but must both be present under both names.
for size in 16 32 128 256 512; do
    rsvg-convert -w "$size" -h "$size" -o "$ICONSET/icon_${size}x${size}.png" "$SRC"
    rsvg-convert -w "$((size * 2))" -h "$((size * 2))" \
        -o "$ICONSET/icon_${size}x${size}@2x.png" "$SRC"
done

mkdir -p "$(dirname "$OUT")"
iconutil --convert icns --output "$OUT" "$ICONSET"
rm -rf "$ICONSET"

echo "make-icns: wrote $OUT"

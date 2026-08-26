#!/bin/sh
# Give the notification small icon a drawable that is not inset for the launcher.
#
#     sh build-aux/android/patch-notification-icon.sh [res/drawable directory]
#
# Run this after `pixiewood generate` and before `pixiewood build`, next to the
# other patch scripts. It writes into `.pixiewood/`, which `generate` rewrites
# from its own templates each time, so there is nothing to keep in the tree.
#
# # What is wrong
#
# `android_notifications.rs` used to name `ic_launcher_monochrome` as the small
# icon, because it is the one monochrome drawable pixiewood generates and a
# silhouette is the right *shape* for a small icon. It is the wrong *size*.
#
# That drawable exists to be the monochrome layer of the adaptive launcher
# icon, so pixiewood wraps the artwork in the adaptive-icon inset named by
# `<drawable target="monochrome" scale=".45">` in the pixiewood manifest:
#
#     <group android:scaleX="0.45" android:scaleY="0.45"
#            android:translateX="6.6" android:translateY="6.6">
#
# For the launcher that is correct -- 45% of a 108dp canvas leaves the glyph
# inside the 66dp safe zone every mask shape is guaranteed to show. A
# notification small icon is drawn *inside* the badge the shade already puts
# around it, and is expected to fill it: the platform asks for a 24dp drawable
# carrying about 22dp of content. Reusing the launcher's layer inherits an
# inset it should never have had, and the glyph lands at roughly 40% of the
# slot -- about half the diameter of every other notification's icon in the
# same shade, which is what this looked like on an x86_64 emulator, API 35.
#
# # The fix
#
# Strip the inset. The paths are already correct -- pixiewood converted them
# from `data/icons/io.github.steeb_k.Commune-symbolic.svg`, whose 24-unit
# viewBox is exactly the 24dp a small icon wants -- so the whole of the fix is
# to drop the `<group>` that scales them down, and write the result out under
# a name of its own.
#
# Deriving it from the generated drawable rather than converting the SVG a
# second time is deliberate: there is one copy of the path data, pixiewood
# still owns the SVG-to-VectorDrawable conversion, and editing the artwork
# updates both icons with no second step to forget.
set -e

DRAWABLE="${1:-.pixiewood/android/app/src/main/res/drawable}"
SRC="$DRAWABLE/ic_launcher_monochrome.xml"
DST="$DRAWABLE/ic_notification.xml"

if [ ! -f "$SRC" ]; then
    echo "patch-notification-icon: no $SRC" >&2
    echo "patch-notification-icon: run \`pixiewood generate\` first" >&2
    exit 1
fi

# One group, and it is the inset. More than one means pixiewood's output has
# changed shape and stripping them all would be a guess, not a fix.
groups=$(grep -c '<group' "$SRC" || true)
if [ "$groups" != "1" ]; then
    echo "patch-notification-icon: expected 1 <group> in $SRC, found $groups" >&2
    exit 1
fi

perl -0777 -pe 's{\s*<group\b[^>]*>}{}g; s{\s*</group>}{}g;' "$SRC" > "$DST.tmp"

# The paths have to have survived, or the notification silently gets a blank
# icon -- which the platform accepts and nobody notices until it is shipped.
paths=$(grep -c '<path' "$DST.tmp" || true)
if [ "$paths" = "0" ]; then
    echo "patch-notification-icon: no <path> survived; refusing to write $DST" >&2
    rm -f "$DST.tmp"
    exit 1
fi

mv "$DST.tmp" "$DST"
echo "patch-notification-icon: wrote $DST ($paths path(s), inset removed)"

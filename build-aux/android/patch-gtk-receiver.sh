#!/bin/sh
# Put Commune's UnifiedPush receiver where the Gradle project will compile it.
#
#     sh build-aux/android/patch-gtk-receiver.sh [glue java directory]
#
# Run this after `pixiewood generate` and before `pixiewood build`, next to
# `patch-manifest.sh` and `patch-gtk-service.sh`.
#
# The reasoning is `patch-gtk-service.sh`'s, unchanged: pixiewood symlinks
# exactly one directory into the Gradle project's Java sources — GTK's own glue
# — and offers an application no way to add a package of its own, so
# `PushReceiver.java` is copied in beside the glue and its package is a
# consequence of that layout. The copy is unconditional because this adds a
# file GTK does not have; if GTK ever ships a `PushReceiver` of its own this
# collides loudly, which is the right way round.
#
# # When to delete this
#
# When pixiewood grows a way to add application Java sources. Then this file
# moves to a directory of ours and the manifest names it there instead.
set -eu

GLUE=${1:-.pixiewood/android/app/src/main/java/org/gtk/android}
if [ ! -d "$GLUE" ]; then
    printf 'no glue java directory at %s -- run `pixiewood generate` first\n' "$GLUE" >&2
    exit 1
fi

for file in PushReceiver.java PushRaiseService.java; do
    cp "$(dirname "$0")/$file" "$GLUE/$file"

    if [ ! -f "$GLUE/$file" ]; then
        printf 'copying %s into %s did not take\n' "$file" "$GLUE" >&2
        exit 1
    fi
done

printf 'copied PushReceiver.java and PushRaiseService.java into %s\n' "$GLUE"

#!/bin/sh
# Put Commune's foreground service where the Gradle project will compile it.
#
#     sh build-aux/android/patch-gtk-service.sh [glue java directory]
#
# Run this after `pixiewood generate` and before `pixiewood build`, next to
# `patch-manifest.sh`, `patch-gtk-ime.sh` and `patch-gtk-intent.sh`.
#
# # Why a copy, and why into GTK's package
#
# pixiewood symlinks exactly one directory into the Gradle project's Java
# sources — `org/gtk/android` out of `java-sources`, which defaults to GTK's own
# glue (`pixiewood:715-717`) — and offers an application no way to add a package
# of its own. `java-sources` cannot be pointed at a directory of ours either,
# because only its `org/gtk/android` subdirectory is linked, so redirecting it
# would mean vendoring GTK's five glue classes and keeping them in step by hand.
#
# So `SyncService.java` is copied in beside the glue, in `org.gtk.android`. The
# package is a consequence of pixiewood's layout, not a claim about whose code
# this is; the file says so at the top.
#
# # Why it is copied rather than edited in place
#
# The other two patches change GTK's own files and have to detect a GTK that has
# moved on. This one adds a file GTK does not have, so there is nothing to
# detect: it is copied unconditionally, and the check afterwards is that the
# copy took. If GTK ever ships a `SyncService` of its own this will collide
# loudly, which is the right way round.
#
# # When to delete this
#
# When pixiewood grows a way to add application Java sources. Then this file
# moves to a directory of ours and the manifest names it there instead.
set -eu

GLUE=${1:-.pixiewood/android/app/src/main/java/org/gtk/android}
SOURCE=$(dirname "$0")/SyncService.java

if [ ! -d "$GLUE" ]; then
    printf 'no glue java directory at %s -- run `pixiewood generate` first\n' "$GLUE" >&2
    exit 1
fi

if [ ! -f "$SOURCE" ]; then
    printf 'no SyncService.java at %s\n' "$SOURCE" >&2
    exit 1
fi

# `$GLUE` is a symlink into `subprojects/gtk`, so this writes into the wrap's
# checkout. That is where the other two patches write too, and the wrap is not
# under version control, so a re-extracted GTK simply loses the file and gets it
# back the next time this runs.
cp "$SOURCE" "$GLUE/SyncService.java"

if [ ! -f "$GLUE/SyncService.java" ]; then
    printf 'SyncService.java did not land in %s\n' "$GLUE" >&2
    exit 1
fi

printf 'patched %s: SyncService.java installed\n' "$GLUE"

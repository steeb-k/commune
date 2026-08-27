#!/bin/sh
# Replace GTK's Android InputConnection with one backed by the document.
#
#     sh build-aux/android/patch-gtk-ime-mirror.sh [glue java dir]
#
# Run this after `pixiewood prepare`, next to the other `patch-gtk-*`
# scripts. It writes into `subprojects/gtk`, which a re-extracted wrap
# loses. The Gradle copy under `.pixiewood` is a hard link to the same
# inode, so writing through the file patches both.
#
# # What is wrong
#
# Upstream's `ImeConnection` extends `BaseInputConnection` over a scratch
# `Editable` that the glue clears after every commit, so every question a
# keyboard asks -- the text around the cursor, the caps mode, where the
# composing region is -- is answered from an empty document. Four separate
# defects were patched downstream one method at a time
# (`patch-gtk-ime-selection.sh`, `patch-gtk-ime-caps.sh`,
# `patch-gtk-ime-composing-region.sh`, all retired by this script) before
# the shape of the whole showed: the connection was backed by the wrong
# document, and every keyboard behaviour not yet met would keep punching
# through it.
#
# # The fix
#
# `build-aux/android/ImContext.java` -- installed wholesale over the
# upstream file -- backs the connection with a real Editable that mirrors
# the text GTK reports around the cursor. `BaseInputConnection`'s stock
# method implementations, the same ones every native Android text field
# runs on, edit that mirror; each finished batch is translated into GTK's
# three verbs (deleteSurrounding, commit, updatePreedit) inside a single
# GLib-thread closure; and the same closure re-reads the surrounding text
# afterwards, from which the mirror is rebuilt. Drift between the mirror
# and the widget can survive at most one transaction, and GtkTextView's
# sliding surrounding-text window is harmless because offsets are never
# carried across a rebuild.
#
# `ToplevelActivity.onCreateInputConnection` is also retargeted to seed
# `EditorInfo`'s initial cursor from the new connection's mirror, so the
# keyboard's first idea of the document is the same document it will be
# talking to.
#
# The mirror expects `onGtkCursorMoved` to be driven from the C side --
# that is `patch-gtk-ime-cursor-notify.sh`, which must also be applied.
#
# # When to delete this
#
# When GTK's Android glue backs its InputConnection with the document
# upstream. The sanity checks below fail loudly if the JNI surface this
# replacement implements stops matching what gtkimcontextandroid.c
# registers, so a GTK update that reworks the glue is noticed here rather
# than papered over.
set -eu

GLUE=${1:-subprojects/gtk/gdk/android/glue/java/org/gtk/android}
IM=$GLUE/ImContext.java
TOPLEVEL=$GLUE/ToplevelActivity.java
OURS=$(dirname "$0")/ImContext.java
IMCONTEXT_C=subprojects/gtk/gtk/gtkimcontextandroid.c

for f in "$IM" "$TOPLEVEL"; do
    if [ ! -f "$f" ]; then
        printf 'no %s -- run `pixiewood prepare` first\n' "$f" >&2
        exit 1
    fi
done
if [ ! -f "$OURS" ]; then
    printf 'replacement %s is missing\n' "$OURS" >&2
    exit 1
fi

# The replacement declares exactly the native surface the C side binds.
# If upstream reworks it, fail here instead of shipping a glue that dies
# in RegisterNatives at startup.
if [ -f "$IMCONTEXT_C" ]; then
    for native in getInputType getSurrounding deleteSurrounding getPreedit updatePreedit commit; do
        if ! grep -q "\"$native\"" "$IMCONTEXT_C"; then
            printf '%s no longer registers `%s` -- the mirror replacement is out of date\n' \
                "$IMCONTEXT_C" "$native" >&2
            exit 1
        fi
    done
    # This one is ours: the replacement calls it from every transaction.
    if ! grep -q '"moveCursor"' "$IMCONTEXT_C"; then
        printf '%s does not register `moveCursor` -- run patch-gtk-ime-move-cursor.sh first\n' \
            "$IMCONTEXT_C" >&2
        exit 1
    fi
fi

# Write through the file: the Gradle tree hard-links this inode, and a
# copy that replaced it would leave the stale one to be compiled.
cat "$OURS" > "$IM"

if ! grep -q "onGtkCursorMoved" "$IM"; then
    printf 'writing the mirror ImContext.java into %s did not take\n' "$IM" >&2
    exit 1
fi

# ToplevelActivity: seed EditorInfo from the connection's own mirror.
if grep -q "connection.seed(outAttrs);" "$TOPLEVEL"; then
    printf 'onCreateInputConnection already seeds from the mirror in %s\n' "$TOPLEVEL"
else
    perl -0777 -pi -e '
        s/\t\t\t\treturn activeImContext\.new ImeConnection\(this\);/\t\t\t\tImContext.ImeConnection connection = activeImContext.new ImeConnection(this);\n\t\t\t\tconnection.seed(outAttrs);\n\t\t\t\treturn connection;/
            or die "onCreateInputConnection anchor not found -- has the glue changed?\n";
    ' "$TOPLEVEL"
    if ! grep -q "connection.seed(outAttrs);" "$TOPLEVEL"; then
        printf 'patching onCreateInputConnection in %s did not take\n' "$TOPLEVEL" >&2
        exit 1
    fi
fi

printf 'patched %s and %s: the InputConnection is backed by the document\n' "$IM" "$TOPLEVEL"

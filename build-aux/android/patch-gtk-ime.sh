#!/bin/sh
# Make GTK's Android glue tell the IME what kind of field has focus.
#
#     sh build-aux/android/patch-gtk-ime.sh [ToplevelActivity.java path]
#
# Run this after `pixiewood generate` and before `pixiewood build`, next to
# `patch-manifest.sh`. pixiewood copies the glue's Java into the Gradle project
# on every `generate`, so this cannot be applied once and forgotten.
#
# # What is wrong
#
# `ToplevelActivity.onCreateInputConnection` hardcodes
#
#     //outAttrs.inputType = GlibContext.blockForMain(() -> activeImContext.getInputType());
#     outAttrs.inputType = InputType.TYPE_NULL;
#
# with the real implementation commented out immediately above it. `TYPE_NULL`
# tells the IME that the field does not accept composed text and that it should
# send hard key events instead. Gboard honours that by drawing **no keyboard at
# all**: `EditorInfo` arrives as `inputType=0, inputTypeString=NULL`, the IME
# reports itself shown, and the screen stays empty. gtk4-demo does the same
# thing, so this is GTK-wide rather than anything about Commune.
#
# # Why un-commenting is enough
#
# The machinery behind that line is complete. `ImContext.java` declares
# `public native int getInputType()`, `gtk/gtkimcontextandroid.c` implements it
# as `_gtk_im_context_android_get_input_type` — mapping every `GtkInputPurpose`
# onto the matching Android `InputType` constants, including password and PIN —
# and registers it in `im_context_natives[]`. Only the call site is disabled.
#
# With the line restored, `EditorInfo` becomes `inputType=1` and Gboard attaches
# with autocorrect and learning enabled.
#
# # When to delete this
#
# When GTK restores the line upstream. The script fails rather than silently
# doing nothing if the text it expects is gone, so that a GTK update which fixes
# or reworks this is noticed here instead of being papered over.
set -eu

JAVA=${1:-.pixiewood/android/app/src/main/java/org/gtk/android/ToplevelActivity.java}

if [ ! -f "$JAVA" ]; then
    printf 'no ToplevelActivity.java at %s -- run `pixiewood generate` first\n' "$JAVA" >&2
    exit 1
fi

WANTED='outAttrs.inputType = GlibContext.blockForMain(() -> activeImContext.getInputType());'
BROKEN='outAttrs.inputType = InputType.TYPE_NULL;'

if grep -qF "$WANTED" "$JAVA" && ! grep -qF "$BROKEN" "$JAVA"; then
    printf 'already patched: %s\n' "$JAVA"
    exit 0
fi

if ! grep -qF "$BROKEN" "$JAVA"; then
    printf 'expected to find `%s` in %s and did not.\n' "$BROKEN" "$JAVA" >&2
    printf 'GTK has probably changed this code -- check whether the patch is still needed.\n' >&2
    exit 1
fi

# Replace the assignment, keeping whatever indentation the line already has.
# `InputType` stays imported and used nowhere, which javac does not mind.
perl -pi -e 's/^(\s*)outAttrs\.inputType = InputType\.TYPE_NULL;/$1outAttrs.inputType = GlibContext.blockForMain(() -> activeImContext.getInputType());/' "$JAVA"

if ! grep -qF "$WANTED" "$JAVA"; then
    printf 'patch did not apply to %s\n' "$JAVA" >&2
    exit 1
fi

printf 'patched %s: inputType comes from the focused GtkIMContext\n' "$JAVA"

#!/bin/sh
# Make GTK's Android IM context read the input purpose that was actually set.
#
#     sh build-aux/android/patch-gtk-input-purpose.sh [gtkimcontextandroid.c path]
#
# Run this after `pixiewood generate` and before `pixiewood build`, next to
# `patch-gtk-ime.sh`. It writes into `subprojects/gtk`, which a re-extracted
# wrap loses.
#
# # What is wrong
#
# `GtkIMContextAndroid` has `input_purpose` and `input_hints` struct members.
# `gtk_im_context_android_init` sets them to `GTK_INPUT_PURPOSE_FREE_FORM` and
# `GTK_INPUT_HINT_NONE`, and **nothing ever assigns them again**. There is no
# `set_property`, no `get_property` and no `g_object_class_override_property` in
# the file at all, where the Wayland and Windows backends have them.
#
# Meanwhile the values are not lost -- they are simply somewhere else.
# `GtkIMContext` installs `input-purpose` and `input-hints` itself, and its
# `set_property` stores them in its own private struct. So a widget setting the
# purpose works exactly as documented, and the Android backend then consults two
# fields nobody has written since they were initialised.
#
# The result is that every text field in every GTK application on Android is
# reported to the IME as free-form prose. Three symptoms of the one bug, all
# observed on a Pixel 9a:
#
#   * **Passwords render in plain text.** Android never hears
#     `TYPE_TEXT_VARIATION_PASSWORD`, so the keyboard leaves suggestions and
#     composing on. Uncommitted composing text is drawn unmasked; forcing a
#     commit (selecting the text, toggling visibility) masks it, which is what
#     made this look intermittent rather than constant.
#   * **A URL field gets a prose keyboard** -- no `/`, no `.`, and
#     autocapitalised. Setting `input-purpose: url` on the widget changes
#     nothing, because the property arrives and is then ignored.
#   * **Buttons driven by `changed` do not update until focus leaves the
#     field**, because a composing IME holds the text in preedit rather than
#     committing per keystroke. This one is inference rather than measurement:
#     the correct input types suppress composing, so it should follow.
#
# # The fix
#
# One line. The switch statement below it is complete and correct -- it already
# maps every `GtkInputPurpose` including password and PIN -- so rather than
# rewrite the function to use locals, this fills the two dead fields from the
# live properties immediately before they are read. Everything downstream is
# untouched.
#
# This is the second unconnected implementation found in this subsystem, after
# `patch-gtk-ime.sh` (the call site commented out with the working code on the
# line above). Worth remembering when judging what is missing here versus what
# is merely unwired.
#
# # When to delete this
#
# When GTK wires the properties upstream, by whatever means -- overriding the
# properties, or reading them here as this does. The script fails rather than
# silently doing nothing if the code it expects is gone, so a GTK update that
# fixes or reworks this is noticed rather than papered over.
set -eu

SOURCE=${1:-subprojects/gtk/gtk/gtkimcontextandroid.c}

if [ ! -f "$SOURCE" ]; then
    printf 'no gtkimcontextandroid.c at %s -- run `pixiewood prepare` first\n' "$SOURCE" >&2
    exit 1
fi

MARKER='g_object_get (self, "input-purpose"'
ANCHOR='  jint input_type = 0;'

if grep -qF "$MARKER" "$SOURCE"; then
    printf 'already patched: %s\n' "$SOURCE"
    exit 0
fi

if [ "$(grep -cF "$ANCHOR" "$SOURCE")" != "1" ]; then
    printf 'expected exactly one `%s` in %s and did not find it.\n' "$ANCHOR" "$SOURCE" >&2
    printf 'GTK has probably changed this code -- check whether the patch is still needed.\n' >&2
    exit 1
fi

# The fields are read here and in no other function, and written only by
# `_init`, so filling them at the top of this one is the whole change.
perl -0pi -e 's/(\n  jint input_type = 0;\n)/$1\n  \/* These two fields are never assigned after `_init`; the properties they\n   * mirror belong to GtkIMContext, which stores them privately. Read them\n   * from where they actually are, or every field on Android is free-form\n   * prose -- passwords included. See build-aux\/android\/patch-gtk-input-purpose.sh.\n   *\/\n  g_object_get (self, "input-purpose", &self->input_purpose,\n                      "input-hints", &self->input_hints, NULL);\n/' "$SOURCE"

if ! grep -qF "$MARKER" "$SOURCE"; then
    printf 'patch did not apply to %s\n' "$SOURCE" >&2
    exit 1
fi

printf 'patched %s: the IME is told what kind of field actually has focus\n' "$SOURCE"

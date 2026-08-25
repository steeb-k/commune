#!/bin/sh
# Stop restarting the input method every time the cursor moves.
#
#     sh build-aux/android/patch-gtk-ime-reset.sh [gtkimcontextandroid.c path]
#
# Run this after `pixiewood prepare` and before `pixiewood build`, next to
# `patch-gtk-input-purpose.sh`. It writes into `subprojects/gtk`, which a
# re-extracted wrap loses.
#
# # What is wrong
#
# This is the root cause of the space-bar cursor slide moving "a letter or two
# and stopping", and it is not in the Java glue where the search started.
#
# `gtk_text_move_cursor` ends, unconditionally, with
#
#     priv->need_im_reset = TRUE;
#     gtk_text_reset_im_context (self);
#
# so **every arrow key** runs `gtk_im_context_reset`. On Wayland or X11 that is
# cheap: it discards preedit state and returns. On Android
# `gtk_im_context_android_reset` calls the Java `ImContext.reset`, which is
#
#     imm.restartInput(view);
#
# and `restartInput` is not cheap. It tears the `InputConnection` down and calls
# `onCreateInputConnection` for a fresh one. Any gesture the keyboard has in
# flight is aborted along with it.
#
# So the sequence during a space-bar slide is: Gboard asks for one cursor step,
# the widget moves the cursor, GTK resets the IM context, Android restarts
# input, Gboard's connection is replaced underneath the gesture, and the slide
# ends after exactly one character. Measured on the emulator: one
# `setSelection(10, 10)` for an entry holding "hello world", then a burst of
# `finishComposingText()` as Gboard reacts to the restart, and nothing more.
#
# The same teardown fires on every plain cursor move, so it also costs a full
# IME round trip per arrow key, per tap into a field, and per selection change.
#
# # The fix
#
# Restart input only when there was actually something to discard. The preedit
# is the only IME state this context holds; when it is empty, `reset` has
# nothing to tell Android about and the connection can stay up.
#
# The focus and field-change cases do not depend on this call:
# `ToplevelActivity.setActiveImContext` already calls `restartInput` when the
# focused context changes, which is what makes a newly focused entry re-read its
# input type.
#
# # When to delete this
#
# When GTK stops resetting the IM context for plain cursor movement, or when the
# Android backend stops mapping `reset` onto `restartInput`. The script fails
# rather than silently doing nothing if the code it expects is gone.
set -eu

SOURCE=${1:-subprojects/gtk/gtk/gtkimcontextandroid.c}

if [ ! -f "$SOURCE" ]; then
    printf 'no gtkimcontextandroid.c at %s -- run `pixiewood prepare` first\n' "$SOURCE" >&2
    exit 1
fi

MARKER='had_preedit'
ANCHOR='  if (surface && surface->surface)'

if grep -qF "$MARKER" "$SOURCE"; then
    printf 'already patched: %s\n' "$SOURCE"
    exit 0
fi

if [ "$(grep -cF "$ANCHOR" "$SOURCE")" != "1" ]; then
    printf 'expected exactly one `%s` in %s and did not find it.\n' "$ANCHOR" "$SOURCE" >&2
    printf 'GTK has probably changed this code -- check whether the patch is still needed.\n' >&2
    exit 1
fi

# `if (self->preedit)` occurs five times in this file, so anchor the declaration
# to the one inside `reset` by starting the match at the function signature and
# taking the first occurrence after it.
perl -0pi -e 's/(gtk_im_context_android_reset \(GtkIMContext \*context\)\n(?:.*?\n)*?)(  if \(self->preedit\)\n)/$1  \/* Whether this reset has anything to tell Android about. See\n   * build-aux\/android\/patch-gtk-ime-reset.sh.\n   *\/\n  gboolean had_preedit = self->preedit != NULL;\n\n$2/' "$SOURCE"

perl -0pi -e 's/\n  if \(surface && surface->surface\)\n/\n  \/* `ImContext.reset` is `InputMethodManager.restartInput`, which destroys the\n   * InputConnection and builds a new one, cancelling whatever gesture the\n   * keyboard had in flight. `gtk_text_move_cursor` resets the IM context on\n   * every arrow key, so doing this unconditionally restarts input on every\n   * cursor movement -- which is what stopped the space-bar slide after one\n   * character.\n   *\/\n  if (had_preedit && surface && surface->surface)\n/' "$SOURCE"

# Check the function body rather than the file: the declaration landing in some
# other function would still satisfy a whole-file grep, and did once.
BODY=$(awk '/^gtk_im_context_android_reset \(GtkIMContext \*context\)$/,/^\}$/' "$SOURCE")

for want in 'gboolean had_preedit = self->preedit != NULL;' \
            'if (had_preedit && surface && surface->surface)'; do
    if ! printf '%s\n' "$BODY" | grep -qF "$want"; then
        printf 'patch did not apply: `%s` missing from gtk_im_context_android_reset\n' "$want" >&2
        exit 1
    fi
done

printf 'patched %s: the input method is restarted only when there was a preedit\n' "$SOURCE"

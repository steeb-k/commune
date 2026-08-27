#!/bin/sh
# Tell the keyboard when GTK moves the cursor.
#
#     sh build-aux/android/patch-gtk-ime-cursor-notify.sh [gtkimcontextandroid.c path]
#
# Run this after `pixiewood prepare`, next to the other `patch-gtk-*`
# scripts, and together with `patch-gtk-ime-mirror.sh`, whose
# `onGtkCursorMoved` this drives. It writes into `subprojects/gtk`, which
# a re-extracted wrap loses.
#
# # What is wrong
#
# Nothing in the glue tells Android when the cursor moves. A native
# editor calls `InputMethodManager.updateSelection` on every selection
# change; this glue never does, so after an arrow key, a tap into the
# text, or the application moving the caret, the keyboard's model of the
# document is whatever it last guessed. That stale model is what seeded
# `onCreateInputConnection`'s wrong initial cursor (Known gaps), and it
# is the ground every "resume this word" duplication grew from: the
# keyboard acts on offsets the widget left behind.
#
# # The fix
#
# `gtk_text_move_cursor` and its textview twin end every cursor movement
# with `gtk_im_context_reset` -- the same unconditional call that
# Defect 1 (`patch-gtk-ime-reset.sh`) had to stop translating into
# `restartInput`. That makes `reset` a reliable cursor-moved signal that
# already fires in exactly the right places. This patch has it call
# `ImContext.onGtkCursorMoved`, whose Java side re-reads the surrounding
# text on the GLib thread and hands the keyboard an `updateSelection` --
# the cheap notification, where `restartInput` was the ruinous one.
#
# # When to delete this
#
# When the glue notifies Android of selection changes itself.
set -eu

SOURCE=${1:-subprojects/gtk/gtk/gtkimcontextandroid.c}

if [ ! -f "$SOURCE" ]; then
    printf 'no gtkimcontextandroid.c at %s -- run `pixiewood prepare` first\n' "$SOURCE" >&2
    exit 1
fi

if grep -q "on_gtk_cursor_moved" "$SOURCE"; then
    printf 'cursor notify already patched in %s\n' "$SOURCE"
    exit 0
fi

perl -0777 -pi -e '
    # 1. The method id, cached beside reset in the struct.
    s/(  jmethodID reset;\n)/$1  jmethodID on_gtk_cursor_moved;\n/
        or die "java cache struct anchor not found -- has the glue changed?\n";

    # 2. Its lookup, after the reset lookup in gdk_im_context_init_jni.
    my $lookup = <<"EOF";
  gtk_im_context_android_java_cache.on_gtk_cursor_moved = (*env)->GetMethodID (env,
                                                                               gtk_im_context_android_java_cache.class,
                                                                               "onGtkCursorMoved",
                                                                               "()V");
EOF
    s{("reset",\n\s*"\(Landroid/view/View;\)V"\);\n)}{$1$lookup}
        or die "reset lookup anchor not found -- has the glue changed?\n";

    # 3. The call, at the end of gtk_im_context_android_reset. reset runs
    #    after every cursor movement, which is exactly the set of moments
    #    a native editor would call updateSelection for.
    my $call_anchor = quotemeta(q{  (*env)->PopLocalFrame (env, NULL);
  GTK_IM_CONTEXT_CLASS (gtk_im_context_android_parent_class)->reset (context);});
    my $call = <<"EOF";
  /* Every cursor movement funnels through reset; let the keyboard update
   * its model of the document. The Java side re-reads the surrounding
   * text and calls InputMethodManager.updateSelection -- the cheap
   * notification, where restartInput above is the ruinous one. */
  (*env)->CallVoidMethod (env, self->context,
                          gtk_im_context_android_java_cache.on_gtk_cursor_moved);
  (*env)->PopLocalFrame (env, NULL);
  GTK_IM_CONTEXT_CLASS (gtk_im_context_android_parent_class)->reset (context);
EOF
    chomp $call;
    s/$call_anchor/$call/
        or die "reset tail anchor not found -- has the glue changed?\n";
' "$SOURCE"

if ! grep -q "on_gtk_cursor_moved);" "$SOURCE"; then
    printf 'patching the cursor notify into %s did not take\n' "$SOURCE" >&2
    exit 1
fi

printf 'patched %s: reset tells the keyboard the cursor moved\n' "$SOURCE"

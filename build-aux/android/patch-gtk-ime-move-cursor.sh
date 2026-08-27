#!/bin/sh
# Give the keyboard a way to actually place the cursor.
#
#     sh build-aux/android/patch-gtk-ime-move-cursor.sh [gtkimcontextandroid.c path]
#
# Run this after `pixiewood prepare`, before `patch-gtk-ime-mirror.sh`,
# whose Java side declares and calls the native this registers. It writes
# into `subprojects/gtk`, which a re-extracted wrap loses.
#
# # What is wrong
#
# `GtkIMContext`'s protocol is commit, delete-surrounding and
# retrieve-surrounding; there is no way for an input method to say "put
# the cursor here". The first downstream fix spelled cursor moves in
# synthetic arrow keys, and it worked right up until a transaction needed
# to know the arrows had landed: key events drain from GDK's event queue
# in a different main-loop phase than an invoked closure, so the closure
# reads the world from before the arrows, whatever order the two were
# queued in. Measured as `setSelection`-adjacent edits refusing to apply.
#
# # The fix
#
# A `moveCursor(delta, anchor_delta)` native on ImContext: move the
# cursor `delta` codepoints from where it stands, leave the selection
# bound `anchor_delta` codepoints from where it lands (0 collapses). It
# goes through the widget -- `GtkEditable` for `GtkText`, the buffer for
# `GtkTextView` -- synchronously on the GLib thread, so a transaction can
# place the cursor, edit, and re-read in one closure with nothing in
# flight between the steps. Deltas rather than absolute offsets, because
# `GtkTextView`'s surrounding window makes absolute offsets a moving
# target and a delta from the current cursor is meaningful in both
# widgets.
#
# # When to delete this
#
# When `GtkIMContext` grows a cursor-position request upstream, which the
# upstream report (doc/upstream-gtk-android-ime.md) already argues for.
set -eu

SOURCE=${1:-subprojects/gtk/gtk/gtkimcontextandroid.c}

if [ ! -f "$SOURCE" ]; then
    printf 'no gtkimcontextandroid.c at %s -- run `pixiewood prepare` first\n' "$SOURCE" >&2
    exit 1
fi

if grep -q "move_cursor.*JNIEnv\|_gtk_im_context_android_move_cursor" "$SOURCE"; then
    printf 'moveCursor already patched in %s\n' "$SOURCE"
    exit 0
fi

perl -0777 -pi -e '
    # 1. The widget headers.
    s{(#include "gtk/gtkimcontextsimple\.h"\n)}{$1#include "gtk/gtkeditable.h"\n#include "gtk/gtktext.h"\n#include "gtk/gtktextview.h"\n}
        or die "include anchor not found -- has the glue changed?\n";

    # 2. The implementation, ahead of the natives table.
    my $impl = <<'"'"'EOF'"'"';
/* Move the cursor `delta` codepoints from where it stands and leave the
 * selection bound `anchor_delta` codepoints from where it lands (0
 * collapses the selection). GtkIMContext cannot express this, and
 * synthetic arrow keys cannot be ordered against a closure on the GLib
 * loop; the widget can, synchronously.
 */
static jboolean
_gtk_im_context_android_move_cursor (JNIEnv *env, jobject this, jint delta, jint anchor_delta)
{
  GTK_IM_CONTEXT_ANDROID_DECLARE_SELF_WITH_RET (this, FALSE)
  GtkWidget *widget = self->client_widget;
  if (!widget)
    return FALSE;
  if (GTK_IS_TEXT_VIEW (widget))
    {
      GtkTextBuffer *buffer = gtk_text_view_get_buffer ((GtkTextView *)widget);
      GtkTextIter cursor;
      gtk_text_buffer_get_iter_at_mark (buffer, &cursor, gtk_text_buffer_get_insert (buffer));
      gint pos = gtk_text_iter_get_offset (&cursor) + delta;
      if (pos < 0)
        pos = 0;
      gtk_text_buffer_get_iter_at_offset (buffer, &cursor, pos);
      if (anchor_delta != 0)
        {
          GtkTextIter bound;
          gint bound_pos = pos + anchor_delta;
          if (bound_pos < 0)
            bound_pos = 0;
          gtk_text_buffer_get_iter_at_offset (buffer, &bound, bound_pos);
          gtk_text_buffer_select_range (buffer, &cursor, &bound);
        }
      else
        gtk_text_buffer_place_cursor (buffer, &cursor);
      gtk_text_view_scroll_mark_onscreen ((GtkTextView *)widget, gtk_text_buffer_get_insert (buffer));
      return TRUE;
    }
  if (GTK_IS_EDITABLE (widget))
    {
      GtkEditable *editable = (GtkEditable *)widget;
      gint pos = gtk_editable_get_position (editable) + delta;
      if (pos < 0)
        pos = 0;
      if (anchor_delta != 0)
        {
          gint bound_pos = pos + anchor_delta;
          if (bound_pos < 0)
            bound_pos = 0;
          gtk_editable_select_region (editable, bound_pos, pos);
        }
      else
        gtk_editable_set_position (editable, pos);
      return TRUE;
    }
  return FALSE;
}

EOF
    s{(static const JNINativeMethod im_context_natives\[\] = \{)}{$impl$1}
        or die "natives table anchor not found -- has the glue changed?\n";

    # 3. Its registration, after commit.
    s{(\{ \.name = "commit", \.signature = "\(Ljava/lang/String;\)Z", \.fnPtr = _gtk_im_context_android_commit \})}{$1,\n  { .name = "moveCursor", .signature = "(II)Z", .fnPtr = _gtk_im_context_android_move_cursor }}
        or die "commit registration anchor not found -- has the glue changed?\n";
' "$SOURCE"

if ! grep -q '"moveCursor"' "$SOURCE"; then
    printf 'patching moveCursor into %s did not take\n' "$SOURCE" >&2
    exit 1
fi

printf 'patched %s: the keyboard can place the cursor\n' "$SOURCE"

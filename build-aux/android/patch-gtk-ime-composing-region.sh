#!/bin/sh
# Make setComposingRegion mean something, so a resumed word replaces itself
# instead of duplicating.
#
#     sh build-aux/android/patch-gtk-ime-composing-region.sh [ImContext.java path]
#
# Run this after `pixiewood prepare`, next to the other `patch-gtk-ime-*`
# scripts. It writes into `subprojects/gtk`, which a re-extracted wrap loses.
#
# # What is wrong
#
# `BaseInputConnection.setComposingRegion` marks the composing span on the
# connection's own `Editable` -- the scratch buffer this glue clears after
# every commit -- so the mark always lands on nothing. The keyboard's
# favourite moment for this call is "resuming" a word: refocus a field whose
# draft ends in `Testing`, press backspace, and Gboard sends
# `setComposingRegion` over the word followed by `setComposingText` carrying
# the word minus its last letter, expecting the marked region to be replaced.
# With no mark, the glue's `setComposingText` presents that text as a
# brand-new preedit at the cursor, and the screen shows the word twice over,
# the copy short one letter: the word duplicated, then backspaced from its
# copy. Measured on a Pixel 9a; a hardware keyboard never shows it,
# because hardware backspace is a key event and speaks none of this protocol.
#
# # The fix
#
# Do what the mark means. Take the committed characters of the region back
# out of the text -- cursor to the region's end by arrow keys, then
# backspaces, the way `deleteSurroundingText` and `setSelection` already
# spell editing that `GtkIMContext` has no verb for -- and re-present the
# same characters as the active preedit, scratch buffer and composing span
# included, so every later composing call operates on a preedit that really
# exists. An empty region means "stop composing", per the documentation.
#
# One knowing divergence: Android documents setComposingRegion as not moving
# the cursor. Here the cursor ends at its position within the re-presented
# preedit, which is where it already was in the only case a keyboard sends
# this -- the cursor inside or at the end of the word being resumed.
#
# # When to delete this
#
# When GTK's Android glue implements setComposingRegion itself.
set -eu

SOURCE=${1:-subprojects/gtk/gdk/android/glue/java/org/gtk/android/ImContext.java}

if [ ! -f "$SOURCE" ]; then
    printf 'no ImContext.java at %s -- run `pixiewood prepare` first\n' "$SOURCE" >&2
    exit 1
fi

if grep -q "setComposingRegion(int start, int end)" "$SOURCE"; then
    printf 'setComposingRegion already present in %s\n' "$SOURCE"
    exit 0
fi

# The method slots in after deleteSurroundingText, whose closing is the last
# thing before the inner class ends.
perl -0777 -pi -e '
    my $anchor = quotemeta("\t\t\treturn super.deleteSurroundingText(leftLength, rightLength);\n\t\t}\n");
    my $method = <<'"'"'EOF'"'"';

\t\t/* `BaseInputConnection` marks the composing region on its own
\t\t * Editable -- the scratch buffer this class clears after every
\t\t * commit -- so the mark always lands on nothing, and a keyboard
\t\t * that then "resumes" a word sends setComposingText expecting to
\t\t * replace the region and instead inserts a copy of it at the
\t\t * cursor. Do what the mark means: take the region back out of the
\t\t * committed text and re-present it as the active preedit.
\t\t */
\t\t\@Override
\t\tpublic boolean setComposingRegion(int start, int end) {
\t\t\tlogger.info("IME: setComposingRegion(" + start + ", " + end + ")");
\t\t\tif (end < start) {
\t\t\t\tint swap = start;
\t\t\t\tstart = end;
\t\t\t\tend = swap;
\t\t\t}

\t\t\tSnapshot s = snapshot();
\t\t\tif (s == null)
\t\t\t\treturn super.setComposingRegion(start, end);

\t\t\tint from = Math.max(0, Math.min(start, s.text.length()));
\t\t\tint to = Math.max(0, Math.min(end, s.text.length()));
\t\t\tif (from == to) {
\t\t\t\t/* An empty region means "stop composing". */
\t\t\t\treturn finishComposingText();
\t\t\t}

\t\t\tString region = s.text.substring(from, to);
\t\t\tint cursorInRegion = Math.max(0, Math.min(s.start, to) - from);

\t\t\t/* Cursor to the region'"'"'s end, region deleted, and only then
\t\t\t * the same characters handed back as preedit. The key events
\t\t\t * and the preedit update both funnel through the view'"'"'s
\t\t\t * thread, in this order. */
\t\t\tif (s.start < to)
\t\t\t\tmoveBy(graphemesBetween(s.text, s.start, to), 0);
\t\t\telse if (s.start > to)
\t\t\t\tmoveBy(-graphemesBetween(s.text, to, s.start), 0);
\t\t\tpress(KeyEvent.KEYCODE_DEL, graphemesBetween(s.text, from, to), 0);

\t\t\tEditable content = getEditable();
\t\t\tcontent.clear();
\t\t\tcontent.append(region);
\t\t\tSelection.setSelection(content, cursorInRegion);
\t\t\tsuper.setComposingRegion(0, content.length());

\t\t\tfinal String preedit = region;
\t\t\tfinal int preeditCursor = cursorInRegion;
\t\t\tGlibContext.blockForMain(() ->
\t\t\t\tImContext.this.updatePreedit(preedit, preeditCursor));
\t\t\treturn true;
\t\t}
EOF
    $method =~ s/\\t/\t/g;
    $method =~ s/\\\@/\@/g;
    s/($anchor)/$1$method/ or die "deleteSurroundingText anchor not found -- has the glue changed?\n";
' "$SOURCE"

if ! grep -q "setComposingRegion(int start, int end)" "$SOURCE"; then
    printf 'patching setComposingRegion into %s did not take\n' "$SOURCE" >&2
    exit 1
fi

printf 'patched %s: setComposingRegion converts the region into the preedit\n' "$SOURCE"

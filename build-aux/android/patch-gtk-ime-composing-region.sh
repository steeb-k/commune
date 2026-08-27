#!/bin/sh
# Remember what setComposingRegion marks, and spend the mark when the
# replacement text arrives -- so a resumed word replaces itself instead of
# duplicating, without ever rewriting the entry from the region call.
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
# With no mark, the glue presents that text as a brand-new preedit at the
# cursor, and the screen shows the word twice over, the copy short one
# letter. Measured on a Pixel 9a; a hardware keyboard never shows it,
# because hardware backspace is a key event and speaks none of this
# protocol.
#
# # The fix, and the fix that had to be taken back out first
#
# The first attempt rewrote the entry from inside setComposingRegion itself:
# synthetic arrow keys, synthetic backspaces, the region re-presented as
# preedit. Rewriting the text notifies the keyboard, the keyboard answers
# with another setComposingRegion, and the two chase each other -- measured
# on the same Pixel as a composer flashing several times a second, converging
# on the first letter, and eating real keystrokes. The lesson it left:
# setComposingRegion must not touch the screen.
#
# So this version touches nothing there. setComposingRegion only records
# what the keyboard marked; the mark is spent at the next setComposingText
# or commitText, which per the InputConnection contract is exactly the
# moment the marked region is replaced. Spending it means: take a fresh
# snapshot, require that it still shows the recorded characters with a
# collapsed cursor inside them, and delete the region through the native
# deleteSurrounding -- GTK's own verb for this, already bound in
# gtkimcontextandroid.c -- on the GLib main thread, strictly ordered before
# the preedit update that follows. A mark that fails verification is
# dropped unspent: the failure mode is the old, mild duplication, never a
# rewrite loop.
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

if grep -q "consumePendingRegion" "$SOURCE"; then
    printf 'setComposingRegion already patched in %s\n' "$SOURCE"
    exit 0
fi

perl -0777 -pi -e '
    # 1. The recorder and the spender, after deleteSurroundingText.
    my $anchor = quotemeta("\t\t\treturn super.deleteSurroundingText(leftLength, rightLength);\n\t\t}\n");
    my $method = <<'"'"'EOF'"'"';

\t\t/* A keyboard "resuming" a word -- refocus a drafted field, press
\t\t * backspace -- sends setComposingRegion over the word and then
\t\t * setComposingText or commitText carrying the replacement.
\t\t * `BaseInputConnection` marks the region on its own Editable, the
\t\t * scratch buffer this class clears after every commit, so the
\t\t * mark lands on nothing and the replacement is inserted at the
\t\t * cursor as a duplicate of the word.
\t\t *
\t\t * Editing the entry from here would be worse than the bug: a
\t\t * rewrite notifies the keyboard, the keyboard answers with
\t\t * another setComposingRegion, and the two chase each other. So
\t\t * this override touches nothing. It only records the mark, and
\t\t * the mark is spent by the call that replaces the region.
\t\t */
\t\tprivate String pendingRegionText;
\t\tprivate int pendingRegionStart;
\t\tprivate int pendingRegionEnd;

\t\t\@Override
\t\tpublic boolean setComposingRegion(int start, int end) {
\t\t\tlogger.info("IME: setComposingRegion(" + start + ", " + end + ")");
\t\t\tpendingRegionText = null;
\t\t\tif (end < start) {
\t\t\t\tint swap = start;
\t\t\t\tstart = end;
\t\t\t\tend = swap;
\t\t\t}

\t\t\tif (getEditable().length() > 0)
\t\t\t\t/* An active preedit already gives composing calls
\t\t\t\t * something real to edit. */
\t\t\t\treturn true;

\t\t\tSnapshot s = snapshot();
\t\t\tif (s == null)
\t\t\t\treturn true;

\t\t\tint from = Math.max(0, Math.min(start, s.text.length()));
\t\t\tint to = Math.max(0, Math.min(end, s.text.length()));
\t\t\tif (from == to || s.start != s.end || s.start < from || s.start > to)
\t\t\t\t/* An empty region, a selection, or a cursor outside
\t\t\t\t * the mark: not the resume shape. Remember nothing. */
\t\t\t\treturn true;

\t\t\tpendingRegionText = s.text.substring(from, to);
\t\t\tpendingRegionStart = from;
\t\t\tpendingRegionEnd = to;
\t\t\treturn true;
\t\t}

\t\t/* Spend a recorded composing region: the text about to arrive
\t\t * replaces it, so delete it first, through the native
\t\t * deleteSurrounding on the GLib thread -- strictly ordered
\t\t * before the preedit update that follows, where synthetic key
\t\t * events would detour through the UI thread and race it. A
\t\t * fresh snapshot must still show the recorded characters with
\t\t * a collapsed cursor inside them; anything else means the mark
\t\t * went stale, and a stale mark is dropped unspent.
\t\t */
\t\tprivate void consumePendingRegion() {
\t\t\tString region = pendingRegionText;
\t\t\tpendingRegionText = null;
\t\t\tif (region == null)
\t\t\t\treturn;

\t\t\tSnapshot s = snapshot();
\t\t\tif (s == null || s.start != s.end
\t\t\t    || s.start < pendingRegionStart || s.start > pendingRegionEnd
\t\t\t    || pendingRegionEnd > s.text.length()
\t\t\t    || !region.equals(s.text.substring(pendingRegionStart, pendingRegionEnd)))
\t\t\t\treturn;

\t\t\t/* deleteSurrounding counts characters, not UTF-16 units. */
\t\t\tfinal int before = s.text.codePointCount(pendingRegionStart, s.start);
\t\t\tfinal int total = s.text.codePointCount(pendingRegionStart, pendingRegionEnd);
\t\t\tGlibContext.blockForMain(() ->
\t\t\t\tImContext.this.deleteSurrounding(-before, total));
\t\t}
EOF
    $method =~ s/\\t/\t/g;
    $method =~ s/\\\@/\@/g;
    s/($anchor)/$1$method/ or die "deleteSurroundingText anchor not found -- has the glue changed?\n";

    # 2. The replacement calls spend the mark before they insert.
    s/(\t\t\tsuper\.setComposingText\(text, newCursorPosition\);)/\t\t\tconsumePendingRegion();\n$1/
        or die "setComposingText anchor not found -- has the glue changed?\n";
    s/(\t\t\tsuper\.commitText\(text, newCursorPosition\);)/\t\t\tconsumePendingRegion();\n$1/
        or die "commitText anchor not found -- has the glue changed?\n";

    # 3. finishComposingText removes the region, per the contract.
    s/(\t\t\tlogger\.info\("IME: finishComposingText\(\)"\);)/$1\n\t\t\tpendingRegionText = null;/
        or die "finishComposingText anchor not found -- has the glue changed?\n";
' "$SOURCE"

if ! grep -q "consumePendingRegion();" "$SOURCE"; then
    printf 'patching setComposingRegion into %s did not take\n' "$SOURCE" >&2
    exit 1
fi

printf 'patched %s: setComposingRegion records the mark, its replacement spends it\n' "$SOURCE"

#!/bin/sh
# Let the IME see the text around the cursor, and move the cursor.
#
#     sh build-aux/android/patch-gtk-ime-selection.sh [gradle java dir]
#
# Run this after `pixiewood generate` and before `pixiewood build`, next to
# `patch-gtk-ime.sh`. pixiewood copies the glue's Java into the Gradle project
# on every `generate`, so this cannot be applied once and forgotten.
#
# # What is wrong
#
# Hold space on Gboard and slide, and the cursor should follow your thumb. On
# GTK it moves a character or two and stops. The same gap breaks tap-to-position
# corrections, double-tap-to-select and anything else the keyboard does that is
# about *where* you are rather than *what* you typed.
#
# Gboard drives that gesture through `InputConnection`: it reads the text around
# the cursor, then calls `setSelection()` to put the cursor where your thumb is.
# `ImeConnection` overrides neither, so both fall through to
# `BaseInputConnection`, which answers them out of the `Editable` returned by
# `getEditable()`. That `Editable` is a scratch buffer for composing text, and
# `commitText`/`finishComposingText` call `content.clear()` on it after every
# commit. So the keyboard is told, truthfully as far as `BaseInputConnection`
# knows, that the document is empty and the cursor is at 0 -- and a cursor at 0
# in an empty document has nowhere to slide to.
#
# The real text is not far away: `ImContext` already declares
# `public native SurroundingRetVal getSurrounding()`, `gtkimcontextandroid.c`
# implements it over `gtk_im_context_get_surrounding_with_selection`, and it is
# registered in `im_context_natives[]`. Nothing calls it. This is the third
# complete-but-unconnected piece found in this subsystem, after `patch-gtk-ime.sh`
# (call site commented out) and `patch-gtk-input-purpose.sh` (fields never
# assigned).
#
# # The fix
#
# Answer `getTextBeforeCursor`, `getTextAfterCursor`, `getSelectedText` and
# `getSurroundingText` from `getSurrounding()` instead of from the scratch
# buffer, and implement `setSelection()`.
#
# `setSelection()` is the interesting one, because `GtkIMContext` has no way to
# express it. The protocol an input method gets is `commit`, `delete-surrounding`
# and `retrieve-surrounding`; there is no `set-cursor`. So the move is spelled
# out in arrow keys, which reach the widget through the same path that
# `deleteSurroundingText()` already uses to spell deletion out in backspaces.
#
# Two details that are easy to get wrong:
#
#   * **`getSurrounding()` counts codepoints** -- `_gtk_im_context_android_get_surrounding`
#     runs the byte offsets through `g_utf8_strlen` -- while every index Java
#     and the IME exchange is a UTF-16 offset. The two agree until someone types
#     an emoji, which is one codepoint and two `char`s. `toUtf16` converts.
#   * **GTK moves by grapheme cluster per keypress**, not by codepoint, so the
#     number of presses is counted with a `BreakIterator` rather than by
#     subtracting offsets. One press to cross a flag, not two.
#
# Finally, `imm.updateSelection()` is called after the move. Nothing in GTK
# calls it, so without it the keyboard's idea of the cursor is whatever it last
# guessed, and each slide starts over from that guess.
#
# # What this does not fix
#
# `gtk_text_view_retrieve_surrounding_handler` returns a *window* onto the
# buffer -- the cursor's line, widened to three word boundaries either side --
# and that window moves as the cursor moves. `gtk_text_retrieve_surrounding_cb`
# returns the whole entry. So offsets are stable and absolute in a `GtkEntry`
# and are relative to a sliding frame in a `GtkTextView`. The move is computed
# as a delta from the position read in the same call, which is correct in both
# cases; the absolute numbers handed to `updateSelection` are only exact for the
# entry. Expect the gesture to be crisp in single-line fields and to drift in a
# long multi-line composer.
#
# # When to delete this
#
# When GTK implements this upstream. The script fails rather than silently doing
# nothing if the code it expects is gone, so a GTK update that reworks the glue
# is noticed here instead of being papered over.
set -eu

JAVA_DIR=${1:-.pixiewood/android/app/src/main/java/org/gtk/android}
IM=$JAVA_DIR/ImContext.java
TOPLEVEL=$JAVA_DIR/ToplevelActivity.java

for f in "$IM" "$TOPLEVEL"; do
    if [ ! -f "$f" ]; then
        printf 'no %s -- run `pixiewood generate` first\n' "$f" >&2
        exit 1
    fi
done

if grep -qF 'static int toUtf16' "$IM"; then
    printf 'already patched: %s\n' "$IM"
    exit 0
fi

# Every anchor below must appear exactly once, or the file is not the one this
# patch was written against.
check_anchor() {
    if [ "$(grep -cF "$2" "$1")" != "1" ]; then
        printf 'expected exactly one `%s` in %s and did not find it.\n' "$2" "$1" >&2
        printf 'GTK has probably changed this code -- check whether the patch is still needed.\n' >&2
        exit 1
    fi
}

check_anchor "$IM" 'import android.text.Editable;'
check_anchor "$IM" 'import android.view.inputmethod.InputMethodManager;'
check_anchor "$IM" 'import java.util.logging.Logger;'
check_anchor "$IM" 'private long native_ptr;'
check_anchor "$IM" 'this.logger = Logger.getLogger("IME Connection");'
check_anchor "$TOPLEVEL" 'outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN;'

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The class-level helper, shared with ToplevelActivity.
cat > "$WORK/helper.java" <<'JAVA'
	/* `getSurrounding` counts codepoints -- the byte offsets it is given go
	 * through `g_utf8_strlen` -- but every index Java and the IME exchange is a
	 * UTF-16 offset. The two agree until someone types an emoji.
	 */
	static int toUtf16(String text, int codepoints) {
		if (text == null || codepoints <= 0)
			return 0;
		if (codepoints >= text.codePointCount(0, text.length()))
			return text.length();
		return text.offsetByCodePoints(0, codepoints);
	}

JAVA

# The InputConnection overrides.
cat > "$WORK/block.java" <<'JAVA'

		/* Everything below answers the IME's questions about the text around
		 * the cursor from the widget, rather than from the composing scratch
		 * buffer that `BaseInputConnection` would use and that this class
		 * clears after every commit. See
		 * build-aux/android/patch-gtk-ime-selection.sh.
		 */
		private final class Snapshot {
			final String text;
			final int start;
			final int end;

			Snapshot(String text, int cursor, int anchor) {
				this.text = text;
				this.start = Math.min(cursor, anchor);
				this.end = Math.max(cursor, anchor);
			}
		}

		private Snapshot snapshot() {
			ImContext.SurroundingRetVal s =
				GlibContext.blockForMain(() -> ImContext.this.getSurrounding());
			if (s == null || s.text == null)
				return null;
			return new Snapshot(s.text,
			                     ImContext.toUtf16(s.text, s.cursor_index),
			                     ImContext.toUtf16(s.text, s.anchor_index));
		}

		/* The lengths come from the keyboard and can be Integer.MAX_VALUE, so
		 * clamp by the room available rather than by adding and hoping:
		 * `end + length` overflows to a negative, `substring` throws, and an
		 * exception thrown across the binder reaches the keyboard as a plain
		 * null. That reads to it as an empty document, silently.
		 */
		private int behind(Snapshot s, int length) {
			return s.start - Math.min(Math.max(length, 0), s.start);
		}

		private int ahead(Snapshot s, int length) {
			return s.end + Math.min(Math.max(length, 0), s.text.length() - s.end);
		}

		@Override
		public CharSequence getTextBeforeCursor(int length, int flags) {
			Snapshot s = snapshot();
			if (s == null)
				return super.getTextBeforeCursor(length, flags);
			logger.info("IME: getTextBeforeCursor(" + length + ") of " + s.text.length());
			return s.text.substring(behind(s, length), s.start);
		}

		@Override
		public CharSequence getTextAfterCursor(int length, int flags) {
			Snapshot s = snapshot();
			if (s == null)
				return super.getTextAfterCursor(length, flags);
			return s.text.substring(s.end, ahead(s, length));
		}

		@Override
		public CharSequence getSelectedText(int flags) {
			Snapshot s = snapshot();
			if (s == null)
				return super.getSelectedText(flags);
			/* Android expects null, not "", when nothing is selected. */
			return s.start == s.end ? null : s.text.substring(s.start, s.end);
		}

		@Override
		public SurroundingText getSurroundingText(int beforeLength, int afterLength, int flags) {
			Snapshot s = snapshot();
			if (s == null)
				return super.getSurroundingText(beforeLength, afterLength, flags);
			int from = behind(s, beforeLength);
			int to = ahead(s, afterLength);
			logger.info("IME: getSurroundingText -> " + (to - from) + " chars");
			return new SurroundingText(s.text.substring(from, to),
			                           s.start - from, s.end - from, from);
		}

		/* `BaseInputConnection` returns null here, and a null reads to the
		 * keyboard as an empty document -- which is what kept Gboard's cursor
		 * model at 0 while the entry held twenty-one characters, so that its
		 * space-bar slide asked for setSelection(0, 0) and stopped.
		 *
		 * GET_EXTRACTED_TEXT_MONITOR is deliberately not honoured. Honouring it
		 * means calling `updateExtractedText` whenever the text changes, and
		 * GTK offers no signal here to do that from; an accurate answer every
		 * time the keyboard asks is the half that can be told the truth.
		 */
		@Override
		public ExtractedText getExtractedText(ExtractedTextRequest request, int flags) {
			Snapshot s = snapshot();
			if (s == null)
				return super.getExtractedText(request, flags);
			ExtractedText extracted = new ExtractedText();
			extracted.text = s.text;
			extracted.startOffset = 0;
			extracted.partialStartOffset = -1;
			extracted.partialEndOffset = -1;
			extracted.selectionStart = s.start;
			extracted.selectionEnd = s.end;
			extracted.flags = 0;
			logger.info("IME: getExtractedText -> " + s.text.length()
			            + " chars, selection " + s.start + ".." + s.end);
			return extracted;
		}

		/* GTK moves the cursor one grapheme cluster per arrow press, so count
		 * clusters rather than subtracting offsets: one press to cross a flag,
		 * not the two `char`s or four codepoints it is made of.
		 */
		private int graphemesBetween(String text, int from, int to) {
			BreakIterator it = BreakIterator.getCharacterInstance();
			it.setText(text);
			int n = 0;
			for (int at = from; at < to; n++) {
				int next = it.following(at);
				if (next == BreakIterator.DONE || next > to)
					break;
				at = next;
			}
			return n;
		}

		private void press(int keyCode, int times, int meta) {
			long now = SystemClock.uptimeMillis();
			for (int i = 0; i < times; i++) {
				sendKeyEvent(new KeyEvent(now, now, KeyEvent.ACTION_DOWN, keyCode, 0, meta));
				sendKeyEvent(new KeyEvent(now, now, KeyEvent.ACTION_UP, keyCode, 0, meta));
			}
		}

		private void moveBy(int graphemes, int meta) {
			if (graphemes < 0)
				press(KeyEvent.KEYCODE_DPAD_LEFT, -graphemes, meta);
			else if (graphemes > 0)
				press(KeyEvent.KEYCODE_DPAD_RIGHT, graphemes, meta);
		}

		/* `GtkIMContext` has no way to say "put the cursor here" -- the protocol
		 * is commit, delete-surrounding and retrieve-surrounding, and that is
		 * all -- so say it in arrow keys, the way `deleteSurroundingText` above
		 * says deletion in backspaces.
		 */
		@Override
		public boolean setSelection(int start, int end) {
			logger.info("IME: setSelection(" + start + ", " + end + ")");
			Snapshot s = snapshot();
			if (s == null)
				return super.setSelection(start, end);

			int from = Math.max(0, Math.min(start, s.text.length()));
			int to = Math.max(0, Math.min(end, s.text.length()));

			/* Unshifted first: that both moves the cursor and drops any
			 * existing selection, which is what setSelection asks for. */
			if (from >= s.start)
				moveBy(graphemesBetween(s.text, s.start, from), 0);
			else
				moveBy(-graphemesBetween(s.text, from, s.start), 0);

			if (to != from) {
				if (to > from)
					moveBy(graphemesBetween(s.text, from, to), KeyEvent.META_SHIFT_ON);
				else
					moveBy(-graphemesBetween(s.text, to, from), KeyEvent.META_SHIFT_ON);
			}

			/* Nothing in GTK calls updateSelection, so without this the
			 * keyboard's idea of the cursor stays whatever it last guessed and
			 * the next slide starts over from that guess.
			 *
			 * On the view's thread, not this one: InputConnection methods
			 * arrive on the keyboard's binder thread, and InputMethodManager
			 * expects to be driven from the thread that owns the view.
			 */
			target.post(() -> {
				InputMethodManager imm =
					target.getContext().getSystemService(InputMethodManager.class);
				if (imm != null)
					imm.updateSelection(target, from, to, -1, -1);
			});
			return true;
		}
JAVA

# The activity tells the IME where the cursor starts out.
cat > "$WORK/initial.java" <<'JAVA'
				/* Seed the keyboard's idea of the cursor. Left unset these are
				 * -1, "unknown", and Gboard will not offer gestures that need
				 * to know where the cursor is.
				 * See build-aux/android/patch-gtk-ime-selection.sh. */
				ImContext.SurroundingRetVal sel =
					GlibContext.blockForMain(() -> activeImContext.getSurrounding());
				if (sel != null && sel.text != null) {
					outAttrs.initialSelStart = ImContext.toUtf16(sel.text, sel.cursor_index);
					outAttrs.initialSelEnd = ImContext.toUtf16(sel.text, sel.anchor_index);
				}
JAVA

awk -v helper="$WORK/helper.java" -v block="$WORK/block.java" '
	function emit(file,   line) {
		while ((getline line < file) > 0)
			print line
		close(file)
	}
	/^import android\.text\.Editable;$/ { print "import android.os.SystemClock;"; }
	/^import android\.view\.inputmethod\.InputMethodManager;$/ {
		print "import android.view.inputmethod.ExtractedText;"
		print "import android.view.inputmethod.ExtractedTextRequest;"
		print
		print "import android.view.inputmethod.SurroundingText;"
		next
	}
	/^import java\.util\.logging\.Logger;$/ { print "import java.text.BreakIterator;"; }
	/^\tprivate long native_ptr;$/ { emit(helper) }
	{ print }
	/this\.logger = Logger\.getLogger\("IME Connection"\);$/ {
		print "\t\t\tthis.target = target;"
		in_ctor = 1
		next
	}
	in_ctor && /^\t\t\}$/ { emit(block); in_ctor = 0 }
' "$IM" > "$WORK/ImContext.java"

# `target` is needed for updateSelection; BaseInputConnection keeps its view to
# itself.
perl -0pi -e 's/(\n\t\tprivate Logger logger;\n)/$1\t\tprivate final android.view.View target;\n/' "$WORK/ImContext.java"

export INITIAL="$WORK/initial.java"
perl -0pi -e '
	BEGIN { local (@ARGV, $/) = ($ENV{INITIAL}); $add = <>; }
	s/(\n\t+)(outAttrs\.imeOptions = EditorInfo\.IME_FLAG_NO_FULLSCREEN;)/"\n" . $add . $1 . $2/e
' "$TOPLEVEL"

cp "$WORK/ImContext.java" "$IM"

for pair in "$IM:static int toUtf16" \
            "$IM:public boolean setSelection" \
            "$IM:public SurroundingText getSurroundingText" \
            "$IM:private final android.view.View target;" \
            "$IM:this.target = target;" \
            "$TOPLEVEL:outAttrs.initialSelStart"; do
    file=${pair%%:*}
    want=${pair#*:}
    if ! grep -qF "$want" "$file"; then
        printf 'patch did not apply: `%s` missing from %s\n' "$want" "$file" >&2
        exit 1
    fi
done

printf 'patched %s and %s: the IME can read the text and move the cursor\n' "$IM" "$TOPLEVEL"

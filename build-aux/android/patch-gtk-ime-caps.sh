#!/bin/sh
# Let the IME work out where a sentence starts.
#
#     sh build-aux/android/patch-gtk-ime-caps.sh [gradle java dir]
#
# Run this after `pixiewood generate` and after `patch-gtk-ime-selection.sh`,
# whose `snapshot()` this uses. pixiewood copies the glue's Java into the Gradle
# project on every `generate`, so this cannot be applied once and forgotten.
#
# # What is wrong
#
# A field that asks for `TYPE_TEXT_FLAG_CAP_SENTENCES` -- which is what
# `GTK_INPUT_HINT_UPPERCASE_SENTENCES` becomes, once the app sets it -- gets a
# keyboard that shifts the first letter of every sentence. Which letters those
# are is not the keyboard's decision alone: it asks the field, through
# `InputConnection.getCursorCapsMode()`, and shifts whatever the field says is
# sentence-initial.
#
# `ImeConnection` does not override it, so the answer comes from
# `BaseInputConnection`, which reads the `Editable` that `getEditable()` returns.
# That `Editable` is the composing scratch buffer, and `commitText` and
# `finishComposingText` call `content.clear()` on it after every commit. So the
# question is always asked of an empty document with the cursor at 0 -- the
# start of a sentence, truthfully, for that buffer -- and the keyboard shifts
# the first letter of every word it composes. Auto-capitalisation that fires
# everywhere is worse than none, which is why this ships with the hints rather
# than after them.
#
# This is the same root cause as `patch-gtk-ime-selection.sh`: the scratch
# buffer answering questions about a document it does not hold. That patch put
# the real text within reach; this one uses it for the one query it left.
#
# # The fix
#
# Override `getCursorCapsMode` and hand `TextUtils.getCapsMode` the text around
# the cursor from `snapshot()`, the same source the text queries already use.
# `TextUtils.getCapsMode` is the implementation `BaseInputConnection` itself
# calls, so this changes what it is asked about and nothing else.
#
# # What this does not fix
#
# `gtk_text_view_retrieve_surrounding_handler` hands back a window onto the
# buffer -- the cursor's line, widened to three word boundaries either side --
# rather than the whole thing. Caps mode only ever looks backwards from the
# cursor, so the window is enough whenever the sentence began inside it, which
# in a composer is a line and therefore nearly always. A sentence carried across
# a hard line break, or past the far edge of a long paragraph, looks like a
# fresh one and gets a capital.
#
# # When to delete this
#
# When GTK implements this upstream. The script fails rather than silently doing
# nothing if the code it expects is gone, so a GTK update that reworks the glue
# is noticed here instead of being papered over.
set -eu

JAVA_DIR=${1:-.pixiewood/android/app/src/main/java/org/gtk/android}
IM=$JAVA_DIR/ImContext.java

if [ ! -f "$IM" ]; then
    printf 'no %s -- run `pixiewood generate` first\n' "$IM" >&2
    exit 1
fi

if grep -qF 'public int getCursorCapsMode' "$IM"; then
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
# From patch-gtk-ime-selection.sh, which has to have run first: this patch is
# written against the text it put within reach.
check_anchor "$IM" 'private Snapshot snapshot() {'
check_anchor "$IM" 'public CharSequence getTextBeforeCursor(int length, int flags) {'

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cat > "$WORK/block.java" <<'JAVA'
		/* Which shift state this position calls for.
		 *
		 * `BaseInputConnection` answers this out of the composing scratch
		 * buffer, which this class clears after every commit -- so every answer
		 * describes the start of an empty document, every word looks like the
		 * first word of a sentence, and a field asking for CAP_SENTENCES gets
		 * all of them shifted. Answer from the text around the cursor, which is
		 * where the sentence actually is.
		 * See build-aux/android/patch-gtk-ime-caps.sh.
		 */
		@Override
		public int getCursorCapsMode(int reqModes) {
			Snapshot s = snapshot();
			if (s == null)
				return super.getCursorCapsMode(reqModes);
			return TextUtils.getCapsMode(s.text, s.start, reqModes);
		}

JAVA

# The import first; the block second, immediately before the `@Override` on
# `getTextBeforeCursor`, so it sits with the other queries answered from the
# widget rather than from the scratch buffer.
awk '
	/^import android\.text\.Editable;$/ {
		print
		print "import android.text.TextUtils;"
		next
	}
	{ print }
' "$IM" > "$WORK/ImContext.java"

export BLOCK="$WORK/block.java"
perl -0pi -e '
	BEGIN { local (@ARGV, $/) = ($ENV{BLOCK}); $add = <>; }
	s/(		\@Override
		public CharSequence getTextBeforeCursor)/$add . $1/e
' "$WORK/ImContext.java"

cp "$WORK/ImContext.java" "$IM"

for want in 'import android.text.TextUtils;' \
            'public int getCursorCapsMode(int reqModes)' \
            'TextUtils.getCapsMode(s.text, s.start, reqModes)'; do
    if ! grep -qF "$want" "$IM"; then
        printf 'patch did not apply: `%s` missing from %s\n' "$want" "$IM" >&2
        exit 1
    fi
done

printf 'patched %s: the IME can tell where a sentence starts\n' "$IM"

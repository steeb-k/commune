#!/bin/sh
# Make GTK's `UPPERCASE_SENTENCES` hint mean sentences, not words.
#
#     sh build-aux/android/patch-gtk-caps-sentences.sh [gtkimcontextandroid.c path]
#
# Run this after `pixiewood generate` and before `pixiewood build`, next to
# `patch-gtk-input-purpose.sh`. It writes into `subprojects/gtk`, which a
# re-extracted wrap loses.
#
# # What is wrong
#
# One line, in the JNI field cache that `gtk_im_context_android_init_java_cache`
# fills:
#
#     FILL_INPUT_TYPE (text_flag_cap_words, "TEXT_FLAG_CAP_WORDS")
#     FILL_INPUT_TYPE (text_flag_cap_sentences, "TEXT_FLAG_CAP_WORDS")
#
# The second line reads `TEXT_FLAG_CAP_WORDS` into the field named
# `text_flag_cap_sentences`. Every use of it is therefore wrong by one Android
# constant: `GTK_INPUT_HINT_UPPERCASE_SENTENCES` sends
# `TYPE_TEXT_FLAG_CAP_WORDS` (`0x2000`) where it means `CAP_SENTENCES`
# (`0x4000`), and the keyboard shifts the first letter of every word.
#
# `_gtk_im_context_android_get_input_type` is correct, and so is everything
# above and below the typo -- there is nothing else to fix.
#
# _Measured_ before the fix, on the emulator, with a composer asking for
# `spellcheck | uppercase-sentences`: `dumpsys input_method` reported
# `inputType=0xa001`, which is `CLASS_TEXT | CAP_WORDS | AUTO_CORRECT`. After
# it: `0xc001`, which is `CLASS_TEXT | CAP_SENTENCES | AUTO_CORRECT`.
#
# # When to delete this
#
# When GTK fixes the typo upstream. The script fails rather than silently doing
# nothing if the line it expects is gone, so a GTK update that touches this is
# noticed here instead of being papered over.
set -eu

SOURCE=${1:-subprojects/gtk/gtk/gtkimcontextandroid.c}

if [ ! -f "$SOURCE" ]; then
    printf 'no gtkimcontextandroid.c at %s -- run `pixiewood prepare` first\n' "$SOURCE" >&2
    exit 1
fi

WRONG='FILL_INPUT_TYPE (text_flag_cap_sentences, "TEXT_FLAG_CAP_WORDS")'
RIGHT='FILL_INPUT_TYPE (text_flag_cap_sentences, "TEXT_FLAG_CAP_SENTENCES")'

if grep -qF "$RIGHT" "$SOURCE"; then
    printf 'already patched: %s\n' "$SOURCE"
    exit 0
fi

if [ "$(grep -cF "$WRONG" "$SOURCE")" != "1" ]; then
    printf 'expected exactly one `%s` in %s and did not find it.\n' "$WRONG" "$SOURCE" >&2
    printf 'GTK has probably fixed this -- check whether the patch is still needed.\n' >&2
    exit 1
fi

perl -0pi -e 's/FILL_INPUT_TYPE \(text_flag_cap_sentences, "TEXT_FLAG_CAP_WORDS"\)/FILL_INPUT_TYPE (text_flag_cap_sentences, "TEXT_FLAG_CAP_SENTENCES")/' "$SOURCE"

if ! grep -qF "$RIGHT" "$SOURCE"; then
    printf 'patch did not apply to %s\n' "$SOURCE" >&2
    exit 1
fi

printf 'patched %s: UPPERCASE_SENTENCES now means CAP_SENTENCES\n' "$SOURCE"

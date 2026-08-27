# GTK on Android: the IME loses the cursor — notes for an upstream report

Working notes for a report against [GNOME/gtk](https://gitlab.gnome.org/GNOME/gtk/-/issues),
written while fixing this downstream in Commune. Four separate defects are described. **The first
is the real bug and is worth reporting on its own**; the second is a missing feature that is only
visible once the first is fixed; the third is a one-method omission with the same root cause as the
second, and is only visible once an application asks for auto-capitalisation; the fourth is a
one-line typo in a JNI field cache that produces the same symptom as the third by a different
route.

Everything marked _measured_ was measured. Everything marked _inference_ was not.

## Environment

| | |
| --- | --- |
| GTK | 4.23.3, `main` at `3e34ecbc89e5` (2026-08-24) |
| Build | [pixiewood](https://codeberg.org/sp1rit/gtk-android-builder), NDK 27.2.12479018, minSdk 31, targetSdk 36 |
| Devices | Pixel 9a, GrapheneOS / Android 17, arm64 · emulator `sdk_gphone64_x86_64`, API 35 |
| Keyboard | Gboard (`com.google.android.inputmethod.latin`) |
| Application | Commune, a GTK4/libadwaita Matrix client |

Reproduced on both devices. Not specific to this application: the affected code is in GTK.

---

## Defect 1 — `reset` restarts the input method on every cursor movement

### Symptom

Hold the space bar on Gboard and slide, the standard Android gesture for dragging the cursor
through text. The cursor moves one character and stops. The same gesture in any native Android text
field tracks the finger continuously.

### Cause

`gtk_text_move_cursor` ends, unconditionally, with

```c
/* gtk/gtktext.c:4146 */
  priv->need_im_reset = TRUE;
  gtk_text_reset_im_context (self);
```

and `gtk_text_view_move_cursor` ends the same way (`gtk/gtktextview.c:6741`). So every cursor
movement runs `gtk_im_context_reset`.

On Wayland and X11 that is cheap — it discards preedit state and returns. On Android,
`gtk_im_context_android_reset` (`gtk/gtkimcontextandroid.c:438`) additionally calls the Java
`ImContext.reset`:

```java
/* gdk/android/glue/java/org/gtk/android/ImContext.java:44 */
	private static void reset(View view) {
		InputMethodManager imm = view.getContext().getSystemService(InputMethodManager.class);
		imm.restartInput(view);
	}
```

[`restartInput`](https://developer.android.com/reference/android/view/inputmethod/InputMethodManager#restartInput(android.view.View))
destroys the current `InputConnection` and asks the view for a new one. Any interaction the IME has
in flight is abandoned with it.

The gesture therefore proceeds: Gboard asks for one cursor step → the widget moves the cursor → GTK
resets the IM context → Android restarts input → Gboard's connection is replaced underneath it →
the gesture ends after one character.

### Evidence

_Measured._ Logging every `InputConnection` call during one swipe, with the fix absent:

```text
IME: setSelection(10, 10)
IME: finishComposingText()          ← ×24, Gboard reacting to the restart
```

and `adb shell dumpsys input_method` immediately afterwards, for an entry holding `hello world`
whose cursor had moved from 11 to 10:

```text
initialSelStart=10 initialSelEnd=10
```

`initialSelStart` is only populated when an `InputConnection` is created. Reading 10 means the
connection was built _after_ the cursor moved — a new one, mid-gesture.

With the fix applied, the same swipe:

```text
IME: setSelection(10, 10)
IME: setSelection(9, 9)
IME: setSelection(8, 8)
IME: setSelection(7, 7)
IME: setSelection(6, 6)
```

one step per ~200 ms, cursor tracking the finger, and `initialSelStart=0` — the connection created
when the field was focused, still alive.

### Suggested fix

The preedit is the only IME state this context holds. When there is none, `reset` has nothing to
tell Android about and the connection can stay up.

```diff
--- a/gtk/gtkimcontextandroid.c
+++ b/gtk/gtkimcontextandroid.c
@@ gtk_im_context_android_reset (GtkIMContext *context)
   (*env)->PushLocalFrame (env, 1);
 
+  gboolean had_preedit = self->preedit != NULL;
+
   if (self->preedit)
     {
       (*env)->DeleteGlobalRef (env, self->preedit);
@@
-  if (surface && surface->surface)
+  /* restartInput() destroys the InputConnection and builds a new one, which
+   * cancels any IME interaction in flight. gtk_text_move_cursor() resets the
+   * IM context on every cursor movement, so doing this unconditionally
+   * restarts input every time the cursor moves. */
+  if (had_preedit && surface && surface->surface)
     {
       (*env)->CallStaticVoidMethod (env, gtk_im_context_android_java_cache.class,
                                     gtk_im_context_android_java_cache.reset,
                                     surface->surface);
     }
```

The focus and field-change paths do not depend on this call:
`ToplevelActivity.setActiveImContext` (`ToplevelActivity.java:198`) already calls `restartInput`
when the focused context changes, which is what makes a newly focused entry re-read its input type.

Carried downstream as
[`patch-gtk-ime-reset.sh`](../build-aux/android/patch-gtk-ime-reset.sh).

### Wider than the reported symptom

The teardown was being paid on **every** cursor movement — each arrow key, each tap into a field,
each selection change — not only during this gesture. Anything an IME does that spans more than one
call is exposed to it. The space-bar slide is simply the most visible instance.

_Inference, not measured:_ this may also explain IME behaviours on Android that have been put down
to Gboard, e.g. autocorrect and suggestion state being dropped after moving the caret.

### If the maintainers prefer a different shape

Two alternatives, noted because the guard above is the smallest change rather than necessarily the
right one:

* Make `reset` on this backend send `finishComposingText` to the IME instead of restarting input,
  and keep `restartInput` for genuine editor changes.
* Have `gtk_text_move_cursor` reset the IM context only when a preedit exists, which would fix it
  for every backend at once — but that is a change to shared code with much wider blast radius, and
  the Android mapping would still be lossy.

---

## Defect 2 — `ImeConnection` answers no text queries and does not implement `setSelection`

Only observable once Defect 1 is fixed; before that the gesture dies too early to matter.

### Cause

`ImeConnection extends BaseInputConnection`
(`gdk/android/glue/java/org/gtk/android/ImContext.java`) overrides exactly four methods —
`setComposingText`, `finishComposingText`, `commitText`, `deleteSurroundingText`. Everything else
falls through to `BaseInputConnection`, which answers out of the `Editable` returned by
`getEditable()`. That `Editable` is this class's composing scratch buffer, and both `commitText`
and `finishComposingText` call `clear()` on it.

So `getTextBeforeCursor`, `getTextAfterCursor`, `getSelectedText`, `getSurroundingText` and
`getExtractedText` all report, truthfully as far as `BaseInputConnection` knows, an empty document
with the cursor at 0.

_Measured:_ Gboard's suggestion strip in a GTK entry containing `hello wonderful world` offers
`what` / `I` / `I'm` — sentence openers. The same string in the stock Settings search box offers
`world` / `worlds`.

The text is one unused call away. `ImContext` already declares
`public native SurroundingRetVal getSurrounding()`, `gtkimcontextandroid.c` already implements it
over `gtk_im_context_get_surrounding_with_selection`, and it is already registered in
`im_context_natives[]`. Nothing calls it.

### The awkward part: `GtkIMContext` cannot express `setSelection`

The protocol an input method gets is `commit`, `delete-surrounding` and `retrieve-surrounding`.
There is no way to say _put the cursor here_.

Downstream this is worked around by translating `setSelection` into arrow-key events — the same
approach `deleteSurroundingText` already takes for deletion, and it works
(`AKEYCODE_DPAD_LEFT` → `GDK_KEY_Left`, `gdkandroidkeysyms-private.h:61`). It is a workaround, and
**a proper fix probably wants a cursor-position request on `GtkIMContext` itself**. That is a
public-API question for the maintainers, which is why this half is offered as a report rather than
as a patch.

Carried downstream as
[`patch-gtk-ime-selection.sh`](../build-aux/android/patch-gtk-ime-selection.sh).

### Three traps for whoever implements this

_All measured while getting it wrong first._

1. **`getSurrounding` counts codepoints, the IME counts UTF-16 units.**
   `_gtk_im_context_android_get_surrounding` runs its byte offsets through `g_utf8_strlen`, but
   every index Java and the `InputConnection` exchange is a UTF-16 offset. The two agree until
   someone types an emoji. Convert with `offsetByCodePoints`.
2. **GTK moves by grapheme cluster per keypress, not by codepoint.** Counting arrow presses by
   subtracting offsets miscounts flags, ZWJ sequences and combining marks. A `BreakIterator` gets
   it right.
3. **The surrounding text is not the same shape for the two widgets.**
   `gtk_text_retrieve_surrounding_cb` returns the entire entry, so offsets are absolute and stable.
   `gtk_text_view_retrieve_surrounding_handler` returns a _window_ — the cursor's line, widened to
   three word boundaries either side — which moves as the cursor moves. Computing every movement as
   a delta from the position read in the same call absorbs the difference; treating the offsets as
   absolute does not. Verified in a real multi-line composer on hardware.

Also worth knowing: `gtk_text_get_display_text` returns the invisible character for a non-visible
entry, so a password is **not** handed to the keyboard in clear. The exception is GTK's transient
"password hint" character — the most recently typed one, briefly shown — which does appear in that
string.

---

## Defect 3 — `getCursorCapsMode` is answered from the cleared scratch buffer

Same root cause as Defect 2, separate symptom, and one an application only meets once it sets
`GTK_INPUT_HINT_UPPERCASE_SENTENCES`.

### Symptom

_Inference, not measurement._ A `GtkTextView` with
`input-hints: GTK_INPUT_HINT_UPPERCASE_SENTENCES` — which
`_gtk_im_context_android_get_input_type` correctly turns into
`InputType.TYPE_TEXT_FLAG_CAP_SENTENCES` — should get a keyboard that shifts the first letter of
_every_ word rather than of every sentence.

Marked as inference because it was read out of the code and fixed in the same change that first set
the hint downstream, so the broken behaviour was never shipped and never watched. What _is_
measured is the fixed behaviour: with the override below, sentence capitalisation in a real
composer is correct. Whoever files this should reproduce the unfixed case before quoting it.

### Cause

Android does not decide by itself which positions are sentence-initial: it asks the field, through
`InputConnection.getCursorCapsMode()`. `ImeConnection` does not override it, so the answer comes
from `BaseInputConnection.getCursorCapsMode`, which calls `TextUtils.getCapsMode` on the `Editable`
returned by `getEditable()` — the composing scratch buffer that `commitText` and
`finishComposingText` `clear()`.

Every call therefore describes an empty document with the cursor at 0, which is the start of a
sentence. The keyboard is being told the truth about the wrong document.

### Suggested fix

The same one-line-per-query shape as Defect 2, over the same `getSurrounding()`:

```java
@Override
public int getCursorCapsMode(int reqModes) {
	/* text and cursor from getSurrounding(), UTF-16 offsets */
	return TextUtils.getCapsMode(text, cursor, reqModes);
}
```

Unlike `setSelection`, nothing here needs new `GtkIMContext` API: caps mode is a pure question about
text GTK already hands over. Trap 3 from Defect 2 applies and is milder — caps mode only looks
backwards from the cursor, so `gtk_text_view_retrieve_surrounding_handler`'s window is enough
whenever the sentence began inside it. A sentence carried across a hard line break is reported as a
fresh one.

Carried downstream as [`patch-gtk-ime-caps.sh`](../build-aux/android/patch-gtk-ime-caps.sh).

---

## Defect 4 — `text_flag_cap_sentences` is filled from `TEXT_FLAG_CAP_WORDS`

A one-line typo, independent of the other three, with the same symptom as Defect 3.

### Cause

In `gtk_im_context_android_init_java_cache` (`gtk/gtkimcontextandroid.c`):

```c
FILL_INPUT_TYPE (text_flag_cap_words, "TEXT_FLAG_CAP_WORDS")
FILL_INPUT_TYPE (text_flag_cap_sentences, "TEXT_FLAG_CAP_WORDS")
```

The second line caches Android's `TYPE_TEXT_FLAG_CAP_WORDS` under the name
`text_flag_cap_sentences`. `_gtk_im_context_android_get_input_type` reads it for
`GTK_INPUT_HINT_UPPERCASE_SENTENCES`, so that hint asks the keyboard to capitalise every word.

### Evidence

_Measured_ on the emulator, `dumpsys input_method`, on a `GtkTextView` whose hints are
`GTK_INPUT_HINT_SPELLCHECK | GTK_INPUT_HINT_UPPERCASE_SENTENCES`:

| | `inputType` | means |
| --- | --- | --- |
| Before | `0xa001` | `CLASS_TEXT \| CAP_WORDS \| AUTO_CORRECT` |
| After | `0xc001` | `CLASS_TEXT \| CAP_SENTENCES \| AUTO_CORRECT` |

### Suggested fix

```c
FILL_INPUT_TYPE (text_flag_cap_sentences, "TEXT_FLAG_CAP_SENTENCES")
```

Carried downstream as
[`patch-gtk-caps-sentences.sh`](../build-aux/android/patch-gtk-caps-sentences.sh).

---

## Reproducing without a phone

Worth including in the report, because it is not obvious and it is how all of the above was
measured.

The emulator appears unable to test keyboards: Gboard draws its collapsed physical-keyboard strip
rather than a keyboard, because the AVD presents an alphabetic keyboard device. It is not a real
obstacle — **the strip's own menu has "Show on-screen keyboard"**, which draws the full QWERTY.
From there:

```sh
adb shell input swipe 790 1444 500 1444 1200   # a slow drag along the space bar
```

Three things make the difference between a measurement and a wasted afternoon:

* **`adb shell input text` is not typing.** It injects key events at the view and never touches the
  `InputConnection`, so the keyboard never learns the characters exist and its cursor model stays
  at 0 regardless of what the application answers. _Measured on one build:_ text entered that way
  produced `setSelection(0, 0)`; the same string typed by tapping Gboard's keys produced
  `setSelection(10, 10)`. Type by tapping key coordinates.
* **Run a control.** The identical swipe against the stock Settings search box moves the cursor
  seven characters, which establishes the swipe as a working instrument before it is pointed at
  GTK.
* **Read the result as a letter, not a caret.** After the gesture, `adb shell input text "X"`
  inserts at the widget's real cursor regardless of where the keyboard believes it to be.

## Open questions for maintainers

1. Is the `had_preedit` guard acceptable, or is `reset` expected to restart input for reasons not
   visible from here?
2. Should `GtkIMContext` gain a way for an input method to request a cursor position? Without one,
   every Android backend implementation of `setSelection` has to synthesise key events.
3. Is `gtk_text_view_retrieve_surrounding_handler`'s sliding window deliberate? It makes the
   offsets an input method receives non-absolute, which no `InputConnection` contract anticipates.

## Not investigated

* Whether other IMEs (Samsung, SwiftKey, HeliBoard) drive this gesture through `setSelection` as
  Gboard does, or through key events. Only Gboard was tested.
* `GET_EXTRACTED_TEXT_MONITOR`. `getExtractedText` is answered downstream, but the monitor flag is
  ignored — honouring it means calling `updateExtractedText` on every change, and there is no
  signal in this glue to do that from.
* Whether any of this affects `GtkIMContextSimple` compose sequences on Android.

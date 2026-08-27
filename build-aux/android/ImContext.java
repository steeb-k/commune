/*
 * Copyright (c) 2024 Florian "sp1rit" <sp1rit@disroot.org>
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library. If not, see <http://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: LGPL-2.1-or-later
 */

/* Commune's replacement for GTK's Android IME glue, installed over the
 * upstream file by build-aux/android/patch-gtk-ime-mirror.sh.
 *
 * Upstream's ImeConnection extends BaseInputConnection over a scratch
 * Editable that it clears after every commit, so every question the
 * keyboard asks -- text around the cursor, caps mode, the composing
 * region -- is answered from an empty document. Each defect that
 * produced was patched one method at a time until the composing-region
 * round showed the shape of the whole: the connection was backed by the
 * wrong document.
 *
 * This version is backed by the right one. The Editable is a mirror of
 * the text GTK reports around the cursor, BaseInputConnection edits it
 * with its stock method implementations -- which are the ones every
 * native Android text field uses -- and each finished edit batch is
 * translated into GTK's three verbs (deleteSurrounding, commit,
 * updatePreedit) inside a single GLib-thread closure. The same closure
 * re-reads the surrounding text afterwards and the mirror is rebuilt
 * from that answer, so the mirror can drift from GTK for at most one
 * transaction, and GtkTextView's sliding surrounding-text window is
 * harmless: offsets are never carried across a rebuild.
 */

package org.gtk.android;

import android.text.Editable;
import android.text.Selection;
import android.text.SpannableStringBuilder;
import android.text.Spanned;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.ExtractedText;
import android.view.inputmethod.ExtractedTextRequest;
import android.view.inputmethod.InputMethodManager;

import androidx.annotation.Keep;
import androidx.annotation.NonNull;

import java.util.logging.Logger;

public final class ImContext {
	public static final class SurroundingRetVal {
		public String text;
		public int cursor_index;
		public int anchor_index;

		private SurroundingRetVal(String text, int cursor_idx, int anchor_idx) {
			this.text = text;
			this.cursor_index = cursor_idx;
			this.anchor_index = anchor_idx;
		}
	}

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

	private long native_ptr;
	public ImContext(long native_ptr) {
		this.native_ptr = native_ptr;
	}

	public native int getInputType();

	public native SurroundingRetVal getSurrounding();
	public native boolean deleteSurrounding(int offset, int n_chars);

	/* Move the widget's cursor by `delta` codepoints, and leave the
	 * selection bound `anchor_delta` codepoints from where the cursor
	 * lands (0 collapses). GtkIMContext has no verb for this, and
	 * synthesising arrow keys loses ordering races against the very
	 * transaction that needs them -- key events drain from GDK's event
	 * queue in a different main-loop phase than an invoked closure. This
	 * places the cursor through the widget, synchronously, on the GLib
	 * thread. See build-aux/android/patch-gtk-ime-move-cursor.sh. */
	public native boolean moveCursor(int delta, int anchor_delta);

	public native String getPreedit();
	public native void updatePreedit(String preedit, int cursor);

	public native boolean commit(String string);

	@Keep
	private static void reset(View view) {
		InputMethodManager imm = view.getContext().getSystemService(InputMethodManager.class);
		imm.restartInput(view);
	}

	private volatile ImeConnection activeConnection;

	/* Called from gtk_im_context_android_reset on the GLib thread -- which
	 * runs after every GTK-side cursor movement (arrow keys, taps, a
	 * backspace, the application moving the caret) -- so that the keyboard's
	 * model of the document follows the widget instead of going stale until
	 * the next question it happens to ask. This is the moment a native
	 * Android editor would call updateSelection from.
	 * See build-aux/android/patch-gtk-ime-cursor-notify.sh.
	 */
	@Keep
	private void onGtkCursorMoved() {
		ImeConnection conn = activeConnection;
		if (conn != null)
			conn.notifyExternalChange();
	}

	final class ImeConnection extends BaseInputConnection {
		private final Logger logger;
		private final View target;

		/* The document the keyboard edits. Committed text and preedit both
		 * live in it, the preedit marked by a SPAN_COMPOSING span, exactly
		 * as in a TextView's buffer.
		 */
		private final SpannableStringBuilder mirror = new SpannableStringBuilder();

		/* GTK's state as of the last rebuild: the committed surrounding
		 * text (no preedit -- GTK keeps preedit out of the buffer), the
		 * insert mark and selection anchor in it (UTF-16), and the preedit
		 * GTK is currently drawing at the insert mark, if any. Every sync
		 * diffs the mirror against this and replaces it with a fresh read.
		 */
		private String gtkCommitted = "";
		private int gtkCursor = 0;
		private int gtkAnchor = 0;
		private String gtkPreedit = null;
		private int gtkPreeditCursorCp = 0;

		/* Bumped by every IME-driven rebuild, and only by those. A cursor
		 * notification snapshots the generation on the GLib thread; by the
		 * time it reaches the UI thread a sync may have rebuilt the mirror
		 * from newer state, and a snapshot from before that transaction
		 * must be dropped rather than roll the mirror back. Notifications
		 * do not bump it: two taps in quick succession post two adopts,
		 * and the second -- the one carrying the settled state -- must
		 * still apply.
		 */
		private volatile int generation = 0;

		private int batchDepth = 0;
		private boolean pendingSync = false;

		/* What updateSelection last told the keyboard, to keep quiet when
		 * nothing moved. */
		private int lastSelStart = -2, lastSelEnd = -2, lastCandStart = -2, lastCandEnd = -2;

		public ImeConnection(@NonNull View target) {
			super(target, true);
			this.logger = Logger.getLogger("IME Connection");
			this.target = target;
			ImContext.this.activeConnection = this;
			SurroundingRetVal s = GlibContext.blockForMain(() -> ImContext.this.getSurrounding());
			adopt(s, null, 0);
		}

		@Override
		public void closeConnection() {
			if (ImContext.this.activeConnection == this)
				ImContext.this.activeConnection = null;
			super.closeConnection();
		}

		@Override
		public Editable getEditable() {
			return mirror;
		}

		/* onCreateInputConnection seeds EditorInfo from here so the
		 * keyboard's first idea of the cursor is the mirror it will be
		 * talking to, not a separate read that may disagree with it. */
		void seed(EditorInfo outAttrs) {
			outAttrs.initialSelStart = Selection.getSelectionStart(mirror);
			outAttrs.initialSelEnd = Selection.getSelectionEnd(mirror);
			outAttrs.initialCapsMode = getCursorCapsMode(outAttrs.inputType);
			lastSelStart = outAttrs.initialSelStart;
			lastSelEnd = outAttrs.initialSelEnd;
			lastCandStart = -1;
			lastCandEnd = -1;
		}

		/* BaseInputConnection's own editing methods wrap themselves in
		 * begin/endBatchEdit, so these are re-entered from inside super
		 * calls; sync only when the outermost batch closes. */
		@Override
		public boolean beginBatchEdit() {
			batchDepth++;
			return true;
		}

		@Override
		public boolean endBatchEdit() {
			if (batchDepth > 0 && --batchDepth == 0 && pendingSync) {
				pendingSync = false;
				sync();
			}
			return batchDepth > 0;
		}

		private void afterEdit() {
			if (batchDepth == 0)
				sync();
			else
				pendingSync = true;
		}

		@Override
		public boolean setComposingText(CharSequence text, int newCursorPosition) {
			logger.info("IME: setComposingText(\"" + text + "\", " + newCursorPosition + ")");
			boolean handled = super.setComposingText(text, newCursorPosition);
			afterEdit();
			return handled;
		}

		@Override
		public boolean setComposingRegion(int start, int end) {
			logger.info("IME: setComposingRegion(" + start + ", " + end + ")");
			boolean handled = super.setComposingRegion(start, end);
			afterEdit();
			return handled;
		}

		@Override
		public boolean finishComposingText() {
			logger.info("IME: finishComposingText()");
			boolean handled = super.finishComposingText();
			afterEdit();
			return handled;
		}

		@Override
		public boolean commitText(CharSequence text, int newCursorPosition) {
			logger.info("IME: commitText(\"" + text + "\", " + newCursorPosition + ")");
			boolean handled = super.commitText(text, newCursorPosition);
			afterEdit();
			return handled;
		}

		@Override
		public boolean deleteSurroundingText(int beforeLength, int afterLength) {
			logger.info("IME: deleteSurroundingText(" + beforeLength + ", " + afterLength + ")");
			boolean handled = super.deleteSurroundingText(beforeLength, afterLength);
			afterEdit();
			return handled;
		}

		@Override
		public boolean deleteSurroundingTextInCodePoints(int beforeLength, int afterLength) {
			logger.info("IME: deleteSurroundingTextInCodePoints(" + beforeLength + ", " + afterLength + ")");
			boolean handled = super.deleteSurroundingTextInCodePoints(beforeLength, afterLength);
			afterEdit();
			return handled;
		}

		@Override
		public boolean setSelection(int start, int end) {
			logger.info("IME: setSelection(" + start + ", " + end + ")");
			boolean handled = super.setSelection(start, end);
			afterEdit();
			return handled;
		}

		/* BaseInputConnection answers null here, and a null reads to the
		 * keyboard as an empty document. The monitor flag is still not
		 * honoured; updateSelection carries the changes a monitor would.
		 */
		@Override
		public ExtractedText getExtractedText(ExtractedTextRequest request, int flags) {
			ExtractedText extracted = new ExtractedText();
			extracted.text = mirror.toString();
			extracted.startOffset = 0;
			extracted.partialStartOffset = -1;
			extracted.partialEndOffset = -1;
			extracted.selectionStart = Selection.getSelectionStart(mirror);
			extracted.selectionEnd = Selection.getSelectionEnd(mirror);
			extracted.flags = 0;
			return extracted;
		}

		/* ------------------------------------------------------------- */

		/* GLib thread. Reads the state that made the cursor notification
		 * fire while still on the thread that owns it, then hands the
		 * result to the UI thread, where the generation check drops it if
		 * a sync got there first.
		 */
		void notifyExternalChange() {
			final int gen = generation;
			final SurroundingRetVal s = ImContext.this.getSurrounding();
			if (s == null || s.text == null)
				return;
			target.post(() -> {
				if (generation != gen)
					return;
				if (batchDepth > 0)
					return;
				if (getComposingSpanStart(mirror) >= 0)
					/* A GTK-side cursor move during composition means
					 * reset cleared the preedit and restartInput is on
					 * its way; the replacement connection re-reads. */
					return;
				adopt(s, null, 0, false);
				/* The snapshot above was taken when reset ran, which for
				 * a gesture can be before the gesture's outcome -- a
				 * double-tap's reset fires before the word is selected.
				 * Look again once the dust settles. */
				target.postDelayed(() -> reconcile(gen), 150);
			});
		}

		/* Compare the mirror against a fresh read and adopt the
		 * difference, if the world has not moved on in the meantime. */
		private void reconcile(int gen) {
			if (generation != gen || batchDepth > 0
			    || getComposingSpanStart(mirror) >= 0)
				return;
			SurroundingRetVal s = GlibContext.blockForMain(() -> ImContext.this.getSurrounding());
			if (s == null || s.text == null)
				return;
			String text = s.text;
			int cursor = toUtf16(text, s.cursor_index);
			int anchor = toUtf16(text, s.anchor_index);
			if (text.equals(gtkCommitted) && cursor == gtkCursor && anchor == gtkAnchor)
				return;
			adopt(s, null, 0, false);
		}

		/* Translate what the keyboard did to the mirror into GTK's verbs.
		 * Runs on the connection's thread with the outermost batch closed.
		 */
		private void sync() {
			final String m = mirror.toString();
			int spanA = getComposingSpanStart(mirror);
			int spanB = getComposingSpanEnd(mirror);
			if (spanB < spanA) {
				int swap = spanA;
				spanA = spanB;
				spanB = swap;
			}
			final boolean composing = spanA >= 0 && spanB > spanA;
			final int ca = composing ? spanA : -1;
			final int cb = composing ? spanB : -1;

			int sa = Selection.getSelectionStart(mirror);
			int sb = Selection.getSelectionEnd(mirror);
			if (sa < 0)
				sa = sb < 0 ? 0 : sb;
			if (sb < 0)
				sb = sa;

			final String preedit = composing ? m.substring(ca, cb) : null;
			final String mc = composing ? m.substring(0, ca) + m.substring(cb) : m;
			final int preCurCp = composing
				? m.codePointCount(ca, Math.max(ca, Math.min(sb, cb)))
				: 0;

			final int wantCursor = toCommitted(sb, ca, cb);
			final int wantAnchor = toCommitted(sa, ca, cb);

			/* Diff the committed view of the mirror against GTK's last
			 * known committed text: common prefix, common suffix, and the
			 * replacement between them, without splitting surrogate pairs
			 * at either boundary. */
			final int shared = Math.min(gtkCommitted.length(), mc.length());
			int p = 0;
			while (p < shared && gtkCommitted.charAt(p) == mc.charAt(p))
				p++;
			if (p > 0 && p < gtkCommitted.length() && p < mc.length()
			    && Character.isHighSurrogate(gtkCommitted.charAt(p - 1)))
				p--;
			int suf = 0;
			while (suf < shared - p
			       && gtkCommitted.charAt(gtkCommitted.length() - 1 - suf) == mc.charAt(mc.length() - 1 - suf))
				suf++;
			if (suf > 0 && Character.isLowSurrogate(gtkCommitted.charAt(gtkCommitted.length() - suf)))
				suf--;
			final int delStart = p;
			final int delEnd = gtkCommitted.length() - suf;
			final String ins = mc.substring(p, mc.length() - suf);

			final boolean textChanged = delEnd > delStart || !ins.isEmpty();
			final boolean preeditChanged = !stringEquals(preedit, gtkPreedit)
				|| (composing && preCurCp != gtkPreeditCursorCp);

			if (!textChanged && !preeditChanged) {
				if (composing || gtkPreedit != null
				    || (wantCursor == gtkCursor && wantAnchor == gtkAnchor)) {
					maybeUpdateSelection();
					return;
				}
				/* A pure cursor move. */
				final String expectMoveText = gtkCommitted;
				final int wantCurCp = expectMoveText.codePointCount(0, wantCursor);
				final int wantAnchorCp = expectMoveText.codePointCount(0, wantAnchor);
				final SurroundingRetVal moved = GlibContext.blockForMain(() -> {
					SurroundingRetVal now = ImContext.this.getSurrounding();
					if (now == null || now.text == null || !now.text.equals(expectMoveText))
						return now;
					if (now.cursor_index != wantCurCp || now.anchor_index != wantAnchorCp)
						ImContext.this.moveCursor(wantCurCp - now.cursor_index,
						                          wantAnchorCp - wantCurCp);
					return ImContext.this.getSurrounding();
				});
				adopt(moved, null, 0);
				return;
			}

			/* A text or preedit transaction. commit() inserts at the insert
			 * mark and a preedit draws there, so when text is inserted the
			 * mark must stand at the end of the changed region first:
			 * deleting [delStart, delEnd) then pulls it to delStart, which
			 * is where the insertion and the preedit both belong. The
			 * closure places the mark itself, through moveCursor, so a
			 * cursor that drifted since the mirror last looked is repaired
			 * rather than fatal; only the text changing underneath aborts.
			 * With a preedit on screen the cursor is not placed -- a
			 * programmatic move would reset the IM context and restart
			 * input mid-transaction -- but there the standard sequences
			 * already hold the mark at the composing anchor, and the
			 * deletion offset is relative, so deletions still land.
			 */
			final int needBefore = textChanged ? delEnd : gtkCursor;
			final int needCp = gtkCommitted.codePointCount(0, needBefore);
			final int delStartCp = gtkCommitted.codePointCount(0, delStart);
			final int delCp = gtkCommitted.codePointCount(delStart, delEnd);
			final String expectText = gtkCommitted;
			final String expectPreedit = gtkPreedit;
			final boolean[] applied = { false };

			/* Where the keyboard wants the cursor to rest, measured from
			 * where commit() will leave it. Almost always zero; commitText
			 * with newCursorPosition other than 1 is the exception. */
			final int landing = delStart + ins.length();
			final int postMoveCp = !composing && textChanged
				? (wantCursor >= landing
				   ? mc.codePointCount(landing, Math.min(wantCursor, mc.length()))
				   : -mc.codePointCount(Math.max(wantCursor, 0), landing))
				: 0;

			final SurroundingRetVal fresh = GlibContext.blockForMain(() -> {
				/* The baseline the diff was computed against must still be
				 * what the widget holds, or the edits would land somewhere
				 * else. A failed check drops the keyboard's edit; the
				 * rebuild below then tells it what the document really is.
				 */
				SurroundingRetVal now = ImContext.this.getSurrounding();
				if (now == null || now.text == null)
					return now;
				if (!now.text.equals(expectText)
				    || !stringEquals(ImContext.this.getPreedit(), expectPreedit))
					return now;

				applied[0] = true;
				int at = now.cursor_index;
				if (expectPreedit == null && (at != needCp || now.anchor_index != at)) {
					ImContext.this.moveCursor(needCp - at, 0);
					at = needCp;
				} else if (expectPreedit != null && at != needCp && !ins.isEmpty()) {
					logger.warning("IME: insertion needs the cursor at " + needCp
					               + " but a preedit holds it at " + at);
				}
				if (delCp > 0)
					ImContext.this.deleteSurrounding(delStartCp - at, delCp);
				if (!ins.isEmpty())
					/* commit() also retires any preedit still on screen. */
					ImContext.this.commit(ins);
				else if (expectPreedit != null && preedit == null)
					ImContext.this.updatePreedit(null, 0);
				if (preedit != null)
					/* Sets or replaces in place: no preedit-end/-start
					 * cycle between two composing states. */
					ImContext.this.updatePreedit(preedit, preCurCp);
				if (postMoveCp != 0)
					ImContext.this.moveCursor(postMoveCp, 0);
				return ImContext.this.getSurrounding();
			});

			if (!applied[0])
				logger.warning("IME: transaction dropped, the text changed underneath it");
			adopt(fresh, applied[0] ? preedit : null, applied[0] ? preCurCp : 0);
		}

		/* Rebuild the mirror from a fresh GTK answer. Everything the
		 * keyboard is told from here on -- queries, updateSelection --
		 * derives from this one state, so its offsets agree with each
		 * other even when GtkTextView's surrounding window slides.
		 */
		private void adopt(SurroundingRetVal s, String preedit, int preeditCursorCp) {
			adopt(s, preedit, preeditCursorCp, true);
		}

		private void adopt(SurroundingRetVal s, String preedit, int preeditCursorCp, boolean imeDriven) {
			if (imeDriven)
				generation++;
			String text = s != null && s.text != null ? s.text : "";
			int cursor = s != null ? toUtf16(text, s.cursor_index) : 0;
			int anchor = s != null ? toUtf16(text, s.anchor_index) : 0;

			gtkCommitted = text;
			gtkCursor = cursor;
			gtkAnchor = anchor;
			gtkPreedit = preedit != null && !preedit.isEmpty() ? preedit : null;
			gtkPreeditCursorCp = gtkPreedit != null ? preeditCursorCp : 0;

			removeComposingSpans(mirror);
			mirror.replace(0, mirror.length(), text);
			if (gtkPreedit != null) {
				mirror.insert(cursor, gtkPreedit);
				mirror.setSpan(new Object(), cursor, cursor + gtkPreedit.length(),
				               Spanned.SPAN_EXCLUSIVE_EXCLUSIVE | Spanned.SPAN_COMPOSING);
				int at = cursor + toUtf16(gtkPreedit, gtkPreeditCursorCp);
				Selection.setSelection(mirror, at, at);
			} else {
				Selection.setSelection(mirror, anchor, cursor);
			}
			maybeUpdateSelection();
		}

		private void maybeUpdateSelection() {
			final int selStart = Selection.getSelectionStart(mirror);
			final int selEnd = Selection.getSelectionEnd(mirror);
			int spanA = getComposingSpanStart(mirror);
			int spanB = getComposingSpanEnd(mirror);
			if (spanB < spanA) {
				int swap = spanA;
				spanA = spanB;
				spanB = swap;
			}
			final int candStart = spanA;
			final int candEnd = spanB;
			if (selStart == lastSelStart && selEnd == lastSelEnd
			    && candStart == lastCandStart && candEnd == lastCandEnd)
				return;
			lastSelStart = selStart;
			lastSelEnd = selEnd;
			lastCandStart = candStart;
			lastCandEnd = candEnd;
			target.post(() -> {
				InputMethodManager imm =
					target.getContext().getSystemService(InputMethodManager.class);
				if (imm != null)
					imm.updateSelection(target, selStart, selEnd, candStart, candEnd);
			});
		}

		/* A mirror offset mapped into committed-text coordinates: preedit
		 * characters do not exist in GTK's buffer. */
		private int toCommitted(int offset, int ca, int cb) {
			if (ca < 0 || offset <= ca)
				return offset;
			return offset >= cb ? offset - (cb - ca) : ca;
		}

		private boolean stringEquals(String a, String b) {
			return a == null ? b == null : a.equals(b);
		}

	}
}

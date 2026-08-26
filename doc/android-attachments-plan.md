# Files on Android: what works, what does not, and why

Working notes for finishing attachment support on the Android port. Written 25 August 2026, after
S4 step 2 turned up two defects that had nothing to do with media and everything to do with files.

Everything marked _measured_ was measured on the emulator. Everything marked _inference_ was read
out of the source and not run.

## The one root cause

Every file the user picks on Android arrives as a `content://` URI wrapped in a
`GdkAndroidContentFile`. Three facts about that object explain every symptom below.

1. **GIO operations on it work.** `read`, `replace`, `query_info`, `enumerate_children` are all
   implemented. Anything that goes through `GFile` is fine — _after_
   `build-aux/android/patch-gtk-jni-attach.sh`, without which the async ones segfault.
2. **It has no filesystem path, and does not say so.** `g_file_get_path` answers `Uri.getPath()`,
   so `content://…/document/video%3A1000000034` becomes `/document/video:1000000034`. GIO's
   contract is that `get_path` returns NULL when there is no local path. This returns a plausible
   lie instead.
3. **`GStreamer` cannot open it.** `GdkAndroidContentFile` is not registered as a GVfs backend, so
   `content://` has no URI handler anywhere in GIO's URI namespace. Anything given `file.uri()` —
   `GstDiscoverer`, `gst_play::Play`, `uridecodebin` — fails.

Fact 2 is the expensive one, because it defeats every `if let Some(path)` guard in the codebase.
Code that would have failed loudly and early instead proceeds with a path that points at nothing
and fails somewhere else entirely, which is exactly how "Invalid attachment data" was reached.

## Inventory

| Route | Reaches a file by | State |
| --- | --- | --- |
| Composer, file picker | path, then URI for preview/thumbnail | **fixed** (`f1765ed`) — _measured_ with an mp4 |
| Composer, clipboard paste | bytes, already | **inference**: fine, it never had a path |
| Composer, voice recorder | writes its own temp file | **inference**: fine |
| Composer, camera | — | declined on Android (`c3724a3c`) |
| Received attachment, view | temp file we wrote | **fixed earlier** (`eb951d5a`) — _measured_ |
| Received attachment, **save as** | `GFile::replace_contents` on the chosen file | **untested**; GIO-level, so it may already work |
| History viewer, **save file** | same | **untested**, same shape |
| Avatar picker | `query_info_future`, `load_contents_future` | **untested**; GIO-level, expected to work post-patch |
| Image pack editor | `open_multiple_future` then `load_contents_future` | **untested**, same shape |
| **Key import** | hands the path to `import_room_keys` | **fixed** — _measured_, round-tripped a real export through the picker |
| **Key export** | hands the path to `export_room_keys` | **fixed** — _measured_, same round trip |

Two smaller things found while reading, neither a crash:

* `utils/media/image/queue.rs` did `file.path().expect("file should have a path")` for a cache
  key. It did not fire only because of fact 2 — the lie is at least unique and stable. **Fixed in
  step 1**, and not by reaching for `local_path`: the request id wants a key that always exists,
  not a path, so `ImageRequestId::File` now holds `file.as_gfile().uri()`. Every `GFile` has a URI,
  `content://` ones included, and for a temporary file it is the same `file://` path spelled
  differently. There is nothing left to panic on.
* `AudioPlayerSource::name` takes the display name from the path's last component, so previewing a
  picked audio file names it after a document id, or after our temp file. Cosmetic, and
  **deliberately left in step 1**: routing it through `local_path` would turn a bad name into an
  empty one. The fix is a real display name — `query_info` for `G_FILE_ATTRIBUTE_STANDARD_DISPLAY_NAME`
  — not a path check, so it is its own change.

## Where to handle the missing path

The tempting move is to patch `gdk_android_content_file_get_path` to return NULL, since that is
what GIO's contract says and what every caller already checks for. **Checked, and it is the wrong
default.** Three things argue against it.

_Measured by reading GTK._ `gdk_android_content_file_get_basename` calls
`g_path_get_basename (g_file_get_path (file))` with no NULL check, so the patch is at minimum two
functions and we are rewriting more of a backend we do not own. That same call reframes the
premise: GTK uses `Uri.getPath()` as a **name** source, not as a filesystem path, which is a
defensible reading of what the function is for. "It returns a lie" is our framing, not an agreed
fact, and upstream could reasonably disagree.

Second, it is a removal rather than an addition. Every other script in `build-aux/android/` adds
something GTK is missing; this one takes away information GTK currently supplies, so the blast
radius covers GTK internals and anything else reading a path — and it surfaces as _new_ unknown
bugs rather than as the known one.

Third, it does not reduce the work. `GStreamer` still cannot open `content://`, so the
copy-to-temporary-file machinery is needed either way. The patch buys honesty, not fewer code paths.
And an honest refusal still needs a message: `can_proceed` going false makes the key page silently
inert, which is worse than failing loudly unless the explanation is written too.

**So: handle it in Commune.** One helper — `local_path(file) -> Option<PathBuf>`, returning
`file.path().filter(|path| path.exists())` — used everywhere in place of `.path()`. No divergence
from upstream, no blast radius, and correct on every platform, because a stale path is stale
anywhere. It is what `send_file_inner` already does, promoted to a name. The cost is discipline: a
convention enforced by grep and review rather than by the compiler.

One limit of it, found while writing it and relevant to steps 2 and 3: **it is a test for reading.**
A file the user has just picked as a _save_ destination does not exist yet, so `local_path` answers
`None` for a perfectly good one. Key export and every save-as route below choose a file that is
about to be created, and none of them may use this helper as their guard — they want the parent
directory, or they want `replace_contents` on the `GFile` and no path at all. The doc comment on
the helper says so; this is the reason it says so.

Two findings that cut the other way, recorded so nobody re-derives them:

* `gdkcontentserializer.c` already handles a NULL path and falls back to the URI — and today it
  does the **wrong** thing on Android, converting the fake path into a bogus `file://` URI for the
  clipboard and for drag and drop. _Inference_; not reproduced. If copying a file out of Commune
  turns out to be broken, this is why, and it is an argument for the GTK patch after all.
* `gtk_file_launcher_launch` has an Android branch (`gtk_show_file_android`) that never touches the
  path, so "open with" is unaffected either way.

Keep the GTK patch in reserve for the case where GTK internals are caught misbehaving. Do not lead
with it.

## The work, in order

1. **Add `local_path`** and route every `.path()` in the inventory through it. Fix the `expect` in
   the image queue at the same time — not because it fires today, but because it is the one call
   site that would turn a NULL into a panic if the GTK patch is ever reached for.

   **Done**, and _measured_ on the emulator: picking `t3.png` from the document picker previewed
   it and sent it, and it came back down as an image in the timeline. That one flow exercises both
   halves of the change — the preview renders from the temporary copy, which only exists because
   `local_path` answered `None`, and the preview is an `ImageRequestSource::File` going through the
   image queue on its new URI key. Nothing was logged at warn or above for the whole flow.

   `crate::utils::local_path` is the helper; the composer now calls it instead of
   spelling the check out inline, and the image queue keys on a URI. Two `.path()` call sites were
   left alone on purpose, and both are the point rather than an oversight:

   * `AudioPlayerSource::name`, for the reason above.
   * `ImportExportKeysSubpage`, in `can_proceed` and in `proceed`. Routing those through
     `local_path` disables the button on Android, and **today the page fails loudly**: import runs,
     `import_room_keys` cannot open `/document/…`, and a "Could not import the keys" toast says so.
     Trading a wrong-but-visible failure for a dead control with no explanation is the exact trap
     this document argues against for the GTK patch, and it would apply here too. Step 2 is what
     makes the page work; it can take these two call sites with it.

   `utils::File` lost its `path()` accessor rather than gaining a `local_path()` one — the image
   queue was its only caller. Everything else reaches a `File` through `as_gfile()`, so the free
   function covers them.
2. **Key import and export.** Import is the composer's problem again: copy to a temp file, import
   from that. Export is the mirror and is harder, because `export_room_keys` writes to a path —
   export to a temp file, then `replace_contents` into the chosen `GFile`. Check first whether
   `matrix-sdk` has a variant that returns bytes.

   **Checked: it does not, and the pieces to build one are out of reach.** `Encryption::export_room_keys`
   and `Encryption::import_room_keys` are the only public entry points and both take a `PathBuf`.
   The primitives underneath are bytes-first and public — `encrypt_room_key_export` returns a
   `String`, `decrypt_room_key_export` takes any `Read` — but reaching them means going through
   `Client::olm_machine()`, which is `pub(crate)`; the only public accessor is
   `olm_machine_for_testing`. Reimplementing on top of that would also mean reimplementing the
   `maybe_trigger_backup` that `import_room_keys` does afterwards. So the temp file is the answer
   in both directions, and it is the same answer on every platform.

   Two details that fall out of it. Export cannot decide whether to skip the temporary file by
   asking `local_path`, because the destination has not been created yet and the helper answers
   `None` for every save target — so export goes through a temporary file **always**, and
   `replace_contents` writes the result. And the temporary file for export wants
   `NamedTempFile::into_temp_path()` rather than a live `NamedTempFile`: `export_room_keys` calls
   `std::fs::File::create` on the path itself, and handing it a path we still hold open is asking
   for trouble on Windows. `can_proceed` stops asking for a path at all and asks only that a file
   was chosen.

   **Done**, and _measured_ on the emulator with a real round trip: exported through
   `ACTION_CREATE_DOCUMENT` to a `content://` destination, pulled the file off the device — a
   165-byte, well-formed `-----BEGIN MEGOLM SESSION DATA-----` block — then fed that same file back
   through the picker into Import. Both directions closed their subpage and neither logged a
   warning or an error, which is this subpage's only success signal; a failure leaves the subpage
   open with a toast instead. The `/document/17` file the picker chose is the same on both sides,
   which is what proves the round trip used the actual exported bytes rather than two unrelated
   successes.

   One bug found only by running it: `finish_export`'s first draft read the temporary file back
   with `tokio::fs::read`, which needs an _ambient_ Tokio runtime — `Handle::current()` — and
   `proceed` is a template callback running on the GLib main context, not inside `spawn_tokio!`.
   It panicked with "there is no reactor running" the first time it actually ran. `RUNTIME
   .spawn_blocking(move || std::fs::read(path))` is the fix, and it is what
   `save_data_to_tmp_file`/`tmp_file_path` already do for the same reason -- `spawn_blocking`
   carries its own runtime handle, so it works from a plain async fn with no ambient runtime.
   Nothing else in this change used `tokio::fs`, so this was the only site to check.
3. **Test the save-as routes.** `replace_contents` on a picked `GFile` should work; `save_future`
   on Android should present `ACTION_CREATE_DOCUMENT`. Both are plausible and neither is measured.
   If they work, this step is a paragraph in `doc/android.md` and nothing else.
4. **Test avatar and image-pack pickers.** Both are GIO-level and expected to be fine post-patch,
   which is exactly the kind of expectation this port has already punished twice.
5. **Sweep for the pattern.** `grep` for `.path()` and for `file.uri()`, and check each against the
   two facts above. The list in this document was built that way and is only as good as that grep.

## What would make this unnecessary

Registering `GdkAndroidContentFile` as a GVfs backend for the `content` scheme would fix fact 3
outright: `GStreamer` would resolve `content://` through `giosrc` like any other GIO URI, and the
copy in the composer could go away. That is a real GTK feature request rather than a patch, and it
does nothing about fact 2. Worth raising upstream alongside the JNI one; not worth waiting for.

## Upstream

Two of the three facts are arguably GTK defects, and both are candidates for a report alongside
`doc/upstream-gtk-android-ime.md`:

* The JNI crash, which is already patched downstream and is unambiguous — GIO calls the
  synchronous `GFile` vfuncs on a thread pool by construction, and the backend cannot survive it.
* `get_path` returning a non-path, pending the question above.

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
| **Key import** | hands the path to `import_room_keys` | **broken** — _inference_, the path does not exist |
| **Key export** | hands the path to `export_room_keys` | **broken** — _inference_, same |

Two smaller things found while reading, neither a crash:

* `utils/media/image/queue.rs` does `file.path().expect("file should have a path")` for a cache
  key. It does not fire today only because of fact 2 — the lie is at least unique and stable. If
  `get_path` is ever fixed to return NULL, **this panics**, so it has to be dealt with in the same
  change.
* `AudioPlayerSource::name` takes the display name from the path's last component, so previewing a
  picked audio file names it after a document id, or after our temp file. Cosmetic.

## The question to settle first

**Should `gdk_android_content_file_get_path` return NULL?**

Arguments for: it is what GIO's contract says, it is what every caller already checks for, and it
turns a confusing late failure into an honest early one. The composer would take its copy branch;
`can_proceed` on the key pages would correctly refuse.

Argument against, and the reason this is a question rather than a patch: `GdkAndroidContentFile` is
built from any `android.net.Uri`, and for a `file://` one `Uri.getPath()` is a real path. So the
patch is not "return NULL" but "return NULL unless the scheme is `file`" — which needs checking
against how the backend is actually constructed, and against whether anything upstream depends on
the current behaviour.

Settling this first matters because it changes what the rest of the work looks like: with an honest
`get_path`, most routes below become "handle NULL", which is ordinary. Without it, every route needs
the same `path.exists()` dance the composer now does, and a reviewer has to be told why each time.

## The work, in order

1. **Settle the `get_path` question**, and either patch GTK downstream or write down why not.
   Fix the `expect` in the image queue in the same change.
2. **Key import and export.** Import is the composer's problem again: copy to a temp file, import
   from that. Export is the mirror and is harder, because `export_room_keys` writes to a path —
   export to a temp file, then `replace_contents` into the chosen `GFile`. Check first whether
   `matrix-sdk` has a variant that returns bytes.
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

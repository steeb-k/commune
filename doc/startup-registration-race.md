# The single-instance guarantee has a race, and it is not Windows' bug

`doc/windows-plan.md` asked, as one of its open questions, whether `GApplication` uniqueness works
without a session bus, and answered "yes" from a real test: a second invocation exited on its own
and left one window, and a second invocation carrying a `matrix:` URI reached the first instance's
`Application::open` instead of opening its own. Both true. But testing this session (2026-08-25,
`windows-port`) found a second invocation that does _not_ get rejected — reliably, on demand, with a
timing change that has nothing to do with Windows' registration mechanism itself:

* Launch `commune.exe`, wait 15 seconds, launch it again: one process. The second exits immediately
  with nothing on stderr — correctly detected it was not primary, forwarded, exited clean.
* Launch it, wait 4 seconds, launch it again: inconsistent. Sometimes one process, sometimes two,
  same script, same machine, back to back.
* With two, both are real: both show up in `Get-Process`, both have their own PID and start time,
  neither is a zombie or a forwarding stub.

`doc/windows-plan.md`'s M1 note already anticipated a _related_ false positive — "a first instance
that fails to start makes the second one look like a second primary, because it is one" — but that
is a different failure than this one. This is a first instance that **is going to start
successfully**, just not yet, at the moment the second launch checks.

## Why: registration happens after the expensive part, not before it

`main()` does real work before `Application::run()` is ever called — the only place
`g_application_register()` actually runs: `app_bundle::init()`, `gtk::init()`, `gst::init()`
(GStreamer's plugin registry scan, which is not fast, especially cold), loading both gresources, and
on Windows two additional rounds of WinRT/COM setup (`windows_notifications::init()`,
`windows_toast_activator::init()`). None of that is gated on being the primary instance — a second
launch that starts inside that window runs through all the same setup, on the same footing as the
first, because as far as either process knows at that point, it might be the only one.

This is not a Windows-only mechanism bug. `GApplication` uniqueness on win32 does not need a session
bus and does work, exactly as M1 found — the race is in Commune's own startup ordering, and the same
ordering exists in `main()` on every platform. What differs is how wide the window is in practice.
Linux — especially a warm Flatpak — gets through `gtk::init()`/`gst::init()`/resource loading fast
enough that two clicks close together rarely both land inside it. Windows starts colder (a few
hundred DLLs to load, no long-running session to inherit warm caches from) and does more work before
`Application::run()` besides, so the window is wide enough that a real user's double-click — or a
single click followed by an impatient second one, because a cold start gives no visible feedback for
several seconds — is a plausible way to hit it, not just a scripted one.

Two real instances of a Matrix client pointed at the same local `matrix-sdk-sqlite` store is worse
than a duplicate window: it is two writers.

## The fix belongs in shared code, not `#[cfg(windows)]`

The idiomatic `GApplication` pattern is for `main()` to do as little as possible before constructing
the `Application` and calling `.run()`, with first-run setup living in the `startup` vtable
instead — which `GApplication` only ever invokes on the confirmed primary, after registration
succeeds. A rejected second instance never reaches it. Moving `gtk::init()`, `gst::init()`,
gresource loading, icon theme setup, and the Windows notification/toast COM registration into
`ApplicationImpl::startup()` (or equivalent) would narrow the race window to whatever
`g_application_register()` itself costs, which is small, rather than to all of that besides.
`app_bundle::init()` looks like it has to stay in `main()`, since it sets environment variables
that must land before any thread exists — worth confirming when this is actually picked up, not
assumed here.

Not attempted on `windows-port`: this touches cross-platform startup ordering, not anything
Windows-specific, and a platform port branch is the wrong place to carry a refactor like that alone.
Flagging it here for whoever does the merge back to `main`.

## Reproducing it

```powershell
Start-Process -FilePath "path\to\commune.exe"
Start-Sleep -Seconds 4          # or less; 15 seconds does not reproduce it
Start-Process -FilePath "path\to\commune.exe"
Start-Sleep -Seconds 4
Get-Process commune | Select-Object Id, StartTime   # two rows means it happened
```

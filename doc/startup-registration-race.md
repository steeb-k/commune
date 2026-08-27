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
succeeds. A rejected second instance never reaches it.

**Done, on the merge back to `main`, where this said it belonged.** `ApplicationImpl::startup` now
loads both gresources, chains up to the parent, and then does `gst::init()`, `aperture::init()`,
the icon theme and the colour scheme. `main()` keeps only what cannot move:

* `app_bundle::init()`, which sets environment variables and so must land while the process is
  still single-threaded. The guess above was right.
* On Windows, `windows_app_id::init()` and the two COM registrations. The toast activator chooses
  the process's COM apartment deliberately, _before_ GTK can choose it — so it has to stay ahead of
  GTK, and GTK is now started later rather than earlier, which preserves that ordering rather than
  disturbing it.

Two things this said or implied turned out to be wrong, and both were found by measuring rather
than reasoning:

* **`gtk::init()` is not called in `main()` at all now.** `GtkApplication` starts GTK in the
  `startup` ours chains up to, which is how a GTK application is ordinarily written. That single
  line was **459ms** of the pre-registration path — far more than everything this document
  originally pointed at.
* **`set_up_color_scheme()` had to move too, and it was not on the list.** It runs from
  `ObjectImpl::constructed`, which fires while `main()` builds the `Application` — before anything
  has started libadwaita. It reaches `AdwStyleManager::default()`, and with GTK no longer started
  in `main()` it aborted the process outright. It is called from `startup` now, which is also the
  first moment it could matter, since there is no window until `activate`.

## What it cost, measured

The width of the race window is how long a launch takes to find out it is not primary: a rejected
second instance's whole lifetime. Measured by interleaving the two binaries in the same directory,
alternating launch by launch against one warm primary, so machine load fell on both equally — which
matters, because consecutive runs of the _same_ binary varied by a factor of two when a Rust build
had just finished:

| | pre-fix | fixed |
| --- | --- | --- |
| second-instance lifetime | 997ms avg, 936ms min | **442ms avg, 413ms min** |
| of which is our own code, before registration | 598ms | **63ms** |

So the application's own share of the window is gone — 598ms to 63ms — and the window itself is
**56% narrower**.

**It is narrowed, not closed, and this document should not have implied otherwise.** "Whatever
`g_application_register()` itself costs, which is small" was wrong: roughly 380ms of the remaining
442ms is spent before `main()` is entered at all, mapping the GTK, GStreamer and libadwaita DLLs
that the executable links against. No amount of reordering reaches code that has not started
running. Closing the rest would mean not linking them, or taking a lock outside the process before
loading anything — neither of which is worth it for a window that now needs two launches inside
four tenths of a second.

Verified on both platforms after the change: Windows (a second launch still exits clean, still
forwards a `matrix:` URI to the primary's `open`, the toast activator still registers ahead of GTK
and notifications still arrive) and Arch Linux (a second launch on one session bus still exits on
its own; 156 tests pass).

## Reproducing it

```powershell
Start-Process -FilePath "path\to\commune.exe"
Start-Sleep -Seconds 4          # or less; 15 seconds does not reproduce it
Start-Process -FilePath "path\to\commune.exe"
Start-Sleep -Seconds 4
Get-Process commune | Select-Object Id, StartTime   # two rows means it happened
```

A warm machine does not reproduce it at four seconds any more, and did not reliably before the fix
either — the session that found it had a colder one. To see the window rather than wait for it to
be hit, measure the second instance instead: start one, let it settle, then time how long a second
launch lives before it exits. That number is the window, and it is the one in the table above.

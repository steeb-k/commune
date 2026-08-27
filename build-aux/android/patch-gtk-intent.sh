#!/bin/sh
# Make GTK's Android glue deliver a redirect to a running Commune.
#
#     sh build-aux/android/patch-gtk-intent.sh [ToplevelActivity.java path]
#
# Run this after `pixiewood generate` and before `pixiewood build`, next to
# `patch-manifest.sh` and `patch-gtk-ime.sh`. pixiewood copies the glue's Java
# into the Gradle project on every `generate`, so this cannot be applied once
# and forgotten.
#
# # What is wrong
#
# `ToplevelActivity.onCreate` reads `getIntent().getData()` and, if it is set,
# hands it to `GdkContext.open()` — the path that becomes
# `Application::process_uri()` on the Rust side, and the path the OAuth 2.0 /
# Matrix SSO redirect on Android needs (`src/login/local_server.rs`).
#
# That only runs in `onCreate`, though, and `patch-manifest.sh` sets
# `launchMode="singleTask"` so that returning to a running Commune resumes it
# instead of stacking a second, empty Activity in front — which means
# `onCreate` does not run for a redirect into an already-running app.
# `onNewIntent` is what Android calls instead, and `ToplevelActivity` does not
# override it: the `Intent` arrives, `getIntent()` still returns the old one,
# and the redirect is silently dropped. This is the same failure the macOS
# port hit with `-application:openURLs:` — see `src/utils/macos_url_events.rs`
# — for the same underlying reason: GTK's glue only forwards a URL through the
# path it already had wired for launch.
#
# # The fix
#
# Add the override, doing what `onCreate` already does with an incoming
# `Intent`'s data: read the last segment of its action as a hint, and hand the
# `Uri` to `GdkContext.open()` on the GTK thread via `GlibContext.blockForMain`
# — the same wrapping `onCreate`'s own call uses. `setIntent(intent)` comes
# first, Android's documented way to make a later `getIntent()` see the new
# `Intent` rather than the one Commune was launched with.
#
# # When to delete this
#
# When GTK's glue grows its own `onNewIntent`. The script fails rather than
# silently doing nothing if the anchor text it expects is gone, so that a GTK
# update which adds or reworks this is noticed here instead of being papered
# over.
set -eu

JAVA=${1:-.pixiewood/android/app/src/main/java/org/gtk/android/ToplevelActivity.java}

if [ ! -f "$JAVA" ]; then
    printf 'no ToplevelActivity.java at %s -- run `pixiewood generate` first\n' "$JAVA" >&2
    exit 1
fi

if grep -q 'protected void onNewIntent(Intent intent)' "$JAVA"; then
    printf 'already patched: %s\n' "$JAVA"
    exit 0
fi

ANCHOR='Logger.getLogger("Toplevel").log(Level.SEVERE, "Call to activate did not spawn a new window");'

if ! grep -qF "$ANCHOR" "$JAVA"; then
    printf 'expected to find `%s` in %s and did not.\n' "$ANCHOR" "$JAVA" >&2
    printf 'GTK has probably changed this code -- check whether the patch is still needed.\n' >&2
    exit 1
fi

# `-0777` slurps the whole file, so the pattern can span the two closing lines
# below the anchor: `});` for the `GlibContext.blockForMain` call, `}` for
# `onCreate` itself. The new method is inserted right after, at the same
# indentation as `onCreate` and its neighbours.
perl -0777 -pi -e '
    s/(Logger\.getLogger\("Toplevel"\)\.log\(Level\.SEVERE, "Call to activate did not spawn a new window"\);\n\t\t\}\);\n\t\}\n)/$1\n\t\@Override\n\tprotected void onNewIntent(Intent intent) {\n\t\tsuper.onNewIntent(intent);\n\t\tsetIntent(intent);\n\n\t\tif (intent.getData() == null)\n\t\t\treturn;\n\n\t\tString hint = "";\n\t\tif (intent.getAction() != null) {\n\t\t\tString[] action = intent.getAction().split("\\\\.");\n\t\t\thint = action[action.length - 1].toLowerCase();\n\t\t}\n\t\tfinal String finalHint = hint;\n\t\tGlibContext.blockForMain(() -> GdkContext.open(intent.getData(), finalHint));\n\t}\n/;
' "$JAVA"

if ! grep -q 'protected void onNewIntent(Intent intent)' "$JAVA"; then
    printf 'patch did not apply to %s\n' "$JAVA" >&2
    exit 1
fi

printf 'patched %s: onNewIntent delivers a redirect to a running Commune\n' "$JAVA"

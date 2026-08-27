#!/bin/sh
# Make sure the window shrinks when the keyboard it asked for arrives.
#
#     sh build-aux/android/patch-gtk-ime-insets.sh [ToplevelActivity.java path]
#
# Run this after `pixiewood prepare`, next to the other `patch-gtk-*`
# scripts. It writes into `subprojects/gtk`, which a re-extracted wrap
# loses.
#
# # What is wrong
#
# The resize plumbing exists end to end: `ToplevelView.onApplyWindowInsets`
# reads `WindowInsets.Type.ime()` along with the system bars, `onMeasure`
# subtracts the insets from the children, and the shrunken SurfaceView
# reaches GDK through `surfaceChanged`. But opening a room in Commune
# grabs the composer's focus while the room transition is still settling,
# `WindowInsetsController.show(ime())` runs mid-layout, and the inset
# application for the appearing keyboard can be missed: the keyboard is
# up, the window is full-height, and the composer is hidden behind it as
# if no keyboard were open.
#
# # The fix
#
# Two guarantees, both idempotent re-requests of what should have
# happened anyway:
#
# * A `WindowInsetsAnimation.Callback` on `ToplevelView` whose `onEnd`
#   calls `requestApplyInsets()` -- the end state of every inset
#   animation, the keyboard's entrance and exit included, is re-applied
#   once the animation settles, whatever the layout was doing when it
#   started.
# * `requestApplyInsets()` directly after `controller.show(ime())`, for
#   the case where the keyboard is already up (no animation will run) but
#   the view's inset state predates it.
#
# # When to delete this
#
# When the glue carries an inset-animation callback of its own.
set -eu

SOURCE=${1:-subprojects/gtk/gdk/android/glue/java/org/gtk/android/ToplevelActivity.java}

if [ ! -f "$SOURCE" ]; then
    printf 'no ToplevelActivity.java at %s -- run `pixiewood prepare` first\n' "$SOURCE" >&2
    exit 1
fi

if grep -q "WindowInsetsAnimation.Callback" "$SOURCE"; then
    printf 'inset animation callback already patched in %s\n' "$SOURCE"
    exit 0
fi

perl -0777 -pi -e '
    # 1. Imports.
    s/(import android\.view\.WindowInsets;\n)/$1import android.view.WindowInsetsAnimation;\n/
        or die "WindowInsets import anchor not found -- has the glue changed?\n";
    if (!/^import java\.util\.List;$/m) {
        s/(import android\.view\.WindowInsetsController;\n)/$1\nimport java.util.List;\n/
            or die "WindowInsetsController import anchor not found -- has the glue changed?\n";
    }

    # 2. The animation callback, registered in the ToplevelView constructor.
    my $ctor = quotemeta("\t\tpublic ToplevelView() {\n\t\t\tsuper(ToplevelActivity.this);\n");
    my $callback = <<'"'"'EOF'"'"';

\t\t\t/* The end state of every inset animation -- the keyboard
\t\t\t * arriving or leaving -- is re-applied once the animation
\t\t\t * settles. Without this, a keyboard shown while a layout
\t\t\t * transition is in flight can leave the window full-height
\t\t\t * underneath it. */
\t\t\tsetWindowInsetsAnimationCallback(new WindowInsetsAnimation.Callback(
\t\t\t\t\tWindowInsetsAnimation.Callback.DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
\t\t\t\t\@Override
\t\t\t\tpublic WindowInsets onProgress(\@NonNull WindowInsets insets,
\t\t\t\t\t\t\@NonNull List<WindowInsetsAnimation> running) {
\t\t\t\t\treturn insets;
\t\t\t\t}

\t\t\t\t\@Override
\t\t\t\tpublic void onEnd(\@NonNull WindowInsetsAnimation animation) {
\t\t\t\t\trequestApplyInsets();
\t\t\t\t}
\t\t\t});
EOF
    $callback =~ s/\\t/\t/g;
    $callback =~ s/\\\@/\@/g;
    s/($ctor)/$1$callback/
        or die "ToplevelView constructor anchor not found -- has the glue changed?\n";

    # 3. Re-request insets when showing a keyboard that may already be up.
    s/(\t*controller\.show\(WindowInsets\.Type\.ime\(\)\);\n)/$1\t\t\t\t\t\t\t\tToplevelView.this.requestApplyInsets();\n/
        or die "controller.show anchor not found -- has the glue changed?\n";
' "$SOURCE"

if ! grep -q "requestApplyInsets" "$SOURCE"; then
    printf 'patching the inset callback into %s did not take\n' "$SOURCE" >&2
    exit 1
fi

printf 'patched %s: the window follows the keyboard it asked for\n' "$SOURCE"

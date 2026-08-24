#!/bin/sh
# Build a pkg-config view for `cargo check --target x86_64-linux-android`.
#
# `cargo check` never links. A `-sys` crate's build script asks pkg-config
# whether a module exists and what version it is; it never opens the library.
# So the Rust side of the port can be compiled long before every C library has
# been cross-built, by combining the real `.pc` files meson wrote for the
# libraries that *have* been built with stubs for the ones that have not.
#
# This is honest for a compile check and dishonest for anything else: it proves
# nothing about linking, and nothing about those libraries existing on a device.
# Do not reach for it to make a build "work".
#
#     sh build-aux/android/pkgconfig-stubs.sh [output dir] [meson build dir]
#
# Then point cargo at the result:
#
#     export PKG_CONFIG_PATH=<output dir>
#     export PKG_CONFIG_LIBDIR=<output dir>   # replaces the default search path
#     export PKG_CONFIG_ALLOW_CROSS=1
#
# `PKG_CONFIG_LIBDIR` matters as much as `PKG_CONFIG_PATH`: it *replaces* the
# search path rather than extending it, so a host `.pc` from /usr cannot leak
# into an Android build and quietly satisfy a dependency.

set -eu

OUT=${1:-$HOME/android/commune-pc}
# Default to the libadwaita build: it is the widest real view available, because
# libadwaita pulls in GTK, which pulls in GLib, Pango, cairo and the rest, so one
# build tree describes almost everything Commune links against — and it makes
# `libadwaita-1` a real module rather than a stub. The S1 Rust spike
# (`~/src/gtk-android-rust-spike`, on the retired Ubuntu host) is the narrower
# alternative: GTK without libadwaita.
MESON_BUILD=${2:-$HOME/src/libadwaita/.pixiewood/bin-x86_64}
UNINSTALLED="$MESON_BUILD/meson-uninstalled"

rm -rf "$OUT"
mkdir -p "$OUT"

if [ -d "$UNINSTALLED" ]; then
    cp "$UNINSTALLED"/*.pc "$OUT"/
    printf 'real .pc files from %s: %s\n' "$UNINSTALLED" "$(ls "$OUT" | wc -l)"
else
    printf 'warning: no meson-uninstalled at %s, everything will be stubbed\n' "$UNINSTALLED"
fi

# stub <module> <version>
stub() {
    # Do not shadow a real .pc: a library that has actually been cross-built
    # should always win, or the stub hides a genuine regression.
    if [ -f "$OUT/$1.pc" ]; then
        printf 'keeping real %s\n' "$1"
        return
    fi
    cat > "$OUT/$1.pc" <<EOF
prefix=/nonexistent-android-stub
libdir=\${prefix}/lib
includedir=\${prefix}/include

Name: $1
Description: stub for cargo check -- checking does not link, so this is never used
Version: $2
Libs: -L\${libdir} -l$1
Cflags: -I\${includedir}
EOF
}

# Not cross-built yet. The versions are the ones `meson.build` asks for, or
# above, so that a version check cannot pass here and fail for real later.
stub libadwaita-1 1.9.1
stub gtksourceview-5 5.21.0
stub shumate-1.0 1.5.1
stub sqlite3 3.46.0
stub libwebp 1.4.0

for module in 1.0 app-1.0 audio-1.0 base-1.0 check-1.0 controller-1.0 net-1.0 \
              pbutils-1.0 play-1.0 rtp-1.0 sdp-1.0 tag-1.0 video-1.0 webrtc-1.0; do
    stub "gstreamer-$module" 1.28.0
done

# Only needed to check the *Linux* path from a machine without GTK development
# packages, which is how "Linux stays untouched" is verified here.
if [ "${WITH_LINUX_ONLY:-0}" = "1" ]; then
    stub glycin-2 2.2.0
    stub glycin-gtk4-2 2.2.0
    stub libglycin-2 2.2.0
    stub libglycin-gtk4-2 2.2.0
fi

printf 'total .pc files in %s: %s\n' "$OUT" "$(ls "$OUT" | wc -l)"

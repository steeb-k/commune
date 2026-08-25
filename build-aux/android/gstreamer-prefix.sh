#!/bin/sh
# Build a GStreamer prefix that Meson and Cargo can safely point pkg-config at.
#
#     sh build-aux/android/gstreamer-prefix.sh <extracted-tarball> <output>
#
# e.g. sh build-aux/android/gstreamer-prefix.sh ~/android/gstreamer \
#          ~/android/gst-android
#
# Run this once per machine, after extracting
# `gstreamer-1.0-android-universal-<version>.tar.xz` from
# <https://gstreamer.freedesktop.org/data/pkg/android/>. It is not part of the
# build; `meson.build` only reads the result. See `doc/android-media-plan.md`.
#
# # Why the tarball cannot be used directly
#
# The Cerbero binaries are a *complete* prefix. Besides GStreamer they carry
# their own GLib, GObject, GIO, cairo, freetype, harfbuzz, libpng, and so on —
# all of them static archives, all of them beside `libgstreamer-1.0.a` in one
# `lib/`. pixiewood builds its own copies of nearly all of the same libraries,
# as shared objects, and those are the ones the rest of the application is
# linked against.
#
# pkg-config resolves the *modules* correctly on its own: `-uninstalled.pc`
# files take priority over plain ones, so pixiewood wins `glib-2.0` even with
# the tarball on the search path. The link does not follow. `gstreamer-1.0.pc`
# emits `-L<tarball>/lib`, that `-L` lands ahead of pixiewood's, and the linker
# searches directories in order for *every* `-l` on the command line. So
# `-lglib-2.0` — asked for by pixiewood's own GLib .pc file — finds the
# tarball's `libglib-2.0.a` first.
#
# Measured, before this script existed: the link pulled GLib 2.82.4 out of the
# tarball instead of pixiewood's 2.89.4, and then failed on `libiconv_open`,
# because the tarball's GLib expects a standalone libiconv and Android's is in
# libc. A build that got past that would have been worse: two GLibs in one
# process.
#
# The fix is to give that `-L` nothing to shadow with. This prefix holds
# GStreamer's own libraries and nothing else.
#
# # What it contains
#
#   * `lib/libgst*.a`, symlinked. ~39 archives, the GStreamer libraries proper.
#   * `lib/gstreamer-1.0/`, symlinked. The plugins, for when they are wanted.
#   * `lib/liborc-0.4.a`, symlinked. A private dependency of the audio, video,
#     rtp and sdp libraries. Nothing in pixiewood is called `orc`, so it can sit
#     here without shadowing anything.
#   * `lib/pkgconfig/gst*.pc` and `orc-0.4.pc`, *copied* rather than symlinked.
#     These set `prefix=${pcfiledir}/../..`, and pkg-config resolves symlinks
#     before computing `pcfiledir` — a symlinked .pc file points back at the
#     tarball and undoes the whole exercise.
#   * `lib/pkgconfig/zlib.pc`, written here. Another private dependency, and the
#     one library in this list that Android itself provides: `libz.so` has been
#     an NDK API since API 1. Taking the system's costs nothing and adds no
#     `-L`, where the tarball's `libz.a` would shadow the one every other
#     library in the process is already using.
#   * `include/gstreamer-1.0`, symlinked. Deliberately not the tarball's whole
#     `include/`, which holds its GLib headers.
#
# What it does *not* contain is the tarball's GLib, GIO, cairo, or any other
# library pixiewood also builds. If a link ever needs one of those, the answer
# is pixiewood's, and it is already on the search path.
#
# See also `build-aux/android/gstreamer.c`, which reconciles the one thing this
# separation cannot: the tarball and pixiewood build different versions of
# proxy-libintl, whose symbols are named differently.
set -eu

src=${1:-}
dst=${2:-}
if [ -z "$src" ] || [ -z "$dst" ]; then
	echo "usage: $0 <extracted-tarball> <output-prefix>" >&2
	exit 2
fi

# The tarball names its architectures the way Android does; the output is named
# the way Meson does, so `meson.build` can reach it with `cpu_family()` alone.
found=0
for pair in x86_64:x86_64 arm64:aarch64 armv7:arm x86:x86; do
	arch=${pair%%:*}
	out=${pair#*:}
	[ -d "$src/$arch/lib" ] || continue
	found=$((found + 1))

	rm -rf "$dst/$out"
	mkdir -p "$dst/$out/lib/pkgconfig" "$dst/$out/include"

	ln -s "$src/$arch/include/gstreamer-1.0" "$dst/$out/include/gstreamer-1.0"
	ln -s "$src/$arch/lib/gstreamer-1.0" "$dst/$out/lib/gstreamer-1.0"

	for lib in "$src/$arch"/lib/libgst*.a "$src/$arch"/lib/liborc-0.4.a; do
		[ -f "$lib" ] || continue
		ln -s "$lib" "$dst/$out/lib/$(basename "$lib")"
	done

	for pc in "$src/$arch"/lib/pkgconfig/gst*.pc "$src/$arch"/lib/pkgconfig/orc-0.4.pc; do
		[ -f "$pc" ] || continue
		cp "$pc" "$dst/$out/lib/pkgconfig/$(basename "$pc")"
	done

	# The NDK's, not the tarball's. No `-L`, so nothing to shadow.
	cat > "$dst/$out/lib/pkgconfig/zlib.pc" <<'PC'
Name: zlib
Description: Android's libz, an NDK API since API 1
Version: 1.2.11
Libs: -lz
PC

	echo "$out: $(ls "$dst/$out"/lib/*.a | wc -l) archives, $(ls "$dst/$out"/lib/pkgconfig/*.pc | wc -l) pkg-config files"
done

if [ "$found" -eq 0 ]; then
	echo "$0: no architecture directories under $src" >&2
	exit 1
fi

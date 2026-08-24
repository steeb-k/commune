#!/usr/bin/env bash
# Assemble a relocatable Commune folder out of a configured Meson build directory.
#
#   build-aux/windows/bundle.sh --build-dir _build --out-dir _build/windows
#
# Or, from Meson, which fills in everything it already knows:
#
#   meson compile -C _build windows-bundle
#
# Run it from an MSYS2 UCRT64 shell. See doc/windows.md.
#
# The layout is the one GLib already expects, which is why there is nothing here
# corresponding to the environment variables the macOS bundle has to set. On
# Windows GLib works out where it was installed by asking the loader where
# `libglib-2.0-0.dll` came from and taking the parent of its directory, and
# GdkPixbuf, GIO and GStreamer all follow the same convention. So a tree of
#
#   Commune/bin/*.dll   Commune/lib/...   Commune/share/...
#
# relocates itself: move the folder and every one of those libraries finds its
# own data again, with nothing set and nothing rewritten.
#
# What has to be done by hand is the closure of DLLs — Windows has no rpath, and
# an executable finds its imports by name in its own directory — and the
# gdk-pixbuf loader cache, which stores absolute paths.
#
# The `ntldd` audit at the end is what proves it worked: it walks everything in
# the bundle and fails if any import resolves outside it or outside the system
# directory. That audit is the point of the script, not a formality; without it
# a bundle that works on this machine because MSYS2 is on PATH is
# indistinguishable from one that works anywhere.
set -euo pipefail

BUILD_DIR=''
OUT_DIR=''
APP_NAME='Commune'
VERSION=''
PROFILE=''
EXECUTABLE='commune'
WANT_ZIP=0
WANT_STRIP=1

while [ $# -gt 0 ]; do
    case "$1" in
    --build-dir) BUILD_DIR=$2 && shift 2 ;;
    --out-dir) OUT_DIR=$2 && shift 2 ;;
    --app-id) shift 2 ;; # Accepted for symmetry with the macOS bundler.
    --app-name) APP_NAME=$2 && shift 2 ;;
    --version) VERSION=$2 && shift 2 ;;
    --profile) PROFILE=$2 && shift 2 ;;
    --executable) EXECUTABLE=$2 && shift 2 ;;
    --zip) WANT_ZIP=1 && shift ;;
    --no-strip) WANT_STRIP=0 && shift ;;
    *)
        echo "bundle: unknown argument: $1" >&2
        exit 2
        ;;
    esac
done

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

[ -n "$BUILD_DIR" ] || BUILD_DIR="$ROOT/_build"
[ -d "$BUILD_DIR" ] || {
    echo "bundle: no such build directory: $BUILD_DIR" >&2
    exit 1
}
BUILD_DIR="$(cd "$BUILD_DIR" && pwd)"
[ -n "$OUT_DIR" ] || OUT_DIR="$BUILD_DIR/windows"

PREFIX="${MINGW_PREFIX:-/ucrt64}"
[ -d "$PREFIX/bin" ] || {
    echo "bundle: no MSYS2 prefix at $PREFIX — run this from a UCRT64 shell" >&2
    exit 1
}

command -v ntldd >/dev/null 2>&1 || {
    echo "bundle: ntldd is not installed (pacman -S mingw-w64-ucrt-x86_64-ntldd)" >&2
    exit 1
}

# The name a user sees. A development build says so, so that two of them can sit
# side by side without either being mistaken for the other.
BUNDLE_NAME="$APP_NAME"
if [ -n "$PROFILE" ] && [ "$PROFILE" != 'Stable' ]; then
    BUNDLE_NAME="$APP_NAME $PROFILE"
fi

BUNDLE="$OUT_DIR/$BUNDLE_NAME"
STAGE="$BUILD_DIR/windows-stage"

echo "bundle: assembling $BUNDLE"
rm -rf "$BUNDLE" "$STAGE"
mkdir -p "$BUNDLE/bin" "$OUT_DIR"

################################################################################
# 1. Our own files, from a staged install.
################################################################################

# `meson install --destdir` writes the whole prefix under the staging directory,
# with the drive letter dropped, so the result is somewhere like
# `stage/msys64/ucrt64`. Rather than reconstruct that path, find the executable
# and work back from it — that keeps this correct whatever the prefix is.
meson install -C "$BUILD_DIR" --destdir "$STAGE" --quiet

staged_exe="$(find "$STAGE" -name "$EXECUTABLE.exe" -type f | head -n 1)"
[ -n "$staged_exe" ] || {
    echo "bundle: no $EXECUTABLE.exe in the staged install at $STAGE" >&2
    exit 1
}
staged_prefix="$(cd "$(dirname "$staged_exe")/.." && pwd)"
echo "bundle: staged install at $staged_prefix"

cp "$staged_exe" "$BUNDLE/bin/"

# Drop the debug sections from the copy that ships.
#
# This is not a nicety. `x86_64-pc-windows-gnu` emits DWARF into the PE, and the
# release profile asks for debug information on purpose, so the binary arrives
# here at well over a gigabyte — of which the program is about forty megabytes.
# Nobody is downloading that, and every tool that has to read the file, ntldd
# and the installer included, pays for it.
#
# `--strip-debug` rather than a full strip: it removes the DWARF while leaving
# the symbol table, so a panic still names the functions in its backtrace. What
# is lost is the file and line beside each frame. The unstripped binary stays in
# the build directory, at `_build/cargo-target/*/commune.exe`, which is what to
# reach for when a backtrace needs to be read properly. `--no-strip` keeps it
# whole here too.
if [ "$WANT_STRIP" = 1 ] && command -v strip >/dev/null 2>&1; then
    before_bytes="$(stat -c %s "$BUNDLE/bin/$EXECUTABLE.exe")"
    strip --strip-debug "$BUNDLE/bin/$EXECUTABLE.exe"
    after_bytes="$(stat -c %s "$BUNDLE/bin/$EXECUTABLE.exe")"
    echo "bundle: stripped $EXECUTABLE.exe, $((before_bytes / 1048576))M -> $((after_bytes / 1048576))M"
fi

mkdir -p "$BUNDLE/share"
cp -r "$staged_prefix/share/$EXECUTABLE" "$BUNDLE/share/"
cp -r "$staged_prefix/share/locale" "$BUNDLE/share/"

################################################################################
# 2. The GSettings schemas.
################################################################################

# The app aborts at startup without its own schema, and GTK and libadwaita read
# theirs. `glib-compile-schemas` produces one cache for all of them, so ours is
# compiled together with the prefix's rather than merged afterwards.
schemas="$BUNDLE/share/glib-2.0/schemas"
mkdir -p "$schemas"
cp "$PREFIX/share/glib-2.0/schemas"/*.xml "$schemas/" 2>/dev/null || true
cp "$staged_prefix/share/glib-2.0/schemas"/*.xml "$schemas/"
glib-compile-schemas "$schemas" >/dev/null
# The sources are only needed to compile the cache; the cache is what is read.
rm -f "$schemas"/*.xml

################################################################################
# 3. Icons, and the rest of the shared data.
################################################################################

for theme in Adwaita hicolor; do
    mkdir -p "$BUNDLE/share/icons/$theme"
    cp -r "$PREFIX/share/icons/$theme/." "$BUNDLE/share/icons/$theme/"
done
# Our own application icon went into hicolor in the staged install.
cp -r "$staged_prefix/share/icons/hicolor/." "$BUNDLE/share/icons/hicolor/"

cp -r "$PREFIX/share/gtksourceview-5" "$BUNDLE/share/"

# Used to work out the type of a file that is being sent or received.
mkdir -p "$BUNDLE/share/mime"
cp "$PREFIX/share/mime/mime.cache" "$BUNDLE/share/mime/"

################################################################################
# 4. The loadable modules.
################################################################################

# GdkPixbuf's loaders. SVG is the one that has to be here: the whole symbolic
# icon set is SVG, and without librsvg every icon in the application fails to
# load. PNG and JPEG are compiled into gdk-pixbuf itself and are not modules.
pixbuf_dir='lib/gdk-pixbuf-2.0/2.10.0'
mkdir -p "$BUNDLE/$pixbuf_dir/loaders"
cp "$PREFIX/$pixbuf_dir/loaders"/*.dll "$BUNDLE/$pixbuf_dir/loaders/"

# GIO's TLS backend. Every HTTPS request fails without it, and it fails by
# looking like a network problem rather than a missing module.
mkdir -p "$BUNDLE/lib/gio/modules"
cp "$PREFIX/lib/gio/modules"/*.dll "$BUNDLE/lib/gio/modules/"

# GStreamer plugins. The list is the macOS one with the platform elements
# swapped: `wasapi2` for `osxaudio`, `mediafoundation` and `d3d12` for
# `applemedia`. The last line is the call — `webrtc` is webrtcbin itself, `nice`
# the ICE agent it drives, and `srtp` holds the srtpenc that `dtls` builds
# inside dtlssrtpenc. None of them announce themselves as missing until a call
# is placed, because webrtcbin is made at runtime by name like every other
# element, so a missing one is found by a failed call rather than by a failed
# build. Hence the hard failure below.
GST_PLUGINS="
coreelements typefindfunctions playback app gio autodetect
audioconvert audioresample audiorate audioparsers volume level
videoconvertscale videorate videoparsersbad deinterlace
wasapi2 gtk4 mediafoundation d3d12 opengl
isomp4 matroska ogg wavparse subparse
opus opusparse vorbis mpg123 vpx png jpeg alaw mulaw
codecalpha codectimestamper id3demux apetag icydemux soup
webrtc nice srtp dtls rtp rtpmanager
"
mkdir -p "$BUNDLE/lib/gstreamer-1.0"
missing_plugins=''
for plugin in $GST_PLUGINS; do
    src="$PREFIX/lib/gstreamer-1.0/libgst$plugin.dll"
    if [ -f "$src" ]; then
        cp "$src" "$BUNDLE/lib/gstreamer-1.0/"
    else
        missing_plugins="$missing_plugins $plugin"
    fi
done
if [ -n "$missing_plugins" ]; then
    echo "bundle: these GStreamer plugins are not in $PREFIX/lib/gstreamer-1.0:" >&2
    for plugin in $missing_plugins; do
        echo "  libgst$plugin.dll" >&2
    done
    exit 1
fi

# GStreamer introspects its plugins by running this helper as a subprocess.
mkdir -p "$BUNDLE/lib/gstreamer-1.0"
cp "$PREFIX/bin/gst-plugin-scanner.exe" "$BUNDLE/bin/" 2>/dev/null ||
    cp "$PREFIX/libexec/gstreamer-1.0/gst-plugin-scanner.exe" "$BUNDLE/bin/"

################################################################################
# 5. The DLL closure.
################################################################################

# Windows resolves an import by name, looking first in the directory the module
# was loaded from. So every non-system DLL any of our binaries needs — directly
# or through another DLL — has to sit beside the executable. `ntldd -R` walks
# that graph; anything it resolves inside the MSYS2 prefix is ours to carry, and
# anything under the system directory is Windows' own.
echo 'bundle: resolving the DLL closure'

# ntldd prints native paths — `C:\msys64\ucrt64\bin\…` — with a carriage return
# at the end of every line. Both have to go before anything can be matched
# against a prefix or anchored with `$`, so every reader of its output runs it
# through this first.
normalize_ntldd() {
    tr -d '\r' | tr '\\' '/'
}

prefix_win="$(cygpath -m "$PREFIX")"
# Windows is case-insensitive about paths and ntldd does not promise a case, so
# compare in lower case.
prefix_lower="$(printf '%s' "$prefix_win" | tr '[:upper:]' '[:lower:]')"

# Is this path one the MSYS2 prefix gave us, and so ours to carry?
from_prefix() {
    case "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" in
    "$prefix_lower"/*) return 0 ;;
    *) return 1 ;;
    esac
}

# Direct imports only, not `ntldd -R`. The loop below runs to a fixed point, so
# it reaches the whole graph anyway — and it does so without re-walking the same
# subtrees once per binary, which on a bundle this size is the difference
# between seconds and many minutes.
collect_deps() {
    # Print the resolved dependencies of one binary, one path per line.
    ntldd "$1" 2>/dev/null |
        normalize_ntldd |
        sed -n 's/^.*=> \(.*\) (0x[0-9a-fA-F]*)$/\1/p'
}

# Every binary in the bundle is a root of the graph, not just the executable:
# the plugins pull in libraries of their own that nothing else references.
#
# A worklist rather than repeated passes over everything. Each binary is walked
# exactly once, when it first arrives, so the cost is one ntldd per file in the
# bundle rather than one per file per layer of the dependency graph — the
# difference between half a minute and the better part of an hour.
worklist="$(mktemp)"
seen="$(mktemp)"
trap 'rm -f "$worklist" "$seen"' EXIT

find "$BUNDLE" -name '*.dll' -o -name '*.exe' >"$worklist"

walked=0
while [ -s "$worklist" ]; do
    binary="$(head -n 1 "$worklist")"
    sed -i '1d' "$worklist"

    key="$(basename "$binary" | tr '[:upper:]' '[:lower:]')"
    grep -qxF "$key" "$seen" && continue
    printf '%s\n' "$key" >>"$seen"

    while IFS= read -r dep; do
        [ -n "$dep" ] || continue
        from_prefix "$dep" || continue

        name="$(basename "$dep")"
        [ -f "$BUNDLE/bin/$name" ] && continue

        cp "$dep" "$BUNDLE/bin/$name"
        # Newly arrived, so its own imports have not been looked at yet.
        printf '%s\n' "$BUNDLE/bin/$name" >>"$worklist"
    done <<EOF
$(collect_deps "$binary")
EOF

    walked=$((walked + 1))
done

echo "bundle:   walked $walked binaries, $(find "$BUNDLE/bin" -name '*.dll' | wc -l) DLLs carried"

################################################################################
# 6. The gdk-pixbuf loader cache.
################################################################################

# The cache the query tool writes names each loader by absolute path, which
# would point back into MSYS2. Written from inside the bundle and then made
# relative, it names them the way GdkPixbuf resolves them against its own
# installation directory.
echo 'bundle: writing the gdk-pixbuf loader cache'
(
    cd "$BUNDLE"
    GDK_PIXBUF_MODULEDIR="$BUNDLE/$pixbuf_dir/loaders" \
        gdk-pixbuf-query-loaders >"$BUNDLE/$pixbuf_dir/loaders.cache"
)
# Rewrite the absolute paths to ones relative to the bundle.
bundle_win="$(cygpath -m "$BUNDLE")"
sed -i "s|$bundle_win/||g; s|$BUNDLE/||g" "$BUNDLE/$pixbuf_dir/loaders.cache"

grep -q '"svg"' "$BUNDLE/$pixbuf_dir/loaders.cache" || {
    echo 'bundle: the loader cache has no svg loader, so every icon would fail' >&2
    exit 1
}

################################################################################
# 7. The audit.
################################################################################

# This is what proves the bundle is self-contained: an import that resolves back
# into MSYS2 is a DLL the closure walk missed, and it works on this machine and
# fails on any other.
#
# What "not found" means takes a little care. Much of it is Windows' own API
# sets — the `api-ms-win-*` and `ext-ms-win-*` names — which are virtual: the
# loader resolves them through a schema and there is no file to find, so ntldd
# reports every one of them as missing. Delay-loaded system components do the
# same. Rather than keep a list of names to forgive, ask the question that
# actually matters: is this something the prefix could have given us? If the
# name is in the prefix, we failed to bundle it. If it is not, it is Windows'
# to provide and never ours.
echo 'bundle: auditing'
audit_failed=0
while IFS= read -r binary; do
    while IFS= read -r line; do
        case "$line" in
        *'not found'*)
            name="$(printf '%s' "$line" | sed -n 's/^[[:space:]]*\([^[:space:]]*\)[[:space:]]*=>.*$/\1/p')"
            if [ -n "$name" ] && [ -f "$PREFIX/bin/$name" ]; then
                echo "bundle: MISSING from the bundle, in $(basename "$binary"): $name" >&2
                audit_failed=1
            fi
            ;;
        *'=>'*)
            resolved="$(printf '%s' "$line" |
                sed -n 's/^.*=> \(.*\) (0x[0-9a-fA-F]*)$/\1/p')"
            if [ -n "$resolved" ] && from_prefix "$resolved"; then
                echo "bundle: ESCAPES to the prefix in $(basename "$binary"): $line" >&2
                audit_failed=1
            fi
            ;;
        esac
    done <<EOF
$(cd "$BUNDLE/bin" && ntldd "$binary" 2>/dev/null | normalize_ntldd)
EOF
done <<EOF
$(find "$BUNDLE" -name '*.dll' -o -name '*.exe')
EOF

if [ "$audit_failed" != 0 ]; then
    echo 'bundle: the audit failed; the bundle is not self-contained' >&2
    exit 1
fi

rm -rf "$STAGE"

size="$(du -sh "$BUNDLE" | cut -f 1)"
count="$(find "$BUNDLE" -type f | wc -l)"
echo "bundle: $BUNDLE ($size, $count files)"

################################################################################
# 8. The archive.
################################################################################

if [ "$WANT_ZIP" = 1 ]; then
    archive="$OUT_DIR/$BUNDLE_NAME${VERSION:+-$VERSION}.zip"
    echo "bundle: writing $archive"
    rm -f "$archive"
    (cd "$OUT_DIR" && zip -qr "$(basename "$archive")" "$BUNDLE_NAME")
    echo "bundle: $archive"
fi

#!/usr/bin/env bash
# Assemble a relocatable Commune.app out of a configured Meson build directory.
#
#   build-aux/macos/bundle.sh --build-dir _build --out-dir _build/macos
#
# Or, from Meson, which fills in everything it already knows:
#
#   meson compile -C _build macos-bundle
#
# The layout this produces is not a matter of taste: `src/utils/app_bundle.rs`
# reads it back at startup, and the two have to agree. It looks for a small Unix
# prefix under `Contents/Resources` and points GLib, GdkPixbuf, GStreamer and
# fontconfig at it through environment variables it sets before any of them are
# initialized.
#
# There is no dylibbundler here, deliberately. Every dylib conda-forge builds
# already has an `@rpath/<basename>` install name, so bundling is a flat copy
# into `Contents/Frameworks` plus one `LC_RPATH` per Mach-O file — no per-
# dependency rewriting, and no dependency on a tool that is not otherwise part
# of this environment. The `otool -L` audit at the end is what proves it worked.
#
# See doc/macos.md.
set -euo pipefail

BUILD_DIR=''
OUT_DIR=''
PREFIX=''
APP_ID=''
APP_NAME=''
VERSION=''
PROFILE=''
EXECUTABLE='commune'
WANT_DMG=0
WANT_TARBALL=0

while [ $# -gt 0 ]; do
    case "$1" in
    --build-dir) BUILD_DIR=$2 && shift 2 ;;
    --out-dir) OUT_DIR=$2 && shift 2 ;;
    --prefix) PREFIX=$2 && shift 2 ;;
    --app-id) APP_ID=$2 && shift 2 ;;
    --app-name) APP_NAME=$2 && shift 2 ;;
    --version) VERSION=$2 && shift 2 ;;
    --profile) PROFILE=$2 && shift 2 ;;
    --executable) EXECUTABLE=$2 && shift 2 ;;
    --dmg) WANT_DMG=1 && shift ;;
    --tarball) WANT_TARBALL=1 && shift ;;
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
    echo "  run: meson setup _build -Dprofile=development" >&2
    exit 1
}
BUILD_DIR="$(cd "$BUILD_DIR" && pwd)"
[ -n "$OUT_DIR" ] || OUT_DIR="$BUILD_DIR/macos"

# Everything is discovered through pkg-config, so the script does not care
# whether the GTK stack came from conda-forge, MacPorts or somewhere else --
# though doc/macos.md explains why it should be conda-forge.
if [ -z "$PREFIX" ]; then
    PREFIX="$(pkg-config --variable=prefix gtk4 2>/dev/null || true)"
fi
[ -n "$PREFIX" ] && [ -d "$PREFIX/lib" ] || {
    echo "bundle: cannot find the GTK prefix." >&2
    echo "  export PKG_CONFIG_PATH and PKG_CONFIG_LIBDIR, or pass --prefix." >&2
    exit 1
}

[ -n "$APP_ID" ] || APP_ID='io.github.steeb_k.Commune'
[ -n "$VERSION" ] || VERSION='1.rc1'
[ -n "$PROFILE" ] || PROFILE='Stable'
# A stable and a development build have to be able to sit in /Applications at
# the same time, and two bundles cannot share a directory name. On Linux the
# profiles are told apart by their application ID and their icon, but here the
# name is what the user sees in the Dock and in the menu bar.
if [ -z "$APP_NAME" ]; then
    case "$PROFILE" in
    Stable) APP_NAME='Commune' ;;
    *) APP_NAME="Commune $PROFILE" ;;
    esac
fi

for tool in otool install_name_tool codesign glib-compile-schemas gdk-pixbuf-query-loaders; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "bundle: $tool not found on PATH." >&2
        exit 1
    }
done

note() { printf 'bundle: %s\n' "$*"; }

APP="$OUT_DIR/$APP_NAME.app"
CONTENTS="$APP/Contents"
FRAMEWORKS="$CONTENTS/Frameworks"
RES="$CONTENTS/Resources"
WORK="$OUT_DIR/.stage"

rm -rf "$APP" "$WORK"
mkdir -p "$CONTENTS/MacOS" "$FRAMEWORKS" "$RES" "$WORK"

# ---------------------------------------------------------------------------
# The app's own files, from a staged `meson install`
# ---------------------------------------------------------------------------

note "installing $BUILD_DIR into a staging tree"
DESTDIR="$WORK" meson install -C "$BUILD_DIR" --quiet >/dev/null

# Meson's prefix is wherever this build was configured to install, so find the
# binary rather than assuming it.
STAGED_BIN="$(find "$WORK" -type f -name "$EXECUTABLE" -path '*/bin/*' | head -1)"
[ -n "$STAGED_BIN" ] || {
    echo "bundle: no $EXECUTABLE in the staged install under $WORK" >&2
    exit 1
}
STAGED="$(cd "$(dirname "$(dirname "$STAGED_BIN")")" && pwd)"

cp "$STAGED_BIN" "$CONTENTS/MacOS/$EXECUTABLE"
chmod u+w "$CONTENTS/MacOS/$EXECUTABLE"

mkdir -p "$RES/share"
cp -R "$STAGED/share/$EXECUTABLE" "$RES/share/"
if [ -d "$STAGED/share/locale" ]; then
    cp -R "$STAGED/share/locale" "$RES/share/"
fi

# ---------------------------------------------------------------------------
# Data the GTK stack reads from the prefix
# ---------------------------------------------------------------------------

note 'copying icon themes, schemas and language definitions'

mkdir -p "$RES/share/icons"
for theme in Adwaita hicolor; do
    if [ -d "$PREFIX/share/icons/$theme" ]; then
        cp -R "$PREFIX/share/icons/$theme" "$RES/share/icons/"
    fi
done
# The app's own icon, on top of the prefix's hicolor index.theme.
if [ -d "$STAGED/share/icons/hicolor" ]; then
    cp -R "$STAGED/share/icons/hicolor" "$RES/share/icons/"
fi
chmod -R u+w "$RES/share/icons"

if [ -d "$PREFIX/share/gtksourceview-5" ]; then
    cp -R "$PREFIX/share/gtksourceview-5" "$RES/share/"
fi
# shared-mime-info is not packaged by conda-forge, so this is usually absent.
if [ -f "$PREFIX/share/mime/mime.cache" ]; then
    mkdir -p "$RES/share/mime"
    cp "$PREFIX/share/mime/mime.cache" "$RES/share/mime/"
fi

# GTK's, GLib's and libadwaita's own translations, so the file chooser and the
# stock dialogs are not left in English. The prefix's share/locale is 35M of
# catalogues for everything conda-forge installed, almost none of which this app
# can reach, so take only the domains that are actually on screen.
for domain in gtk40 gtk40-properties glib20 gsettings-desktop-schemas libadwaita gtksourceview-5; do
    for mo in "$PREFIX"/share/locale/*/LC_MESSAGES/"$domain".mo; do
        [ -f "$mo" ] || continue
        lang_dir="$RES/share/locale/$(basename "$(dirname "$(dirname "$mo")")")/LC_MESSAGES"
        mkdir -p "$lang_dir"
        cp "$mo" "$lang_dir/"
    done
done

# GSettings schemas. Two sources have to end up in one compiled file: the app's
# own schema, which src/application.rs aborts without, and GTK's, which the file
# chooser and the emoji chooser need. There is no way to merge two compiled
# files, so collect the XML and compile once.
SCHEMA_DIR="$RES/share/glib-2.0/schemas"
mkdir -p "$SCHEMA_DIR"
for dir in "$PREFIX/share/glib-2.0/schemas" "$STAGED/share/glib-2.0/schemas"; do
    [ -d "$dir" ] || continue
    for f in "$dir"/*.gschema.xml "$dir"/*.enums.xml; do
        if [ -f "$f" ]; then
            cp "$f" "$SCHEMA_DIR/"
        fi
    done
done
glib-compile-schemas "$SCHEMA_DIR"
rm -f "$SCHEMA_DIR"/*.xml
[ -f "$SCHEMA_DIR/gschemas.compiled" ] || {
    echo "bundle: glib-compile-schemas produced no gschemas.compiled" >&2
    exit 1
}

# fontconfig. GTK links it, so it is bundled whether we like it or not, and the
# path to a configuration file is compiled into the library -- pointing at this
# machine's prefix, which will not exist anywhere else. Copy the configuration
# in and drop the one absolute cache directory it names; the `prefix="xdg"` and
# `~/.fontconfig` entries below it are already relocatable.
#
# The copy dereferences symlinks. Every file in `conf.d` is a link to
# `../../../share/fontconfig/conf.avail/`, which is outside the directory being
# copied, so preserving the links would put two dozen dangling symlinks in the
# bundle and quietly lose most of the font configuration.
if [ -f "$PREFIX/etc/fonts/fonts.conf" ]; then
    mkdir -p "$RES/etc"
    cp -RL "$PREFIX/etc/fonts" "$RES/etc/"
    chmod -R u+w "$RES/etc/fonts"
    sed -i '' "\#<cachedir>$PREFIX/#d" "$RES/etc/fonts/fonts.conf"
    # Documentation for whoever packaged fontconfig, which names the prefix it
    # was built in and is the one thing left that the audit would report.
    rm -f "$RES/etc/fonts/conf.d/README"
fi

# ---------------------------------------------------------------------------
# Loadable modules
# ---------------------------------------------------------------------------

note 'copying gdk-pixbuf loaders, gio modules and GStreamer plugins'

PIXBUF_DIR="$RES/lib/gdk-pixbuf-2.0/2.10.0"
mkdir -p "$PIXBUF_DIR/loaders"
cp "$PREFIX"/lib/gdk-pixbuf-2.0/2.10.0/loaders/* "$PIXBUF_DIR/loaders/"

# The cache is generated against the *prefix*, never against the bundle. Run
# against the prefix, gdk-pixbuf-query-loaders emits paths relative to it, which
# resolve against whatever directory the cache is found in -- so the same file
# works from Contents/Resources wherever the user drags the app. Run against a
# staging directory it emits absolute paths, which would pin the bundle to this
# machine.
#
# Both extensions are passed explicitly: the librsvg loader installs as a
# `.dylib` while every other loader is a `.so`, and letting the tool scan finds
# only the `.so` files. Without the SVG loader every symbolic icon fails to
# load, which is most of the user interface.
LOADERS="$PREFIX/lib/gdk-pixbuf-2.0/2.10.0/loaders"
gdk-pixbuf-query-loaders "$LOADERS"/*.so "$LOADERS"/*.dylib >"$PIXBUF_DIR/loaders.cache"
# Only the module-path lines, which are the ones ending in .so" or .dylib".
# Matching a bare leading quote-slash would also catch `"/*"`, which is the
# magic signature the XPM loader recognises files by.
if grep -qE '^"/.*\.(so|dylib)"$' "$PIXBUF_DIR/loaders.cache"; then
    echo "bundle: gdk-pixbuf-query-loaders wrote absolute paths; the bundle would not relocate." >&2
    exit 1
fi
grep -q 'loader_svg\|loader-svg' "$PIXBUF_DIR/loaders.cache" || {
    echo "bundle: no SVG loader in loaders.cache -- is librsvg installed in $PREFIX?" >&2
    exit 1
}

# GIO's TLS backend is a module. Without it every https request fails, which
# means there is no login at all.
mkdir -p "$RES/lib/gio/modules"
cp "$PREFIX"/lib/gio/modules/*.so "$RES/lib/gio/modules/" 2>/dev/null || true
if ! ls "$RES"/lib/gio/modules/*.so >/dev/null 2>&1; then
    echo "bundle: no gio modules in $PREFIX/lib/gio/modules -- glib-networking missing?" >&2
    exit 1
fi

# The GStreamer plugins Commune can actually reach. The full set is 178 plugins
# for decklink capture cards, JACK, festival speech synthesis and the like; the
# cost of carrying them is not the plugins but the dylibs they would drag into
# Frameworks behind them.
#
# The last line is the call. `webrtc` is webrtcbin itself, `nice` is the ICE
# agent it drives, and `srtp` holds the srtpenc that `dtls` builds inside
# dtlssrtpenc — none of which announce themselves as missing until a call is
# placed, because webrtcbin is made at runtime by name like every other
# element. `rtp` is the payloaders on either end of the pipeline, and
# `rtpmanager` the jitter buffers webrtcbin puts behind them. Four of the six
# are built from source by setup-conda-macos.sh; a bundle attempted without
# that step fails here rather than shipping a client whose calls do not work.
GST_PLUGINS="
coreelements typefindfunctions playback app gio autodetect
audioconvert audioresample audiorate audioparsers volume level
videoconvertscale videorate videoparsersbad deinterlace
osxaudio gtk4 applemedia opengl
isomp4 matroska ogg wavparse subparse
opus opusparse vorbis mpg123 vpx png jpeg alaw mulaw
codecalpha codectimestamper id3demux apetag icydemux soup
webrtc nice srtp dtls rtp rtpmanager
"
mkdir -p "$RES/lib/gstreamer-1.0"
for plugin in $GST_PLUGINS; do
    src="$PREFIX/lib/gstreamer-1.0/libgst$plugin.dylib"
    if [ -f "$src" ]; then
        cp "$src" "$RES/lib/gstreamer-1.0/"
    else
        echo "bundle: GStreamer plugin '$plugin' is not in $PREFIX/lib/gstreamer-1.0" >&2
        exit 1
    fi
done

# GStreamer introspects its plugins by running this helper as a subprocess.
mkdir -p "$RES/libexec/gstreamer-1.0"
cp "$PREFIX/libexec/gstreamer-1.0/gst-plugin-scanner" "$RES/libexec/gstreamer-1.0/"

chmod -R u+w "$RES/lib" "$RES/libexec"

# ---------------------------------------------------------------------------
# The dylib walk
# ---------------------------------------------------------------------------

# Every Mach-O file that the walk has to look inside. Frameworks fills up as it
# goes, and each new arrival is appended to this list, so the loop below ends
# only when nothing new has been found.
SCAN="$WORK/scan-list"
: >"$SCAN"

for f in "$CONTENTS/MacOS/$EXECUTABLE" \
    "$RES"/lib/gstreamer-1.0/*.dylib \
    "$PIXBUF_DIR"/loaders/* \
    "$RES"/lib/gio/modules/*.so \
    "$RES/libexec/gstreamer-1.0/gst-plugin-scanner"; do
    if [ -f "$f" ]; then
        printf '%s\n' "$f" >>"$SCAN"
    fi
done

# The install name a dependency was recorded under, turned into a file to copy.
resolve_dep() { # <dependency> <the file that named it>
    case "$1" in
    @rpath/*) printf '%s\n' "$PREFIX/lib/${1#@rpath/}" ;;
    @loader_path/*) printf '%s\n' "$(dirname "$2")/${1#@loader_path/}" ;;
    @executable_path/*) printf '%s\n' "$(dirname "$2")/${1#@executable_path/}" ;;
    *) printf '%s\n' "$1" ;;
    esac
}

note 'walking dependencies into Contents/Frameworks'

line=1
while true; do
    file="$(sed -n "${line}p" "$SCAN")"
    [ -n "$file" ] || break
    line=$((line + 1))

    self="$(basename "$file")"
    # `otool -L` prints the file's own install name first when it is a dylib.
    # The list goes through a file rather than a pipe so that the loop runs in
    # this shell: a `while read` on the right of a pipe is a subshell, and a
    # missing dependency could only abort that subshell, not the bundle.
    otool -L "$file" | tail -n +2 | awk '{print $1}' >"$WORK/deps"

    while read -r dep; do
        case "$dep" in
        /usr/lib/* | /System/*) continue ;;
        esac

        base="$(basename "$dep")"
        [ "$base" = "$self" ] && continue
        [ -f "$FRAMEWORKS/$base" ] && continue

        src="$(resolve_dep "$dep" "$file")"
        if [ ! -f "$src" ]; then
            echo "bundle: cannot find $dep, named by $file" >&2
            exit 1
        fi

        cp "$src" "$FRAMEWORKS/$base"
        chmod u+w "$FRAMEWORKS/$base"
        printf '%s\n' "$FRAMEWORKS/$base" >>"$SCAN"
    done <"$WORK/deps"
done

note "$(find "$FRAMEWORKS" -name '*.dylib' | wc -l | tr -d ' ') libraries in Frameworks"

# ---------------------------------------------------------------------------
# Install names and rpaths
# ---------------------------------------------------------------------------

# Where Contents/Frameworks is, as seen from the directory a given file sits in.
# @loader_path works in a plain executable too, where it means the same thing as
# @executable_path, so one rule covers the binary, the libraries, the plugins
# and the out-of-process plugin scanner alike.
rel_to_frameworks() { # <file inside Contents>
    _dir="$(dirname "${1#"$CONTENTS/"}")"
    _up=''
    while [ "$_dir" != '.' ]; do
        _up="../$_up"
        _dir="$(dirname "$_dir")"
    done
    printf '%s%s\n' "$_up" 'Frameworks'
}

has_rpath() { # <file> <rpath>
    otool -l "$1" | awk '/LC_RPATH/{f=1} f&&/path /{print $2; f=0}' |
        grep -qxF "$2"
}

note 'setting install names and rpaths'

# Everything Mach-O in the bundle, now that Frameworks is populated.
ALL_BINARIES="$WORK/binaries"
find "$CONTENTS" -type f \( -name '*.dylib' -o -name '*.so' -o -perm -u+x \) >"$ALL_BINARIES"

while read -r f; do
    file "$f" | grep -q 'Mach-O' || continue
    chmod u+w "$f"

    # A dylib built outside conda-forge -- the gtk4 plugin is built here by
    # cargo-c -- carries the absolute path it was built at as its install name.
    # Nothing links against these by path, but leaving it in would make the
    # audit below unable to tell a stale absolute reference from a harmless one.
    case "$f" in
    *.dylib | *.so)
        # A loadable module is a Mach-O bundle rather than a dylib and has no
        # install name at all, so `otool -D` prints nothing for it.
        current_id="$(otool -D "$f" 2>/dev/null | tail -n +2 || true)"
        case "$current_id" in
        /*) install_name_tool -id "@rpath/$(basename "$f")" "$f" ;;
        esac
        ;;
    esac

    rpath="@loader_path/$(rel_to_frameworks "$f")"
    has_rpath "$f" "$rpath" || install_name_tool -add_rpath "$rpath" "$f" 2>/dev/null || true

    # An rpath pointing back into the prefix this was built from would let the
    # app keep working on this machine while failing on every other one.
    otool -l "$f" | awk '/LC_RPATH/{f=1} f&&/path /{print $2; f=0}' |
        grep -qxF "$PREFIX/lib" && install_name_tool -delete_rpath "$PREFIX/lib" "$f" 2>/dev/null || true
done <"$ALL_BINARIES"

# ---------------------------------------------------------------------------
# Info.plist, PkgInfo and the icon
# ---------------------------------------------------------------------------

# The deployment floor the bundle can honestly claim: the highest `minos` of
# anything it carries. conda-forge's arm64 packages are built against the macOS
# 11 SDK, so this should come out at 11.0 -- if it comes out as this machine's
# OS version, something Homebrew-built has crept in. See doc/macos.md.
minos_of() {
    otool -l "$1" | awk '/LC_BUILD_VERSION/{f=1} f && $1=="minos" {print $2; exit}'
}

MIN_OS="$(
    {
        while read -r f; do
            file "$f" | grep -q 'Mach-O' && minos_of "$f"
        done <"$ALL_BINARIES"
    } | sort -V | tail -1
)"
[ -n "$MIN_OS" ] || MIN_OS='11.0'
note "deployment target $MIN_OS"

# The icon. macOS gets artwork of its own rather than the GNOME icon the Linux
# builds ship, because the shape rules are not the same: the GNOME icon is drawn
# full-bleed with its own silhouette, while a Mac icon is a square plate that the
# system encloses.
#
# The beveled plate is used on every version of macOS. Tahoe re-shapes and lights
# an application icon itself, so a flat plate meant for it exists in `assets` as
# well -- but what Tahoe made of it did not look good enough to be worth carrying
# two of these and a switch to pick between them.
case "$PROFILE" in
Stable) ICON_SRC="$ROOT/assets/macos-legacy-bevel.svg" ;;
# A development build keeps the GNOME devel icon, which is the only thing
# telling it apart from a stable one at a glance.
*) ICON_SRC="$ROOT/assets/appicon-devel.svg" ;;
esac
note "icon $(basename "$ICON_SRC")"
"$HERE/make-icns.sh" "$ICON_SRC" "$RES/$EXECUTABLE.icns" >/dev/null

# CFBundleVersion and CFBundleShortVersionString only accept period-separated
# numbers, so a pre-release version like "1.rc1" cannot go into them as-is:
# LaunchServices shrugs today, but notarization and the App Store validate.
# The plist gets the longest numeric prefix -- "1.rc1" becomes "1", a final
# "1.0" passes through whole -- and the full string stays everywhere else:
# the artefact names, and the About dialog, which is where a human looks.
PLIST_VERSION=$(printf '%s' "$VERSION" | sed -E 's/^([0-9]+(\.[0-9]+)*).*$/\1/')
printf '%s' "$PLIST_VERSION" | grep -qE '^[0-9]+(\.[0-9]+)*$' || PLIST_VERSION=0

sed -e "s|@APP_ID@|$APP_ID|g" \
    -e "s|@APP_NAME@|$APP_NAME|g" \
    -e "s|@EXECUTABLE@|$EXECUTABLE|g" \
    -e "s|@ICON@|$EXECUTABLE.icns|g" \
    -e "s|@VERSION@|$VERSION|g" \
    -e "s|@PLIST_VERSION@|$PLIST_VERSION|g" \
    -e "s|@MIN_OS@|$MIN_OS|g" \
    "$HERE/Info.plist.in" >"$CONTENTS/Info.plist"

printf 'APPL????' >"$CONTENTS/PkgInfo"

plutil -lint "$CONTENTS/Info.plist" >/dev/null

# ---------------------------------------------------------------------------
# Signing
# ---------------------------------------------------------------------------

# arm64 refuses to run unsigned code at all, so even a bundle nobody is going to
# distribute has to be signed with something. An ad-hoc signature ("-") is a new
# identity on every build, and the Keychain binds its access control to the
# signing identity -- so every rebuild makes macOS ask again whether the app may
# read the session it stored last time. A stable self-signed certificate is what
# stops the prompting; doc/macos.md explains how to make one.
# A symlink pointing outside the bundle is a reference to this machine that
# `otool` cannot see, and it has to be caught before signing rather than after:
# it makes `codesign --verify --strict` fail with a bare ENOENT that names only
# the bundle and gives no hint which file is at fault.
DANGLING="$(find "$APP" -type l ! -exec test -e {} \; -print 2>/dev/null | wc -l | tr -d ' ')"
if [ "$DANGLING" != '0' ]; then
    echo "bundle: $DANGLING dangling symlinks in the bundle:" >&2
    find "$APP" -type l ! -exec test -e {} \; -print 2>/dev/null | sed 's/^/  /' >&2
    exit 1
fi

IDENTITY="${CODESIGN_IDENTITY:--}"
note "signing with identity '$IDENTITY'"

# Nested code first, the bundle last. `--deep` would do this in one call but is
# deprecated, and it signs in an order codesign itself warns about.
while read -r f; do
    file "$f" | grep -q 'Mach-O' || continue
    codesign --force --timestamp=none --sign "$IDENTITY" "$f" 2>/dev/null
done <"$ALL_BINARIES"
codesign --force --timestamp=none --sign "$IDENTITY" "$APP" 2>/dev/null
codesign --verify --deep --strict "$APP"

# ---------------------------------------------------------------------------
# The audit
# ---------------------------------------------------------------------------

# This is the safety net for the whole exercise. If anything in the bundle still
# names a path on this machine, it will load here and fail on the first machine
# it is copied to -- which is exactly the failure that is hardest to notice.
note 'auditing for references outside the bundle'

leaked=0
while read -r f; do
    file "$f" | grep -q 'Mach-O' || continue
    otool -L "$f" | tail -n +2 | awk '{print $1}' >"$WORK/audit"
    while read -r dep; do
        case "$dep" in
        /usr/lib/* | /System/* | @executable_path/* | @rpath/* | @loader_path/*) ;;
        *)
            echo "bundle: $f still references $dep" >&2
            leaked=1
            ;;
        esac
    done <"$WORK/audit"
done <"$ALL_BINARIES"

[ "$leaked" -eq 0 ] || {
    echo 'bundle: the audit found references outside the bundle.' >&2
    exit 1
}

if grep -rIl "$PREFIX" "$RES" 2>/dev/null | head -1 | grep -q .; then
    echo "bundle: warning: some file under Resources still mentions $PREFIX" >&2
    grep -rIl "$PREFIX" "$RES" 2>/dev/null | sed 's/^/  /' >&2
fi

rm -rf "$WORK"

note "built $APP ($(du -sh "$APP" | awk '{print $1}'))"

# The distributable shapes, built from the bundle that was just verified. Both
# are optional because the bundle on its own is what a developer runs.
if [ "$WANT_DMG" -eq 1 ]; then
    "$HERE/make-dmg.sh" "$APP"
fi
if [ "$WANT_TARBALL" -eq 1 ]; then
    "$HERE/make-tarball.sh" "$APP"
fi

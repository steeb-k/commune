#!/bin/sh
# Inventory the macOS GTK development environment for the Commune port.
#
# Read-only: this script never installs, builds or modifies anything. It prints
# one row per dependency as `name | found | version | required | status` and
# ends with the list of things that have to be built or installed before the
# port can be compiled and bundled.
#
# The script is prefix-agnostic. It does not assume Homebrew, MacPorts, jhbuild
# or Nix: everything is discovered through `pkg-config`, so point
# `PKG_CONFIG_PATH` at the GTK installation before running it.
#
#     sh build-aux/macos/probe-env.sh
#
# Paste the output into `doc/macos.md` when the environment changes.

set -u

missing=''
optional_missing=''

# Record a name in a space-separated list, avoiding duplicates.
#
# The list is split on whitespace when it is printed, so names must not contain
# spaces.
remember() {
    _list=$1
    _name=$2
    for _entry in $(eval "printf '%s' \"\$$_list\""); do
        if [ "$_entry" = "$_name" ]; then
            return 0
        fi
    done
    eval "$_list=\"\${$_list} \$_name\""
}

row() {
    printf '%-28s | %-5s | %-22s | %-14s | %s\n' "$1" "$2" "$3" "$4" "$5"
}

header() {
    printf '\n%s\n' "$1"
    printf '%s\n' '--------------------------------------------------------------------------------'
}

table_header() {
    row 'name' 'found' 'version' 'required' 'status'
}

have() {
    command -v "$1" >/dev/null 2>&1
}

# pkg_check <module> <minimum version|-> <required|optional>
pkg_check() {
    _mod=$1
    _min=$2
    _kind=$3

    if ! have pkg-config; then
        row "$_mod" 'no' '-' "$_min" 'no pkg-config'
        remember missing "$_mod"
        return
    fi

    if ! pkg-config --exists "$_mod" 2>/dev/null; then
        row "$_mod" 'no' '-' "$_min" 'MISSING'
        if [ "$_kind" = 'required' ]; then
            remember missing "$_mod"
        else
            remember optional_missing "$_mod"
        fi
        return
    fi

    _ver=$(pkg-config --modversion "$_mod" 2>/dev/null)
    if [ "$_min" = '-' ]; then
        row "$_mod" 'yes' "$_ver" 'any' 'ok'
        return
    fi

    if pkg-config --atleast-version="$_min" "$_mod" 2>/dev/null; then
        row "$_mod" 'yes' "$_ver" ">= $_min" 'ok'
    else
        row "$_mod" 'yes' "$_ver" ">= $_min" 'TOO OLD'
        if [ "$_kind" = 'required' ]; then
            remember missing "$_mod"
        else
            remember optional_missing "$_mod"
        fi
    fi
}

# tool_check <command> <required|optional> [version argument]
tool_check() {
    _cmd=$1
    _kind=$2
    _verarg=${3:---version}

    if ! have "$_cmd"; then
        row "$_cmd" 'no' '-' "$_kind" 'MISSING'
        if [ "$_kind" = 'required' ]; then
            remember missing "$_cmd"
        else
            remember optional_missing "$_cmd"
        fi
        return
    fi

    _ver=$("$_cmd" "$_verarg" 2>/dev/null | head -n 1)
    [ -n "$_ver" ] || _ver='(no version output)'
    row "$_cmd" 'yes' "$_ver" "$_kind" 'ok'
}

# gst_check <element> <required|optional>
gst_check() {
    _el=$1
    _kind=$2

    if ! have gst-inspect-1.0; then
        row "$_el" 'no' '-' "$_kind" 'no gst-inspect-1.0'
        remember missing 'gstreamer-tools'
        return
    fi

    if gst-inspect-1.0 --exists "$_el" 2>/dev/null; then
        _plugin=$(gst-inspect-1.0 "$_el" 2>/dev/null |
            sed -n 's/^ *Filename *\(.*\)$/\1/p' | head -n 1)
        [ -n "$_plugin" ] || _plugin='(found)'
        row "$_el" 'yes' "$(basename "$_plugin")" "$_kind" 'ok'
    else
        row "$_el" 'no' '-' "$_kind" 'MISSING'
        if [ "$_kind" = 'required' ]; then
            remember missing "gst:$_el"
        else
            remember optional_missing "gst:$_el"
        fi
    fi
}

# path_check <label> <path> <required|optional>
path_check() {
    _label=$1
    _path=$2
    _kind=$3

    if [ -e "$_path" ]; then
        row "$_label" 'yes' "$_path" "$_kind" 'ok'
    else
        row "$_label" 'no' "$_path" "$_kind" 'MISSING'
        if [ "$_kind" = 'required' ]; then
            remember missing "$_label"
        else
            remember optional_missing "$_label"
        fi
    fi
}

################################################################################
header 'System'
################################################################################

if have sw_vers; then
    printf 'macOS            : %s (%s)\n' \
        "$(sw_vers -productVersion)" "$(sw_vers -buildVersion)"
fi
printf 'arch             : %s\n' "$(uname -m)"

if have xcode-select; then
    printf 'Xcode CLT        : %s\n' "$(xcode-select -p 2>/dev/null || echo 'MISSING')"
else
    printf 'Xcode CLT        : MISSING\n'
    remember missing 'xcode-command-line-tools'
fi

################################################################################
header 'Environment'
################################################################################

if have pkg-config && pkg-config --exists gtk4 2>/dev/null; then
    GTK_PREFIX=$(pkg-config --variable=prefix gtk4)
else
    GTK_PREFIX=''
fi

printf 'GTK_PREFIX       : %s\n' "${GTK_PREFIX:-<gtk4 not found via pkg-config>}"
printf 'PKG_CONFIG_PATH  : %s\n' "${PKG_CONFIG_PATH:-<unset>}"
printf 'GI_TYPELIB_PATH  : %s\n' "${GI_TYPELIB_PATH:-<unset>}"
printf 'XDG_DATA_DIRS    : %s\n' "${XDG_DATA_DIRS:-<unset>}"
printf 'DYLD_LIBRARY_PATH: %s\n' "${DYLD_LIBRARY_PATH:-<unset>}"

if [ -z "$GTK_PREFIX" ]; then
    printf '\n'
    printf 'gtk4 was not found by pkg-config. Set PKG_CONFIG_PATH to the GTK\n'
    printf 'installation, for example:\n'
    printf '\n'
    printf '    export PKG_CONFIG_PATH=/path/to/prefix/lib/pkgconfig\n'
    printf '\n'
    printf 'and run this script again. Everything below will be reported as\n'
    printf 'missing until then.\n'
fi

################################################################################
header 'Build dependencies (meson.build)'
################################################################################
table_header

# Same list and versions as meson.build, plus what the code links at runtime.
pkg_check 'glib-2.0'              '2.82'   required
pkg_check 'gio-2.0'               '2.82'   required
pkg_check 'gtk4'                  '4.20.2' required
pkg_check 'libadwaita-1'          '1.8.0'  required
pkg_check 'gstreamer-1.0'         '1.20'   required
pkg_check 'gstreamer-app-1.0'     '1.20'   required
pkg_check 'gstreamer-base-1.0'    '1.20'   required
pkg_check 'gstreamer-pbutils-1.0' '1.20'   required
pkg_check 'gstreamer-play-1.0'    '1.20'   required
pkg_check 'gstreamer-video-1.0'   '1.20'   required
pkg_check 'gtksourceview-5'       '5.0.0'  required
pkg_check 'libwebp'               '1.0.0'  required
pkg_check 'shumate-1.0'           '1.1.0'  required
pkg_check 'sqlite3'               '3.24.0' required

printf '\n'
printf 'Linux-only, not needed on macOS (the image decoder is replaced):\n'
pkg_check 'glycin-2'              '2.0.0'  optional
pkg_check 'glycin-gtk4-2'         '2.0.0'  optional

printf '\n'
printf 'Pulled in transitively, needed for bundling:\n'
pkg_check 'libsoup-3.0'           '3.0'    required
pkg_check 'gdk-pixbuf-2.0'        '2.42'   required
pkg_check 'librsvg-2.0'           '2.46'   required

################################################################################
header 'gettext'
################################################################################
table_header

pkg_check 'intl' '-' optional

if [ -n "$GTK_PREFIX" ] && [ -f "$GTK_PREFIX/include/libintl.h" ]; then
    row 'libintl.h' 'yes' "$GTK_PREFIX/include/libintl.h" 'optional' 'ok'
    _have_libintl=yes
elif [ -f /usr/include/libintl.h ]; then
    row 'libintl.h' 'yes' '/usr/include/libintl.h' 'optional' 'ok'
    _have_libintl=yes
else
    row 'libintl.h' 'no' '-' 'optional' 'vendored build'
    _have_libintl=no
fi

tool_check 'msgfmt' required

printf '\n'
if [ "$_have_libintl" = 'yes' ] && [ -n "$GTK_PREFIX" ]; then
    printf 'gettext-sys can link the prefix libintl. For bare cargo runs, export:\n'
    printf '\n'
    printf '    export GETTEXT_SYSTEM=1\n'
    printf '    export GETTEXT_DIR=%s\n' "$GTK_PREFIX"
else
    printf 'No prefix libintl found: gettext-sys will build its vendored static\n'
    printf 'copy, which needs the Xcode command line tools (cc, make).\n'
fi

################################################################################
header 'GStreamer elements'
################################################################################
table_header

gst_check 'gtk4paintablesink' required
gst_check 'playbin3'          required
gst_check 'uridecodebin3'     required
gst_check 'videoconvert'      required
gst_check 'audioconvert'      required
gst_check 'level'             required
gst_check 'osxaudiosink'      required
gst_check 'webpdec'           optional
gst_check 'vtdec_hw'          optional
gst_check 'avdec_h264'        optional
gst_check 'avfvideosrc'       optional
gst_check 'avfdeviceprovider' optional
gst_check 'zbar'              optional

printf '\n'
if have pkg-config && pkg-config --exists gstreamer-1.0 2>/dev/null; then
    printf 'pluginsdir       : %s\n' \
        "$(pkg-config --variable=pluginsdir gstreamer-1.0)"
    printf 'pluginscannerdir : %s\n' \
        "$(pkg-config --variable=pluginscannerdir gstreamer-1.0)"
fi

################################################################################
header 'libshumate vector renderer'
################################################################################
table_header

_shumate_renderer='no'
if have pkg-config && pkg-config --exists shumate-1.0 2>/dev/null; then
    _shumate_prefix=$(pkg-config --variable=prefix shumate-1.0)
    _shumate_libdir=$(pkg-config --variable=libdir shumate-1.0)
    _gir="$_shumate_prefix/share/gir-1.0/Shumate-1.0.gir"

    if [ -f "$_gir" ] && grep -q 'ShumateVectorRenderer' "$_gir" 2>/dev/null; then
        _shumate_renderer='yes'
        row 'ShumateVectorRenderer' 'yes' "$(basename "$_gir")" 'required' 'ok'
    elif [ -f "$_shumate_libdir/libshumate-1.0.dylib" ] && have nm &&
        nm -gU "$_shumate_libdir/libshumate-1.0.dylib" 2>/dev/null |
        grep -q 'shumate_vector_renderer_new'; then
        _shumate_renderer='yes'
        row 'ShumateVectorRenderer' 'yes' 'libshumate-1.0.dylib' 'required' 'ok'
    else
        row 'ShumateVectorRenderer' 'no' '-' 'required' 'MISSING'
        remember missing 'libshumate(-Dvector_renderer=true)'
    fi
else
    row 'ShumateVectorRenderer' 'no' '-' 'required' 'no shumate-1.0'
fi

################################################################################
header 'gdk-pixbuf loaders'
################################################################################
table_header

if have pkg-config && pkg-config --exists gdk-pixbuf-2.0 2>/dev/null; then
    printf 'gdk_pixbuf_moduledir : %s\n' \
        "$(pkg-config --variable=gdk_pixbuf_moduledir gdk-pixbuf-2.0)"
    printf 'gdk_pixbuf_cache_file: %s\n' \
        "$(pkg-config --variable=gdk_pixbuf_cache_file gdk-pixbuf-2.0)"
    printf '\n'
fi

# PNG and JPEG are compiled into gdk-pixbuf itself these days, so they are not
# loadable modules and must not be looked for as such. SVG comes from librsvg
# and is the one that actually has to be present, and bundled: the whole icon
# set is symbolic SVG.
# Read the cache rather than re-running the query tool: the cache is what
# gdk-pixbuf actually loads, and a rescan misses the librsvg loader because it
# is installed as a `.dylib` while every other loader is a `.so`.
if have pkg-config && pkg-config --exists gdk-pixbuf-2.0 2>/dev/null; then
    _cache=$(pkg-config --variable=gdk_pixbuf_cache_file gdk-pixbuf-2.0)

    if [ ! -f "$_cache" ]; then
        row 'loaders.cache' 'no' "$_cache" 'required' 'MISSING'
        remember missing 'gdk-pixbuf-loaders.cache'
    elif grep -q '"svg"' "$_cache" 2>/dev/null; then
        row 'loader: svg (librsvg)' 'yes' 'in loaders.cache' 'required' 'ok'
    else
        row 'loader: svg (librsvg)' 'no' 'not in loaders.cache' 'required' 'MISSING'
        remember missing 'librsvg-gdk-pixbuf-loader'
    fi

    row 'loader: png, jpeg' 'n/a' 'built into gdk-pixbuf' 'required' 'ok'
fi

tool_check 'gdk-pixbuf-query-loaders' required

################################################################################
header 'gio modules (TLS)'
################################################################################
table_header

if have pkg-config && pkg-config --exists gio-2.0 2>/dev/null; then
    _giomoduledir=$(pkg-config --variable=giomoduledir gio-2.0)
    printf 'giomoduledir     : %s\n\n' "$_giomoduledir"

    if ls "$_giomoduledir" 2>/dev/null | grep -qi 'gioopenssl\|giognutls\|gnetworking'; then
        row 'glib-networking' 'yes' \
            "$(ls "$_giomoduledir" 2>/dev/null | grep -i 'gioopenssl\|giognutls' | head -n 1)" \
            'required' 'ok'
    else
        row 'glib-networking' 'no' '-' 'required' 'MISSING'
        remember missing 'glib-networking'
    fi
else
    row 'glib-networking' 'no' '-' 'required' 'no gio-2.0'
fi

################################################################################
header 'Shared data the bundle has to carry'
################################################################################
table_header

if [ -n "$GTK_PREFIX" ]; then
    path_check 'gschemas.compiled' \
        "$GTK_PREFIX/share/glib-2.0/schemas/gschemas.compiled" required
    path_check 'icons/Adwaita' "$GTK_PREFIX/share/icons/Adwaita" required
    path_check 'icons/hicolor' "$GTK_PREFIX/share/icons/hicolor" required
    path_check 'gtksourceview-5' "$GTK_PREFIX/share/gtksourceview-5" required
    path_check 'mime.cache' "$GTK_PREFIX/share/mime/mime.cache" optional
    path_check 'etc/fonts' "$GTK_PREFIX/etc/fonts" optional
else
    printf 'Skipped: GTK_PREFIX is unknown.\n'
fi

################################################################################
header 'Deployment target'
################################################################################

# The single most important property of the prefix: what macOS version the
# dylibs we bundle will demand. conda-forge builds against the macOS 11 SDK;
# Homebrew stamps the build host's OS, which would make the app uninstallable
# for anyone on an older system. See doc/macos.md.
if [ -n "$GTK_PREFIX" ] && [ -f "$GTK_PREFIX/lib/libgtk-4.dylib" ] && have otool; then
    _minos=$(otool -l "$GTK_PREFIX/lib/libgtk-4.dylib" 2>/dev/null |
        sed -n 's/^ *minos *//p' | head -n 1)
    _arch=$(lipo -archs "$GTK_PREFIX/lib/libgtk-4.dylib" 2>/dev/null)

    printf 'libgtk-4.dylib   : arch %s, minos %s\n' \
        "${_arch:-unknown}" "${_minos:-unknown}"

    case "$_minos" in
    1[0-2].*)
        printf 'This is a shippable floor.\n'
        ;;
    '')
        printf 'Could not read the deployment target.\n'
        ;;
    *)
        printf 'WARNING: a floor of %s means the bundle will not run on older\n' "$_minos"
        printf 'macOS versions. This is the signature of a Homebrew-built GTK.\n'
        remember missing "gtk4-with-a-macos-$_minos-floor"
        ;;
    esac
else
    printf 'Skipped: no libgtk-4.dylib found in the prefix.\n'
fi

################################################################################
header 'Build tools'
################################################################################
table_header

tool_check 'meson'                  required
tool_check 'ninja'                  required
tool_check 'cargo'                  required
tool_check 'rustc'                  required
tool_check 'rustup'                 required
tool_check 'glib-compile-resources' required
tool_check 'glib-compile-schemas'   required
tool_check 'blueprint-compiler'     required
tool_check 'grass'                  optional
tool_check 'sass'                   optional

printf '\n'
if have rustup; then
    if rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
        row 'nightly toolchain' 'yes' \
            "$(rustup run nightly rustfmt --version 2>/dev/null | head -n 1)" \
            'required' 'ok'
    else
        row 'nightly toolchain' 'no' '-' 'required' 'MISSING'
        remember missing 'rust-nightly-toolchain'
    fi
fi

################################################################################
header 'Lint and test tools (pre-commit hook)'
################################################################################
table_header

tool_check 'cargo-nextest' required
tool_check 'cargo-deny'    required
tool_check 'cargo-machete' required
tool_check 'cargo-sort'    required
tool_check 'typos'         required
tool_check 'rumdl'         required

################################################################################
header 'Bundling tools'
################################################################################
table_header

tool_check 'dylibbundler' required -h
tool_check 'rsvg-convert' required
tool_check 'iconutil'     required --help
tool_check 'hdiutil'      required help
tool_check 'codesign'     required --help
tool_check 'otool'        required --version
tool_check 'install_name_tool' required --help
tool_check 'create-dmg'   optional

################################################################################
header 'Summary'
################################################################################

if [ -z "${missing# }" ]; then
    printf 'Nothing required is missing.\n'
else
    printf 'needs building from source:\n'
    for _item in $missing; do
        printf '  - %s\n' "$_item"
    done
fi

if [ -n "${optional_missing# }" ]; then
    printf '\noptional, degrades a feature:\n'
    for _item in $optional_missing; do
        printf '  - %s\n' "$_item"
    done
fi

printf '\nRecipes for the missing pieces are in doc/macos.md.\n'

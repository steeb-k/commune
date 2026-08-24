#!/bin/sh
# Inventory the Windows GTK development environment for the Commune port.
#
# Read-only: this script never installs, builds or modifies anything. It prints
# one row per dependency as `name | found | version | required | status` and
# ends with the list of things that have to be installed before the port can be
# compiled and packaged.
#
# Run it from an MSYS2 **UCRT64** shell, which is where the whole port is built:
#
#     sh build-aux/windows/probe-env.sh
#
# Everything is discovered through `pkg-config` and `$MINGW_PREFIX`, so nothing
# here hard-codes `C:\msys64`. The gvsbuild prefix at `C:\gtk` is deliberately
# not supported: it has no GStreamer, libshumate or libsoup3, and its MSVC ABI
# cannot link against them anyway. See `doc/windows-plan.md`.
#
# Paste the output into `doc/windows.md` when the environment changes.

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

if have cmd.exe; then
    printf 'Windows          : %s\n' \
        "$(cmd.exe /c ver 2>/dev/null | tr -d '\r' | sed -n '2p')"
fi
printf 'arch             : %s\n' "$(uname -m)"
printf 'MSYSTEM          : %s\n' "${MSYSTEM:-<unset>}"

if [ "${MSYSTEM:-}" != 'UCRT64' ]; then
    printf '\n'
    printf 'WARNING: this is not a UCRT64 shell. The port is built in UCRT64 and\n'
    printf 'nothing below will describe the environment it is built in. Start\n'
    printf '"MSYS2 UCRT64" from the Start Menu and run this again.\n'
fi

# Cargo target dirs under `_build` go deep enough to pass 260 characters, and a
# build that trips over it fails in ways that look like anything but a path
# length. Both switches have to be on: the registry one for Windows, the git one
# so a checkout can even be created.
# `MSYS2_ARG_CONV_EXCL` stops MSYS2 from rewriting the registry key as if it
# were a path, which turns it into something reg.exe rejects as invalid syntax.
_long_paths=$(MSYS2_ARG_CONV_EXCL='*' reg.exe query \
    'HKLM\SYSTEM\CurrentControlSet\Control\FileSystem' /v LongPathsEnabled 2>/dev/null |
    tr -d '\r' | sed -n 's/.*REG_DWORD *//p')
case "$_long_paths" in
0x1) printf 'LongPathsEnabled : 0x1\n' ;;
'')
    printf 'LongPathsEnabled : <unreadable>\n'
    remember optional_missing 'LongPathsEnabled'
    ;;
*)
    printf 'LongPathsEnabled : %s (should be 0x1)\n' "$_long_paths"
    remember missing 'LongPathsEnabled'
    ;;
esac

if have git; then
    _git_long=$(git config --get core.longpaths 2>/dev/null)
    printf 'git core.longpaths: %s\n' "${_git_long:-<unset>}"
    if [ "$_git_long" != 'true' ]; then
        remember optional_missing 'git-core.longpaths'
    fi
fi

################################################################################
header 'Environment'
################################################################################

if have pkg-config && pkg-config --exists gtk4 2>/dev/null; then
    GTK_PREFIX=$(pkg-config --variable=prefix gtk4)
else
    GTK_PREFIX=''
fi

printf 'MINGW_PREFIX     : %s\n' "${MINGW_PREFIX:-<unset>}"
printf 'GTK_PREFIX       : %s\n' "${GTK_PREFIX:-<gtk4 not found via pkg-config>}"
printf 'PKG_CONFIG_PATH  : %s\n' "${PKG_CONFIG_PATH:-<unset (pkgconf defaults are enough)>}"
printf 'XDG_DATA_DIRS    : %s\n' "${XDG_DATA_DIRS:-<unset>}"

# Where `cargo install` puts things. MSYS2's cargo is a native Windows binary,
# so it uses `%USERPROFILE%` and not the MSYS2 home directory, which is a
# different place — and that directory is not on the MSYS2 PATH by default, so
# every tool installed there looks missing until it is added.
if have cygpath; then
    _cargo_bin="$(cygpath -u "${USERPROFILE:-}")/.cargo/bin"
    printf 'cargo install dir: %s\n' "$_cargo_bin"

    case ":$PATH:" in
    *":$_cargo_bin:"*) ;;
    *)
        printf '\n'
        printf 'WARNING: that directory is not on PATH, so cargo-installed tools\n'
        printf 'and rustup will be reported as missing below. Add it:\n'
        printf '\n'
        printf '    export PATH="$PATH:%s"\n' "$_cargo_bin"
        printf '\n'
        printf 'after /ucrt64/bin, never before it: it holds rustup shims for the\n'
        printf 'MSVC cargo, which must not win over the UCRT64 one.\n'
        printf '\n'
        ;;
    esac
fi

# The port builds against MSYS2's mingw-ABI GTK, so it needs MSYS2's Rust. A
# rustup toolchain first on PATH is the MSVC one, which links against a
# different C runtime and cannot use these libraries at all — and the error it
# gives is a wall of unresolved symbols that says nothing about the ABI.
if have cargo; then
    _cargo_path=$(command -v cargo)
    _host=$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')
    printf 'cargo            : %s\n' "$_cargo_path"
    printf 'rustc host       : %s\n' "${_host:-<unknown>}"

    case "$_host" in
    *windows-gnu | *windows-gnullvm)
        : # The mingw ABI, which is what MSYS2's GTK needs.
        ;;
    *)
        printf '\n'
        printf 'WARNING: this cargo targets %s, which cannot link\n' "${_host:-?}"
        printf 'MSYS2 GTK. Put the UCRT64 toolchain first on PATH:\n'
        printf '\n'
        printf '    pacman -S mingw-w64-ucrt-x86_64-rust\n'
        printf '\n'
        remember missing 'ucrt64-rust'
        ;;
    esac
fi

if [ -z "$GTK_PREFIX" ]; then
    printf '\n'
    printf 'gtk4 was not found by pkg-config. Everything below will be reported\n'
    printf 'as missing until it is installed.\n'
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
pkg_check 'gstreamer-sdp-1.0'     '1.20'   required
pkg_check 'gstreamer-video-1.0'   '1.20'   required
pkg_check 'gstreamer-webrtc-1.0'  '1.20'   required
pkg_check 'gtksourceview-5'       '5.0.0'  required
pkg_check 'libwebp'               '1.0.0'  required
pkg_check 'shumate-1.0'           '1.1.0'  required
pkg_check 'sqlite3'               '3.24.0' required

printf '\n'
printf 'Linux-only, not needed on Windows (the image decoder is replaced):\n'
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
else
    row 'libintl.h' 'no' '-' 'optional' 'vendored build'
    _have_libintl=no
fi

tool_check 'msgfmt' required

printf '\n'
if [ "$_have_libintl" = 'yes' ]; then
    printf 'gettext-sys can link the prefix libintl, which meson.build exports for\n'
    printf 'it. For bare cargo runs, export:\n'
    printf '\n'
    printf '    export GETTEXT_SYSTEM=1\n'
    printf '    export GETTEXT_DIR=%s\n' "$GTK_PREFIX"
else
    printf 'No prefix libintl found: gettext-sys will build its vendored static\n'
    printf 'copy, which needs a working C toolchain.\n'
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
gst_check 'wasapi2sink'       required
gst_check 'wasapisink'        optional
gst_check 'webpdec'           optional
gst_check 'd3d12h264dec'      optional
gst_check 'avdec_h264'        optional

printf '\n'
printf 'Calls (doc/calls.md):\n'
gst_check 'webrtcbin'         required
gst_check 'nicesrc'           required
gst_check 'dtlssrtpenc'       required
gst_check 'srtpenc'           required
gst_check 'opusenc'           required

printf '\n'
printf 'Camera QR scanning, the last milestone:\n'
gst_check 'mfvideosrc'        optional
gst_check 'mfdeviceprovider'  optional
gst_check 'ksvideosrc'        optional

printf '\n'
if have pkg-config && pkg-config --exists gstreamer-1.0 2>/dev/null; then
    printf 'pluginsdir       : %s\n' \
        "$(pkg-config --variable=pluginsdir gstreamer-1.0)"
    printf 'pluginscannerdir : %s\n' \
        "$(pkg-config --variable=pluginscannerdir gstreamer-1.0)"
fi

################################################################################
header 'GTK media backend'
################################################################################
table_header

# GTK plays media through a loadable module, and only builds a GStreamer one
# where it was configured with GStreamer. The macOS port found none in its
# prefix and had to carry its own `GstMediaStream` implementation
# (`src/components/media/gst_media_stream.rs`); whether Windows needs the same
# seam widened is decided here.
if [ -n "$GTK_PREFIX" ]; then
    _media_dir=$(find "$GTK_PREFIX/lib/gtk-4.0" -type d -name media 2>/dev/null | head -n 1)
    if [ -n "$_media_dir" ]; then
        _media_mods=$(ls "$_media_dir" 2>/dev/null | tr '\n' ' ')
    else
        _media_mods=''
    fi

    case "$_media_mods" in
    *gstreamer*)
        row 'gtk media: gstreamer' 'yes' "$_media_mods" 'optional' 'ok'
        ;;
    '')
        row 'gtk media: gstreamer' 'no' '-' 'optional' 'MISSING'
        remember optional_missing 'gtk-gstreamer-media-backend'
        ;;
    *)
        row 'gtk media: gstreamer' 'no' "$_media_mods" 'optional' 'MISSING'
        remember optional_missing 'gtk-gstreamer-media-backend'
        ;;
    esac

    printf '\n'
    printf 'Missing is not fatal: it means the macOS `gst_media_stream` seam has\n'
    printf 'to widen to Windows. See M1 in doc/windows-plan.md.\n'
fi

################################################################################
header 'libshumate vector renderer'
################################################################################
table_header

if have pkg-config && pkg-config --exists shumate-1.0 2>/dev/null; then
    _shumate_prefix=$(pkg-config --variable=prefix shumate-1.0)
    _shumate_bindir="$_shumate_prefix/bin"
    _gir="$_shumate_prefix/share/gir-1.0/Shumate-1.0.gir"
    _dll=$(ls "$_shumate_bindir"/libshumate-1.0-*.dll 2>/dev/null | head -n 1)

    if [ -f "$_gir" ] && grep -q 'ShumateVectorRenderer' "$_gir" 2>/dev/null; then
        row 'ShumateVectorRenderer' 'yes' "$(basename "$_gir")" 'required' 'ok'
    elif [ -n "$_dll" ] && have strings &&
        strings "$_dll" 2>/dev/null | grep -q 'shumate_vector_renderer_new'; then
        row 'ShumateVectorRenderer' 'yes' "$(basename "$_dll")" 'required' 'ok'
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

# PNG and JPEG are compiled into gdk-pixbuf itself these days, so they are not
# loadable modules and must not be looked for as such. SVG comes from librsvg
# and is the one that actually has to be present, and bundled: the whole icon
# set is symbolic SVG.
if have pkg-config && pkg-config --exists gdk-pixbuf-2.0 2>/dev/null; then
    _moduledir=$(pkg-config --variable=gdk_pixbuf_moduledir gdk-pixbuf-2.0)
    _cache=$(pkg-config --variable=gdk_pixbuf_cache_file gdk-pixbuf-2.0)
    printf 'gdk_pixbuf_moduledir : %s\n' "$_moduledir"
    printf 'gdk_pixbuf_cache_file: %s\n' "$_cache"
    printf '\n'

    # Read the cache rather than re-running the query tool: the cache is what
    # gdk-pixbuf actually loads.
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

    _tls=$(ls "$_giomoduledir" 2>/dev/null | grep -i 'gioopenssl\|giognutls' | head -n 1)
    if [ -n "$_tls" ]; then
        row 'glib-networking' 'yes' "$_tls" 'required' 'ok'
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
else
    printf 'Skipped: GTK_PREFIX is unknown.\n'
fi

################################################################################
header 'User directories'
################################################################################

# What GLib derives the data and cache directories from. `g_get_user_data_dir()`
# is `%LOCALAPPDATA%` on Windows, and the cache directory is the one worth
# checking rather than assuming — see `DataType::base_dir_path` in
# `src/utils/mod.rs`, which overrides both.
printf 'LOCALAPPDATA     : %s\n' "${LOCALAPPDATA:-<unset>}"
printf 'APPDATA          : %s\n' "${APPDATA:-<unset>}"
printf 'USERPROFILE      : %s\n' "${USERPROFILE:-<unset>}"
printf 'XDG_DATA_HOME    : %s\n' "${XDG_DATA_HOME:-<unset>}"
printf 'XDG_CACHE_HOME   : %s\n' "${XDG_CACHE_HOME:-<unset>}"

################################################################################
header 'Build tools'
################################################################################
table_header

tool_check 'meson'                  required
tool_check 'ninja'                  required
# Meson stamps a development build with the commit it was built from, so a
# configure fails outright without git — and Windows' own git is not on the
# MSYS2 PATH.
tool_check 'git'                    required
tool_check 'cargo'                  required
tool_check 'rustc'                  required
tool_check 'gcc'                    required
tool_check 'cmake'                  required
tool_check 'glib-compile-resources' required
tool_check 'glib-compile-schemas'   required
tool_check 'blueprint-compiler'     required
tool_check 'grass'                  optional
tool_check 'sassc'                  optional

printf '\n'
# rustfmt is the one tool that must come from rustup rather than from MSYS2: the
# hook runs the nightly one, and MSYS2 packages only stable. It formats source
# and links nothing, so its ABI does not matter.
if have rustup; then
    if rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
        row 'nightly rustfmt' 'yes' \
            "$(rustup run nightly rustfmt --version 2>/dev/null | head -n 1)" \
            'required' 'ok'
    else
        row 'nightly rustfmt' 'no' '-' 'required' 'MISSING'
        remember missing 'rust-nightly-toolchain'
    fi
else
    row 'nightly rustfmt' 'no' 'no rustup on PATH' 'required' 'MISSING'
    remember missing 'rustup'
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
header 'Packaging tools'
################################################################################
table_header

tool_check 'ntldd'        required -h
tool_check 'rsvg-convert' required
tool_check 'icotool'      required --version
tool_check 'strip'        optional

printf '\n'
printf 'These two live on the Windows side rather than in MSYS2, and are only\n'
printf 'needed to build and sign the .msi:\n'

# `wix` is a dotnet tool and `signtool` comes with the Windows SDK; neither is
# on the MSYS2 PATH by default even when installed.
if have dotnet && dotnet tool list --global 2>/dev/null | grep -qi '^wix '; then
    row 'wix' 'yes' \
        "$(dotnet tool list --global 2>/dev/null | awk '/^wix /{print $2}')" \
        'optional' 'ok'
else
    row 'wix' 'no' '-' 'optional' 'MISSING'
    remember optional_missing 'wix'
fi

_signtool=$(ls -1 '/c/Program Files (x86)/Windows Kits/10/bin'/*/x64/signtool.exe \
    2>/dev/null | tail -n 1)
if [ -n "$_signtool" ]; then
    row 'signtool' 'yes' "$_signtool" 'optional' 'ok'
else
    row 'signtool' 'no' '-' 'optional' 'MISSING'
    remember optional_missing 'signtool'
fi

################################################################################
header 'Summary'
################################################################################

if [ -z "${missing# }" ]; then
    printf 'Nothing required is missing.\n'
else
    printf 'needs installing:\n'
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

printf '\nThe package names for these are in doc/windows.md.\n'

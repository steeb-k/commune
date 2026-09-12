#!/usr/bin/env bash
# Create the conda-forge GTK environment that the macOS build and bundle use.
#
#   build-aux/macos/setup-conda-macos.sh              # osx-arm64 env
#   build-aux/macos/setup-conda-macos.sh --universal  # + osx-64 env, for lipo
#   build-aux/macos/setup-conda-macos.sh --x86-only   # osx-64 env alone
#   build-aux/macos/setup-conda-macos.sh --skip-extras
#
# `--universal` works on Apple Silicon, where Rosetta runs the osx-64
# packages' post-link scripts. It does NOT work on an Intel Mac, because
# nothing translates arm64 back to x86 — the osx-arm64 post-link scripts are
# arm64 binaries that machine cannot execute, and the install dies on
# gdk-pixbuf's loader cache. An Intel machine building its half of a universal
# bundle therefore wants `--x86-only`, not `--universal`.
#
# Why conda-forge and not Homebrew: conda-forge builds its osx-arm64 packages
# against the macOS 11 SDK and its osx-64 packages against ~10.13, so every
# dylib we bundle carries a macOS 11 `minos` no matter which macOS this machine
# runs. Homebrew stamps the build host's OS instead, which on a current dev box
# forces a deployment floor nobody can install. See doc/macos.md.
#
# Requires conda, mamba or micromamba on PATH. miniforge is the lightweight,
# fully conda-forge option: https://github.com/conda-forge/miniforge
#
# Envs are created at .conda-gtk/{arm64,x86}; override with COMMUNE_CONDA_ARM
# and COMMUNE_CONDA_X86.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ARM_ENV="${COMMUNE_CONDA_ARM:-$ROOT/.conda-gtk/arm64}"
X86_ENV="${COMMUNE_CONDA_X86:-$ROOT/.conda-gtk/x86}"

# The three projects behind `webrtcbin`, which calls do not run without and
# which conda-forge does not package. See build_webrtc() at the bottom.
#
# libnice must be at least 0.1.23: that is what gst-plugins-bad's
# gst-libs/gst/webrtc/nice/meson.build asks for, and an older one silently
# leaves libgstwebrtcnice unbuilt, which silently leaves the webrtc plugin out.
LIBNICE_VERSION="${COMMUNE_LIBNICE_VERSION:-0.1.23}"
LIBSRTP_VERSION="${COMMUNE_LIBSRTP_VERSION:-v2.8.0}"

# The dependencies of meson.build, plus what the GTK stack dlopens at runtime.
#
# zlib/freetype/expat  their .pc files: conda-forge ships these separately from
#                      the runtime libs (libzlib/libfreetype/…) that gtk4 pulls,
#                      but gio/harfbuzz/fontconfig list them in Requires.private,
#                      so the Rust *-sys builds need the .pc.
# libintl-devel        the unversioned libintl.dylib symlink the linker needs for
#                      the `-lintl` that glib's .pc emits, and the libintl.h that
#                      lets gettext-rs link it instead of vendoring its own.
# librsvg              the SVG gdk-pixbuf loader, for symbolic icons.
# glib-networking      the TLS gio module. Without it every https:// request
#                      fails, which means no login at all.
# adwaita-icon-theme   the symbolic icons the UI is built from.
# gst-plugins-bad      applemedia (avfvideosrc, vtdec_hw) and osxaudio.
# libxml2-devel        the libxml2 headers and .pc. conda-forge's libxml2 runtime
#                      package ships neither, and gtksourceview needs the headers
#                      while appstream names libxml-2.0 in Requires.private.
PKGS="
gtk4 libadwaita librsvg pkg-config zlib freetype expat libintl-devel libxml2-devel
gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad
gtksourceview libshumate libsoup glib-networking
libwebp sqlite adwaita-icon-theme
gobject-introspection pygobject meson ninja
"

# Which environments to create: arm, x86, or both.
WHICH=arm
SKIP_EXTRAS=0
for arg in "$@"; do
    case "$arg" in
    --universal) WHICH=both ;;
    --x86-only) WHICH=x86 ;;
    --skip-extras) SKIP_EXTRAS=1 ;;
    *)
        echo "setup-conda-macos: unknown argument: $arg" >&2
        exit 2
        ;;
    esac
done

# Prefer a micromamba vendored into the repo, so the environment does not depend
# on whatever happens to be on PATH. Fall back to anything installed.
CONDA="${COMMUNE_CONDA_BIN:-}"
if [ -z "$CONDA" ] && [ -x "$ROOT/.conda-gtk/.bin/micromamba" ]; then
    CONDA="$ROOT/.conda-gtk/.bin/micromamba"
fi
if [ -z "$CONDA" ]; then
    for c in mamba micromamba conda; do
        command -v "$c" >/dev/null 2>&1 && {
            CONDA="$c"
            break
        }
    done
fi
[ -n "$CONDA" ] || {
    echo "setup-conda-macos: need conda/mamba/micromamba." >&2
    echo "  vendor one at .conda-gtk/.bin/micromamba, set COMMUNE_CONDA_BIN," >&2
    echo "  or install miniforge: https://github.com/conda-forge/miniforge" >&2
    exit 1
}

# micromamba needs a root prefix for its package cache; keep it in the repo so
# nothing lands in the home directory.
export MAMBA_ROOT_PREFIX="${MAMBA_ROOT_PREFIX:-$ROOT/.conda-gtk/.root}"

# Fallback for a prefix without libxml2-devel: conda-forge's libxml2 runtime
# package ships no libxml-2.0.pc, appstream (pulled by libadwaita) lists
# libxml-2.0 in Requires.private, and pkg-config errors on a missing private dep
# even for a dynamic build — so the libadwaita-1 probe fails. Installing
# libxml2-devel is the real fix and is in PKGS above; this synthesizes a minimal
# parseable .pc if it is somehow still absent. Note that a synthesized .pc is not
# enough to *build* gtksourceview, which needs the actual headers.
synth_libxml_pc() { # <env-path>
    local env="$1" pc="$1/lib/pkgconfig/libxml-2.0.pc" ver
    [ -f "$pc" ] && return 0
    ls "$env"/lib/libxml2*.dylib >/dev/null 2>&1 || return 0
    ver="$("$CONDA" list -p "$env" 2>/dev/null | awk '$1=="libxml2"{print $2; exit}')"
    cat >"$pc" <<EOF
prefix=$env
libdir=\${prefix}/lib
includedir=\${prefix}/include

Name: libXML
Version: ${ver:-2.14.0}
Description: libXML library version2 (synthesized by setup-conda-macos.sh).
Libs: -L\${libdir} -lxml2
Cflags: -I\${includedir}/libxml2
EOF
    echo "setup-conda-macos: synthesized libxml-2.0.pc (libxml2 ${ver:-?} ships none)"
}

create_env() { # <subdir> <env-path>
    local subdir="$1" env="$2"
    echo "setup-conda-macos: creating $subdir env at $env ($CONDA)"
    rm -rf "$env"
    # shellcheck disable=SC2086  # PKGS is a deliberately word-split list.
    CONDA_SUBDIR="$subdir" "$CONDA" create -y -p "$env" -c conda-forge $PKGS
    # Pin the platform so any later install into this env keeps the same arch.
    printf 'subdir: %s\n' "$subdir" >"$env/.condarc"
    synth_libxml_pc "$env"
    finalize_env "$env"
    echo "setup-conda-macos: $subdir env ready"
}

# Two caches that the packages do not build for us, and that nothing works
# without.
finalize_env() { # <env-path>
    local env="$1"

    # GTK aborts on startup without compiled schemas, and the file chooser
    # crashes with "No GSettings schemas are installed" even if it gets that
    # far. conda-forge ships the XML but never runs the compiler.
    if [ -d "$env/share/glib-2.0/schemas" ]; then
        "$env/bin/glib-compile-schemas" "$env/share/glib-2.0/schemas"
        echo "setup-conda-macos: compiled GSettings schemas"
    fi

    # The librsvg loader is installed as `.dylib` while every other loader is
    # `.so`, and the cache conda-forge ships lists only the `.so` ones. Without
    # the SVG loader the entire symbolic icon set fails to load, so pass both
    # extensions explicitly rather than letting the tool scan.
    local loaders="$env/lib/gdk-pixbuf-2.0/2.10.0/loaders"
    if [ -d "$loaders" ]; then
        "$env/bin/gdk-pixbuf-query-loaders" "$loaders"/*.so "$loaders"/*.dylib \
            >"$env/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
        echo "setup-conda-macos: rebuilt gdk-pixbuf loaders.cache with the SVG loader"
    fi
}

# Three things Commune needs are not packaged by conda-forge at all, so they
# are built into the env from source. All three are required: meson will not
# configure without blueprint-compiler, video playback has no sink without
# gtk4paintablesink, and a call cannot be placed or answered without
# webrtcbin.
build_extras() { # <env-path>
    local env="$1"
    export PKG_CONFIG_PATH="$env/lib/pkgconfig"
    export PKG_CONFIG_LIBDIR="$env/lib/pkgconfig"
    export PATH="$env/bin:$PATH"
    export GI_TYPELIB_PATH="$env/lib/girepository-1.0"
    export MACOSX_DEPLOYMENT_TARGET=11.0
    # cargo-c gets built while this env is visible, so it links the env's
    # libssl and then cannot find it again from a plain shell. Keep the env's
    # libraries reachable rather than fighting over which OpenSSL it picks.
    export DYLD_FALLBACK_LIBRARY_PATH="$env/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"

    build_typelibs "$env"

    if [ ! -x "$env/bin/blueprint-compiler" ]; then
        echo "setup-conda-macos: building blueprint-compiler into $env"
        local src
        src="$(mktemp -d)"
        git clone --depth 1 --branch v0.18.0 \
            https://gitlab.gnome.org/GNOME/blueprint-compiler.git "$src"
        meson setup "$src/_build" "$src" --prefix="$env"
        meson install -C "$src/_build"
        rm -rf "$src"
    fi

    if ! "$env/bin/gst-inspect-1.0" --exists gtk4paintablesink 2>/dev/null; then
        echo "setup-conda-macos: building gst-plugin-gtk4 into $env"
        command -v cargo >/dev/null 2>&1 || {
            echo "setup-conda-macos: cargo is needed to build gst-plugin-gtk4" >&2
            return 1
        }
        command -v cargo-cbuild >/dev/null 2>&1 || cargo install cargo-c

        # gst-plugins-rs branches are named after the gstreamer-**rs** binding
        # version, not the GStreamer C version: 0.15 is the branch that uses
        # the gstreamer 0.25 crates that Cargo.toml pins. Bump both together.
        local branch src
        branch="${GST_PLUGINS_RS_BRANCH:-0.15}"
        src="$(mktemp -d)"
        git clone --depth 1 --branch "$branch" \
            https://gitlab.freedesktop.org/gstreamer/gst-plugins-rs.git "$src"
        (
            cd "$src"
            cargo cbuild -p gst-plugin-gtk4 --release \
                --prefix="$env" --libdir="$env/lib"
            cargo cinstall -p gst-plugin-gtk4 --release \
                --prefix="$env" --libdir="$env/lib"
        )
        rm -rf "$src"
    fi

    build_webrtc "$env"
}

# conda-forge builds libshumate and gtksourceview without introspection, so
# neither ships a typelib. blueprint-compiler resolves `using Shumate 1.0` and
# `using GtkSource 5` through typelibs and fails without them, which means no
# `.ui` file compiles and nothing builds at all.
#
# Only the introspection data is taken. The libraries themselves are left as
# conda-forge built them, because those carry the macOS 11 deployment floor that
# is the whole reason for using conda-forge; anything compiled here would be
# stamped with this machine's SDK instead.
build_typelibs() { # <env-path>
    local env="$1"

    if [ -f "$env/lib/girepository-1.0/Shumate-1.0.typelib" ] &&
        [ -f "$env/lib/girepository-1.0/GtkSource-5.typelib" ]; then
        return 0
    fi

    local stage src version
    stage="$(mktemp -d)/stage"

    if [ ! -f "$env/lib/girepository-1.0/Shumate-1.0.typelib" ]; then
        echo "setup-conda-macos: generating the Shumate typelib"
        version="$("$env/bin/pkg-config" --modversion shumate-1.0)"
        src="$(mktemp -d)"
        git clone --depth 1 --branch "$version" \
            https://gitlab.gnome.org/GNOME/libshumate.git "$src"
        # -Dsysprof=disabled: the sysprof subproject does not build on macOS.
        # There is no vector_renderer option to set — as of 1.6 the vector
        # renderer is always enabled.
        meson setup "$src/_b" "$src" --prefix="$stage" --libdir=lib \
            -Dgir=true -Dvapi=false -Dgtk_doc=false -Ddemos=false -Dsysprof=disabled
        meson install -C "$src/_b"
        rm -rf "$src"
    fi

    if [ ! -f "$env/lib/girepository-1.0/GtkSource-5.typelib" ]; then
        echo "setup-conda-macos: generating the GtkSource typelib"
        version="$("$env/bin/pkg-config" --modversion gtksourceview-5)"
        src="$(mktemp -d)"
        git clone --depth 1 --branch "$version" \
            https://gitlab.gnome.org/GNOME/gtksourceview.git "$src"
        meson setup "$src/_b" "$src" --prefix="$stage" --libdir=lib \
            -Dintrospection=enabled -Dvapi=false -Ddocumentation=false -Dsysprof=false
        meson install -C "$src/_b"
        rm -rf "$src"
    fi

    mkdir -p "$env/lib/girepository-1.0" "$env/share/gir-1.0"
    cp -f "$stage/lib/girepository-1.0/"*.typelib "$env/lib/girepository-1.0/"
    cp -f "$stage/share/gir-1.0/"*.gir "$env/share/gir-1.0/" 2>/dev/null || true
    rm -rf "$(dirname "$stage")"
}

# `webrtcbin` is what every call is built on, and conda-forge does not ship it.
# Its gst-plugins-bad carries the libgstwebrtc-1.0 *library* and the
# gstreamer-webrtc-1.0.pc that Cargo.toml links against — so the Rust side
# builds and links perfectly well — but not the `webrtc`, `nice` or `srtp`
# *plugins*, and there is no libnice or libsrtp package in the channel at all
# for it to have built them from. The result is a client that compiles cleanly
# and then ends every call with a missing-element error.
#
# So three things are built here, in the order they depend on each other:
#
#   libsrtp2   the ciphers under `srtpenc`, which `dtlssrtpenc` makes by name
#              at runtime. conda-forge's dtls plugin already exists and works
#              once srtp is beside it, so that one is left alone.
#   libnice    ICE, and the `nice` plugin (nicesrc/nicesink) that comes with it.
#   the two    gst-plugins-bad rebuilt at the version conda-forge installed,
#   plugins    with everything except webrtc/srtp/dtls/sctp switched off.
#
# The last one installs into a staging prefix and only three files are taken
# out of it. gst-plugins-bad also builds a dozen libraries conda-forge already
# ships — libgstwebrtc-1.0, libgstsctp-1.0, libgstcodecparsers-1.0 — and
# overwriting those with copies compiled here would replace the macOS 11
# deployment floor with this machine's SDK, which is the whole reason for using
# conda-forge. libgstwebrtcnice-1.0 is the exception: it exists nowhere in the
# env, because it is the half of gst-plugins-bad that needs libnice.
#
# MACOSX_DEPLOYMENT_TARGET is exported by build_extras(), so what is compiled
# here carries `minos 11.0` the same way the packages do. Check with
# `vtool -show-build`.
build_webrtc() { # <env-path>
    local env="$1"

    if "$env/bin/gst-inspect-1.0" --exists webrtcbin 2>/dev/null; then
        return 0
    fi

    echo "setup-conda-macos: building webrtcbin and its dependencies into $env"

    local stage src version
    stage="$(mktemp -d)/stage"

    if ! "$env/bin/pkg-config" --exists libsrtp2; then
        echo "setup-conda-macos: building libsrtp2 $LIBSRTP_VERSION"
        src="$(mktemp -d)"
        git clone --depth 1 --branch "$LIBSRTP_VERSION" \
            https://github.com/cisco/libsrtp.git "$src"
        # openssl rather than the built-in ciphers: it is already in the env,
        # and it is what the AES-GCM profiles WebRTC negotiates need.
        meson setup "$src/_b" "$src" --prefix="$env" --libdir=lib --buildtype=release \
            -Dcrypto-library=openssl -Dtests=disabled -Ddoc=disabled -Dpcap-tests=disabled
        meson install -C "$src/_b"
        rm -rf "$src"
    fi

    if ! "$env/bin/pkg-config" --exists nice; then
        echo "setup-conda-macos: building libnice $LIBNICE_VERSION"
        src="$(mktemp -d)"
        git clone --depth 1 --branch "$LIBNICE_VERSION" \
            https://gitlab.freedesktop.org/libnice/libnice.git "$src"
        # gupnp asks the router to map a port, which a call does not need and
        # which would add a dependency the env does not have.
        meson setup "$src/_b" "$src" --prefix="$env" --libdir=lib --buildtype=release \
            -Dcrypto-library=openssl -Dgstreamer=enabled -Dgupnp=disabled \
            -Dintrospection=disabled -Dexamples=disabled -Dtests=disabled -Dgtk_doc=disabled
        meson install -C "$src/_b"
        rm -rf "$src"
    fi

    # The version conda-forge installed, so the plugins match the libraries
    # they will be loaded beside. Since 1.20 the modules live in one repository,
    # so take only the subproject rather than cloning all of GStreamer's blobs.
    version="$("$env/bin/pkg-config" --modversion gstreamer-1.0)"
    echo "setup-conda-macos: building the webrtc and srtp plugins from gst-plugins-bad $version"
    src="$(mktemp -d)"
    git clone --depth 1 --branch "$version" --filter=blob:none --sparse \
        https://gitlab.freedesktop.org/gstreamer/gstreamer.git "$src"
    git -C "$src" sparse-checkout set subprojects/gst-plugins-bad

    # dtls and sctp are enabled because the webrtc option requires them to be —
    # webrtcbin will not configure otherwise — not because their plugins are
    # wanted. Those two are not copied out.
    meson setup "$src/subprojects/gst-plugins-bad/_b" "$src/subprojects/gst-plugins-bad" \
        --prefix="$stage" --libdir=lib --buildtype=release \
        --auto-features=disabled -Dwebrtc=enabled -Dsrtp=enabled -Ddtls=enabled -Dsctp=enabled \
        -Dintrospection=disabled -Dexamples=disabled -Dtests=disabled \
        -Dnls=disabled -Ddoc=disabled -Dorc=disabled
    meson install -C "$src/subprojects/gst-plugins-bad/_b"

    cp "$stage/lib/gstreamer-1.0/libgstwebrtc.dylib" \
        "$stage/lib/gstreamer-1.0/libgstsrtp.dylib" "$env/lib/gstreamer-1.0/"
    cp -a "$stage"/lib/libgstwebrtcnice-1.0*.dylib "$env/lib/"

    rm -rf "$src" "$(dirname "$stage")"

    # The registry caches which plugins exist, and a stale one hides the ones
    # that just appeared.
    rm -rf "${HOME:?}/.cache/gstreamer-1.0"

    "$env/bin/gst-inspect-1.0" --exists webrtcbin || {
        echo "setup-conda-macos: built the webrtc plugin but webrtcbin is still missing" >&2
        return 1
    }
    echo "setup-conda-macos: webrtcbin is available"
}

# The environments this run is responsible for, as pairs.
ENVS=()
case "$WHICH" in
arm) ENVS=(osx-arm64 "$ARM_ENV") ;;
x86) ENVS=(osx-64 "$X86_ENV") ;;
both) ENVS=(osx-arm64 "$ARM_ENV" osx-64 "$X86_ENV") ;;
esac

for ((i = 0; i < ${#ENVS[@]}; i += 2)); do
    create_env "${ENVS[i]}" "${ENVS[i + 1]}"
done

if [ "$SKIP_EXTRAS" = 0 ]; then
    for ((i = 0; i < ${#ENVS[@]}; i += 2)); do
        build_extras "${ENVS[i + 1]}"
    done
else
    echo "setup-conda-macos: skipping blueprint-compiler, gst-plugin-gtk4 and webrtcbin"
fi

# The closing advice should name an environment this run actually created.
PRIMARY="${ENVS[1]}"

cat <<EOF

setup-conda-macos: done.

Point the build at the env, then check it:

    export PKG_CONFIG_PATH=$PRIMARY/lib/pkgconfig
    export PKG_CONFIG_LIBDIR=$PRIMARY/lib/pkgconfig
    export PATH=$PRIMARY/bin:\$PATH
    export GI_TYPELIB_PATH=$PRIMARY/lib/girepository-1.0
    export XDG_DATA_DIRS=$PRIMARY/share
    export MACOSX_DEPLOYMENT_TARGET=11.0

    bash build-aux/macos/probe-env.sh

PKG_CONFIG_LIBDIR matters as much as PKG_CONFIG_PATH: it replaces the default
search path, so nothing leaks in from the two Homebrew prefixes on this machine
(/usr/local is x86_64 and comes first on PATH).
EOF
